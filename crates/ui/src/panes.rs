//! Pane-level operations inside a tab's split tree: split, close, navigate, focus,
//! divider ratios and the per-pane resize path.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use fogu_core::{Dir, EffectiveProfile, Profile, ProfileKind, Side, Size, resolve};
use fogu_session::SessionInput;
use iced::Task;

use crate::actions::CLOSE_GRACE;
use crate::app::App;
use crate::message::Message;
use crate::modal::{Modal, ModalButton};
use crate::tab::{Pane, PaneId, TabId};

/// Transport resize debounce during drags.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);
/// How long a split waits for the focused pane's cwd before opening without one.
pub(crate) const SPLIT_CWD_WAIT: Duration = Duration::from_millis(500);

/// What a pending cwd query is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CwdFor {
    /// `NewTabHere`: open a tab there.
    Tab,
    /// `SplitRight`/`SplitDown`: open the new pane there, on this side of the queried pane.
    Split(Side),
    /// `DuplicateTab`: open the pane's profile there in a new tab.
    Duplicate,
    /// `CopyPath`: put it on the clipboard.
    Copy,
}

/// What a close request (keymap, `×`, middle-click) is for; kept while the "close live SSH
/// session?" modal is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseTarget {
    /// A whole tab.
    Tab(TabId),
    /// One pane.
    Pane(PaneId),
    /// Every tab but this one.
    OthersOf(TabId),
    /// The tabs right of this one.
    RightOf(TabId),
}

/// A `QueryCwd` sent to `pane` at `asked`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CwdQuery {
    pub pane: PaneId,
    pub asked: Instant,
    pub purpose: CwdFor,
}

impl App {
    /// Splits the active tab's focused pane; a running local pane is asked for its cwd first.
    pub(crate) fn split_focused(&mut self, side: Side) -> Task<Message> {
        let Some(tab) = self.active_tab() else {
            return Task::none();
        };
        let target = tab.focused;
        let local_running = tab.focused_pane().is_some_and(|p| {
            matches!(p.profile.kind, ProfileKind::Local(_)) && p.state.is_running() && !p.closing
        });
        if local_running && self.ask_cwd(target, CwdFor::Split(side)) {
            return self.after(SPLIT_CWD_WAIT, Message::CwdTimeout(target));
        }
        self.split_now(target, side, None, None);
        Task::none()
    }

    /// Opens a pane with `profile` (default: `target`'s own) on `side` of `target` and
    /// focuses it.
    pub(crate) fn split_now(
        &mut self,
        target: PaneId,
        side: Side,
        cwd: Option<PathBuf>,
        profile: Option<Profile>,
    ) {
        let Some(index) = self.tab_of(target) else {
            return;
        };
        // Only a split of the same profile continues in the target's remote cwd.
        let same_profile = profile
            .as_ref()
            .is_none_or(|p| self.pane(target).is_some_and(|t| t.profile.id == p.id));
        let Some(profile) = profile.or_else(|| self.pane(target).map(|p| p.profile.clone())) else {
            return;
        };
        let cwd = cwd.or_else(|| {
            self.pane(target)
                .filter(|_| same_profile)
                .and_then(|p| p.remote_cwd.clone())
                .map(PathBuf::from)
        });
        let pane = self.open_pane(&profile, cwd);
        let id = pane.id;
        let active = self.active;
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if !tab.layout.split(&target, side, id) {
            return;
        }
        tab.panes.insert(id, pane);
        if tab.id == active {
            self.focus_pane(id);
        } else {
            tab.focused = id;
        }
    }

    /// The split's cwd never came: split without it.
    fn on_cwd_timeout(&mut self, pane: PaneId) {
        let Some(query) = self.pending_cwd.filter(|q| q.pane == pane) else {
            return;
        };
        match query.purpose {
            CwdFor::Split(side) => {
                self.pending_cwd = None;
                self.split_now(pane, side, None, None);
            }
            CwdFor::Duplicate => {
                self.pending_cwd = None;
                if let Some(profile) = self.pane(pane).map(|p| p.profile.clone()) {
                    self.open_tab(&profile, None);
                }
            }
            CwdFor::Tab | CwdFor::Copy => {}
        }
    }

    /// `true` when closing `pane` must be confirmed: a live SSH session whose profile keeps
    /// `warn_on_close` (default on).
    fn warns_on_close(&self, pane: &Pane) -> bool {
        if !pane.is_ssh() || !pane.state.is_running() || pane.closing {
            return false;
        }
        matches!(
            resolve(&pane.profile, &self.config.profile_defaults),
            EffectiveProfile::Ssh(ssh) if ssh.warn_on_close
        )
    }

