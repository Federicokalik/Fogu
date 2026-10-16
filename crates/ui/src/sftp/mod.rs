//! The SFTP side panel of an SSH tab: browses the remote filesystem over the pane's SFTP engine
//! (`fogu_session::sftp`), downloads and uploads through the portal file pickers, and lists
//! transfers with progress and cancel.
//!
//! `SftpPanelState` (this file) is the reducer: it sends commands on the pane's
//! [`SftpLink`](crate::tab::SftpLink) and folds engine events into the listing, the transfers
//! and the conflict queue. App wiring is in `handlers.rs`, drawing in `panel.rs`.
//!
//! Remote names are attacker data: they are shown only through `display_name` and become local
//! path components only inside the engine (validated) or through `transfers::reissue`
//! (re-validated).

pub mod breadcrumb;
mod events;
mod handlers;
mod keys;
pub mod listing;
pub mod panel;
mod transfer_view;
pub use transfer_view::conflict_dialog;
pub mod transfers;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fogu_core::config::SftpCfg;
use fogu_session::sftp::{CmdId, OverwritePolicy, SftpOptions, TransferId};
use fogu_session::{SftpCmd, SftpEvent, SftpRequest, display_name};
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::notifications::{Level, Notification};
use crate::tab::{PaneId, TabId};
use listing::{Listing, SortKey};
use transfers::{Conflict, ConflictChoice, Conflicts, Direction, Origin, Transfer, Transfers};

/// Smallest / largest panel width as a share of the tab.
pub const MIN_WIDTH: f32 = 0.15;
pub const MAX_WIDTH: f32 = 0.85;

/// SFTP messages: engine events of a pane, and panel input of a tab.
#[derive(Clone, Debug)]
pub enum SftpMsg {
    /// An engine event from pane `.0`'s connection.
    Event(PaneId, Box<SftpEvent>),
    /// Input in tab `.0`'s panel.
    Panel(TabId, PanelMsg),
}

/// Panel input.
#[derive(Clone, Debug)]
pub enum PanelMsg {
    /// `×`: hide the panel (transfers keep running).
    Close,
    /// Divider drag (share of the tab) or double-click (`None` = configured default).
    Width(Option<f32>),
    /// The pointer entered/left the panel (decides who gets the keyboard on the next click).
    Hover(bool),
    /// Breadcrumb segment or programmatic navigation.
    Go(String),
    /// Click on the empty breadcrumb bar: type a path.
    EditPath,
    PathInput(String),
    PathSubmit,
    Back,
    Up,
    Home,
    Refresh,
    ShowHidden(bool),
    Filter(String),
    Sort(SortKey),
    /// Click on an entry (modifiers from the app's tracked state).
    Click(String),
    /// Double-click on an entry: enter a directory / download a file.
    Open(String),
    /// The list scrolled: offset and viewport height.
    Scrolled(f32, f32),
    /// `Upload…`: pick local files.
    Upload,
    /// The picker answered: upload into `remote_dir` (`None` = cancelled).
    UploadPicked {
        remote_dir: String,
        local: Option<Vec<PathBuf>>,
    },
    /// `Download…` for the selection: pick a local folder.
    Download,
    /// The picker answered: fetch `remote` (paths in `remote_dir`) into `local_dir`.
    DownloadPicked {
        remote_dir: String,
        remote: Vec<String>,
        local_dir: Option<PathBuf>,
    },
    Cancel(TransferId),
    ClearTransfers,
    ToggleTransfers,
    /// Finished transfers may be due for removal.
    Sweep,
    Conflict(ConflictChoice),
    ConflictApplyAll(bool),
}

/// What a command was sent for.
#[derive(Clone, Debug)]
enum Pending {
    /// `Realpath` of the start directory (`home`: the login directory) or a typed path.
    Resolve { home: bool, path: String },
    /// `List`: `from` is pushed on the back stack when it succeeds.
    List { path: String, from: Option<String> },
}

/// What the app should do after an event.
#[derive(Debug, Default)]
pub struct Reaction {
    /// Toasts to show.
    pub notes: Vec<Notification>,
    /// A transfer finished: schedule a sweep after the linger time.
    pub sweep: bool,
}