    /// A close from the keymap, `×` or middle-click: asks first when a live SSH session would
    /// go (`warn_on_close`), otherwise closes now.
    pub(crate) fn request_close(&mut self, target: CloseTarget) -> Task<Message> {
        let warn = match target {
            CloseTarget::Pane(id) => self.pane(id).is_some_and(|p| self.warns_on_close(p)),
            _ => self
                .closing_tabs(target)
                .iter()
                .filter_map(|id| self.tabs.iter().find(|t| t.id == *id))
                .any(|t| t.panes.values().any(|p| self.warns_on_close(p))),
        };
        if warn {
            self.pending_close = Some(target);
            return Task::none();
        }
        self.close_now(target)
    }

    /// The user confirmed (`true`) or cancelled the pending close.
    pub(crate) fn on_confirm_close(&mut self, confirmed: bool) -> Task<Message> {
        match self.pending_close.take() {
            Some(target) if confirmed => self.close_now(target),
            _ => Task::none(),
        }
    }

    fn close_now(&mut self, target: CloseTarget) -> Task<Message> {
        if let CloseTarget::Pane(id) = target {
            return self.close_pane(id);
        }
        let tabs = self.closing_tabs(target);
        Task::batch(
            tabs.into_iter()
                .map(|id| self.close_tab(id))
                .collect::<Vec<_>>(),
        )
    }

    /// The tabs a tab-level `target` closes, left to right.
    fn closing_tabs(&self, target: CloseTarget) -> Vec<TabId> {
        let index = |id| self.tabs.iter().position(|t| t.id == id);
        let ids = self.tabs.iter().enumerate().map(|(i, t)| (i, t.id));
        match target {
            CloseTarget::Tab(id) => vec![id],
            CloseTarget::Pane(_) => Vec::new(),
            CloseTarget::OthersOf(keep) => ids
                .filter(|(_, id)| *id != keep)
                .map(|(_, id)| id)
                .collect(),
            CloseTarget::RightOf(from) => match index(from) {
                Some(at) => ids.filter(|(i, _)| *i > at).map(|(_, id)| id).collect(),
                None => Vec::new(),
            },
        }
    }

    /// What closing `target` stops: whether an SSH connection ends, and how many listening
    /// port forwards stop. A connection other open panes still use keeps its forwards.
    fn close_effects(&self, target: CloseTarget) -> (bool, usize) {
        let closing: Vec<&Pane> = if let CloseTarget::Pane(id) = target {
            self.pane(id).into_iter().collect()
        } else {
            let tabs = self.closing_tabs(target);
            self.tabs
                .iter()
                .filter(|t| tabs.contains(&t.id))
                .flat_map(|t| t.panes.values())
                .collect()
        };
        let closing_ids: Vec<PaneId> = closing.iter().map(|p| p.id).collect();
        let still_used = |conn| {
            self.tabs.iter().flat_map(|t| t.panes.values()).any(|p| {
                !closing_ids.contains(&p.id) && p.state.is_running() && p.connection == Some(conn)
            })
        };
        let ending: Vec<&Pane> = closing
            .into_iter()
            .filter(|p| p.is_ssh() && p.state.is_running())
            .filter(|p| p.connection.is_none_or(|c| !still_used(c)))
            .collect();
        let forwards = crate::forwards_dialog::listening(ending.iter().copied());
        (!ending.is_empty(), forwards)
    }

    /// The "close live SSH session?" modal, if one is pending.
    pub(crate) fn close_modal(&self) -> Option<Modal> {
        let target = self.pending_close?;
        let (ends, forwards) = self.close_effects(target);
        let mut body = if ends {
            "The connection is still open; closing ends it.".to_owned()
        } else {
            "Other tabs keep using the connection.".to_owned()
        };
        match forwards {
            0 => {}
            1 => body.push_str("\n1 port forward will stop."),
            n => {
                let _infallible = write!(body, "\n{n} port forwards will stop.");
            }
        }
        Some(Modal {
            title: "Close SSH session?",
            body,
            buttons: vec![
                ModalButton {
                    label: "Close",
                    on_press: Message::ConfirmClose(true),
                },
                ModalButton {
                    label: "Cancel",
                    on_press: Message::ConfirmClose(false),
                },
            ],
        })
    }

    /// Asks pane `id`'s session to close; the pane goes on `Exited` or after [`CLOSE_GRACE`].
    /// A pane whose session is already gone is removed at once.
    pub(crate) fn close_pane(&mut self, id: PaneId) -> Task<Message> {
        let Some(pane) = self.pane_mut(id) else {
            return Task::none();
        };
        if pane.closing {
            return Task::none();
        }
        if pane.state.is_running() && pane.control(SessionInput::Close) {
            pane.closing = true;
            return self.after(CLOSE_GRACE, Message::CloseTimeout(id));
        }
        self.remove_pane(id)
    }