/// The panel of one tab.
#[derive(Debug)]
pub struct SftpPanelState {
    /// The SSH pane whose connection the panel uses.
    pub pane: PaneId,
    pub visible: bool,
    /// Keyboard goes to the panel (set by a click inside it).
    pub focused: bool,
    pub hovered: bool,
    /// Share of the tab's width.
    pub width: f32,
    /// The directory shown (raw).
    pub path: Option<String>,
    /// The login directory, once known.
    pub home: Option<String>,
    /// Back stack.
    pub back: Vec<String>,
    pub listing: Listing,
    /// The `List` in flight (spinner); later answers win, older ones are ignored.
    pub loading: Option<CmdId>,
    /// Shown instead of the list when nothing could be listed yet.
    pub error: Option<String>,
    /// Typed path while the breadcrumb is in edit mode.
    pub editing: Option<String>,
    pub transfers: Transfers,
    pub conflicts: Conflicts,
    /// List scroll offset and viewport height (row virtualization).
    pub scroll_y: f32,
    pub view_h: f32,
    pending: HashMap<CmdId, Pending>,
    next_cmd: u64,
}

impl SftpPanelState {
    /// A hidden, empty panel on `pane`'s connection.
    pub fn new(pane: PaneId, cfg: &SftpCfg) -> Self {
        Self {
            pane,
            visible: false,
            focused: false,
            hovered: false,
            width: cfg.panel_width.clamp(MIN_WIDTH, MAX_WIDTH),
            path: None,
            home: None,
            back: Vec::new(),
            listing: Listing::new(cfg.show_hidden),
            loading: None,
            error: None,
            editing: None,
            transfers: Transfers::default(),
            conflicts: Conflicts::default(),
            scroll_y: 0.0,
            view_h: 0.0,
            pending: HashMap::new(),
            next_cmd: 1,
        }
    }

    /// Sends `cmd`; the error text says why it could not be sent.
    pub fn send(&mut self, tx: &Sender<SftpRequest>, cmd: SftpCmd) -> Result<CmdId, String> {
        let id = CmdId(self.next_cmd);
        self.next_cmd += 1;
        match tx.try_send(SftpRequest { id, cmd }) {
            Ok(()) => Ok(id),
            Err(TrySendError::Full(_)) => Err("the SFTP engine is busy; try again".to_owned()),
            Err(TrySendError::Closed(_)) => Err("the connection is closed".to_owned()),
        }
    }

    /// Shows the panel: options first, then the start directory (`start` = profile cwd, else
    /// the login directory), or a refresh when a directory is already shown.
    pub fn open(
        &mut self,
        tx: &Sender<SftpRequest>,
        options: SftpOptions,
        start: Option<String>,
    ) -> Result<(), String> {
        self.visible = true;
        self.focused = true;
        self.send(tx, SftpCmd::SetOptions(options))?;
        match self.path.clone() {
            Some(path) => self.list(tx, path, false),
            None => self.resolve(tx, start),
        }
    }

    /// `Realpath` of `path` (`None` = the login directory), then list it.
    pub fn resolve(
        &mut self,
        tx: &Sender<SftpRequest>,
        path: Option<String>,
    ) -> Result<(), String> {
        let home = path.is_none();
        let path = path.unwrap_or_else(|| ".".into());
        let id = self.send(tx, SftpCmd::Realpath(path.clone()))?;
        self.pending.insert(id, Pending::Resolve { home, path });
        self.loading = Some(id);
        Ok(())
    }

    /// Lists `path`; `push` records the current directory on the back stack on success.
    pub fn list(
        &mut self,
        tx: &Sender<SftpRequest>,
        path: String,
        push: bool,
    ) -> Result<(), String> {
        let id = self.send(tx, SftpCmd::List(path.clone()))?;
        let from = self.path.clone().filter(|p| push && *p != path);
        self.pending.insert(id, Pending::List { path, from });
        self.loading = Some(id);
        Ok(())
    }

    /// Back: the previous directory (not pushed again).
    pub fn go_back(&mut self, tx: &Sender<SftpRequest>) -> Result<(), String> {
        match self.back.pop() {
            Some(path) => self.list(tx, path, false),
            None => Ok(()),
        }
    }

    /// Up: the parent directory.
    pub fn go_up(&mut self, tx: &Sender<SftpRequest>) -> Result<(), String> {
        match self.path.as_deref().and_then(breadcrumb::parent) {
            Some(parent) => self.list(tx, parent, true),
            None => Ok(()),
        }
    }

    /// Home: the login directory.
    pub fn go_home(&mut self, tx: &Sender<SftpRequest>) -> Result<(), String> {
        match self.home.clone() {
            Some(home) => self.list(tx, home, true),
            None => self.resolve(tx, None),
        }
    }

    /// Sends a transfer command and adds its row.
    pub fn issue(
        &mut self,
        tx: &Sender<SftpRequest>,
        cmd: SftpCmd,
        direction: Direction,
        label: String,
        origin: Origin,
    ) -> Result<(), String> {
        let id = self.send(tx, cmd)?;
        self.transfers
            .push(Transfer::new(id, direction, label, origin));
        Ok(())
    }

    /// Answers the current conflict; re-issues overwritten files as new transfers. Returns
    /// notes for names that could not be re-issued.
    pub fn answer_conflict(
        &mut self,
        tx: &Sender<SftpRequest>,
        choice: ConflictChoice,
    ) -> Vec<Notification> {
        let chosen = self.conflicts.answer(choice);
        let mut by_transfer: Vec<(TransferId, Vec<String>)> = Vec::new();
        for Conflict { transfer, path, .. } in chosen {
            match by_transfer.iter_mut().find(|(t, _)| *t == transfer) {
                Some((_, paths)) => paths.push(path),
                None => by_transfer.push((transfer, vec![path])),
            }
        }
        let mut notes = Vec::new();
        for (transfer, paths) in by_transfer {
            let Some(old) = self.transfers.get(transfer) else {
                continue;
            };
            let (origin, direction) = (old.origin.clone(), old.direction);
            let (cmds, rejected) = transfers::reissue(&origin, &paths);
            if !rejected.is_empty() {
                notes.push(
                    Notification::new(Level::Warn, "Not overwritten")
                        .detail(format!("Invalid name(s): {}", rejected.join(", "))),
                );
            }
            for cmd in cmds {
                if let (
                    SftpCmd::Download { local_dir, .. },
                    Origin::Download {
                        local_dir: base, ..
                    },
                ) = (&cmd, &origin)
                    && !transfers::real_subdir(base, local_dir)
                {
                    notes.push(
                        Notification::new(Level::Warn, "Not overwritten")
                            .detail("A folder on the way is missing or is a symbolic link."),
                    );
                    continue;
                }
                let label = format!("{} (overwrite)", transfer_label(&cmd));
                if let Err(err) = self.issue(tx, cmd, direction, label, origin.clone()) {
                    notes.push(Notification::new(Level::Error, "SFTP").detail(err));
                }
            }
        }
        notes
    }

    /// The pane was replaced (reconnect): everything that ran on the old connection is gone.
    pub fn rebind(&mut self, pane: PaneId, now: Instant) {
        self.pane = pane;
        self.transfers.fail_active("connection closed", now);
        self.conflicts.clear();
        self.pending.clear();
        self.loading = None;
    }
}

/// `Download` with the "ask on conflict" policy.
pub fn download_cmd(remote: Vec<String>, local_dir: PathBuf) -> SftpCmd {
    SftpCmd::Download {
        remote,
        local_dir,
        overwrite: OverwritePolicy::Ask,
        resume: false,
    }
}

/// A short label for a transfer command (sanitized names).
pub fn transfer_label(cmd: &SftpCmd) -> String {
    let names: Vec<String> = match cmd {
        SftpCmd::Download { remote, .. } => remote
            .iter()
            .map(|p| display_name(p.rsplit('/').next().unwrap_or(p)))
            .collect(),
        SftpCmd::Upload { local, .. } => local
            .iter()
            .map(|p| {
                display_name(
                    &p.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} + {} more", rest.len()),
    }
}

fn error_note(title: &str, detail: String) -> Notification {
    Notification::new(Level::Error, title).detail(detail)
}

/// The engine options for `[sftp]` (same defaults as `SftpOptions::default()`).
pub fn engine_options(cfg: &SftpCfg) -> SftpOptions {
    SftpOptions {
        max_parallel_files: cfg.max_parallel_files.max(1),
        keep_partial: cfg.keep_partial,
        request_timeout: Duration::from_secs(cfg.request_timeout_secs.max(1)),
        max_listing_entries: cfg.max_listing_entries.max(1),
        max_walk_entries: cfg.max_walk_entries.max(1),
        max_walk_depth: cfg.max_walk_depth.max(1),
        allow_growing_files: cfg.allow_growing_files,
        max_download_bytes: cfg.max_download_bytes,
        free_space_reserve_bytes: cfg.free_space_reserve_bytes,
    }
}

/// Where double-click downloads go: `[sftp] download_dir`, else the XDG download directory,
/// else the home directory.
pub fn download_dir(cfg: &SftpCfg) -> Option<PathBuf> {
    cfg.download_dir
        .clone()
        .or_else(dirs::download_dir)
        .or_else(dirs::home_dir)
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