    pub(crate) fn on_close_timeout(&mut self, id: PaneId) -> Task<Message> {
        if self.pane(id).is_some_and(|p| p.closing) {
            return self.remove_pane(id);
        }
        Task::none()
    }

    /// Drops pane `id` (closing its session); the tab goes with its last pane.
    pub(crate) fn remove_pane(&mut self, id: PaneId) -> Task<Message> {
        let Some(index) = self.tab_of(id) else {
            return Task::none();
        };
        if self.pending_cwd.is_some_and(|q| q.pane == id) {
            self.pending_cwd = None;
        }
        let active = self.active;
        let Some(tab) = self.tabs.get_mut(index) else {
            return Task::none();
        };
        tab.panes.remove(&id);
        tab.layout.remove(&id);
        if tab.sftp.as_ref().is_some_and(|p| p.pane == id) {
            tab.sftp = None;
        }
        let Some(next) = tab.layout.leaves().first().map(|p| **p) else {
            let tab = tab.id;
            return self.remove_tab(tab);
        };
        if tab.focused == id {
            tab.focused = next;
            if tab.id == active {
                self.focus_report(true);
            }
        }
        Task::none()
    }

    /// Moves focus to the geometric neighbour of the focused pane.
    pub(crate) fn pane_nav(&mut self, dir: Dir) {
        let next = self
            .active_tab()
            .and_then(|t| t.layout.neighbor(&t.focused, dir).copied());
        if let Some(next) = next {
            self.focus_pane(next);
        }
    }

    /// Focuses pane `id` of the active tab (click, navigation, new split); also commits an
    /// inline rename and takes keyboard focus back from the tab bar and the profile sidebar.
    pub(crate) fn focus_pane(&mut self, id: PaneId) {
        self.commit_rename();
        self.profiles.focused = false;
        let active = self.active;
        let Some(tab) = self.tabs.iter().find(|t| t.id == active) else {
            return;
        };
        if !tab.panes.contains_key(&id) {
            return;
        }
        if tab.focused != id {
            self.focus_report(false);
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == active) {
                tab.focused = id;
            }
            self.focus_report(true);
        }
        if !self.focused {
            self.on_focused(true);
        }
    }

    /// Divider drag / double-click, pane clicks and split cwd timeouts.
    pub(crate) fn on_split_ui(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SplitRatio(tab, path, ratio) => {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    tab.layout.set_ratio(&path, ratio);
                }
            }
            Message::Equalize(tab) => {
                self.menu = None;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab) {
                    tab.layout.equalize();
                }
            }
            Message::FocusPane(pane) => self.focus_pane(pane),
            Message::CwdTimeout(pane) => self.on_cwd_timeout(pane),
            _ => {}
        }
        Task::none()
    }

    /// Pane `id`'s rectangle now holds `cols`×`rows`: resize its grid now (correct rendering)
    /// and its PTY after a 50 ms quiet period. Window resizes, zoom and divider drags all land
    /// here.
    pub(crate) fn on_grid_resized(&mut self, id: PaneId, cols: u16, rows: u16) -> Task<Message> {
        let (cols, rows) = (cols.max(2), rows.max(1));
        let (cell_w, cell_h) = (self.metrics.cell_w_px, self.metrics.cell_h_px);
        let Some(pane) = self.pane_mut(id) else {
            return Task::none();
        };
        pane.term.resize(cols, rows);
        pane.refresh();
        pane.pending_resize = Some(Size {
            cols,
            rows,
            px_w: cols.saturating_mul(cell_w),
            px_h: rows.saturating_mul(cell_h),
        });
        pane.resize_generation += 1;
        let message = Message::ResizeTimer(id, pane.resize_generation);
        self.after(RESIZE_DEBOUNCE, message)
    }

    pub(crate) fn on_resize_timer(&mut self, id: PaneId, generation: u64) -> Task<Message> {
        let Some(pane) = self.pane_mut(id) else {
            return Task::none();
        };
        if generation != pane.resize_generation {
            return Task::none();
        }
        let Some(size) = pane.pending_resize.take() else {
            return Task::none();
        };
        if pane.control(SessionInput::Resize(size)) || !pane.state.is_running() {
            return Task::none();
        }
        // The session is busy; retry after another debounce period.
        tracing::debug!("PTY resize deferred");
        pane.pending_resize = Some(size);
        pane.resize_generation += 1;
        let message = Message::ResizeTimer(id, pane.resize_generation);
        self.after(RESIZE_DEBOUNCE, message)
    }
}
