//! The SFTP panel's touch points with the desktop: files dropped onto the window are uploaded
//! into the panel's directory (X11 only: winit 0.30 has no Wayland drops), downloaded files are
//! opened with `xdg-open` only when the user clicks "Open" / "Download and open" / "Show in
//! file manager" (never automatically), and the file pickers are checked for
//! before use (rfd opens nothing, silently, when neither the portal nor zenity exists).

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use fogu_session::sftp::OverwritePolicy;
use fogu_session::{SftpCmd, display_name};
use iced::Task;

use super::transfer_label;
use super::transfers::{Direction, Origin};
use crate::app::App;
use crate::message::Message;
use crate::notifications::{Level, Notification};

/// Drop events of one gesture arrive one file at a time: they are gathered this long into one
/// upload.
const DROP_BATCH: Duration = Duration::from_millis(150);

/// Desktop messages.
#[derive(Clone, Debug)]
pub enum DesktopMsg {
    /// A file was dropped onto the window (X11).
    Dropped(PathBuf),
    /// The drop batch is complete: upload it.
    FlushDrops,
    /// Open a local file or folder with the desktop's default application.
    Open(PathBuf),
    /// The opener failed (sanitized reason).
    OpenFailed(String),
}

/// Per-window desktop state.
#[derive(Debug, Default)]
pub struct Desktop {
    /// Dropped files waiting for the batch to end.
    dropped: Vec<PathBuf>,
    /// The Wayland "drop needs X11" hint was shown.
    hinted: bool,
}

impl App {
    pub(crate) fn on_desktop(&mut self, message: DesktopMsg) -> Task<Message> {
        match message {
            DesktopMsg::Dropped(path) => {
                let first = self.desktop.dropped.is_empty();
                self.desktop.dropped.push(path);
                if first {
                    return self.after(DROP_BATCH, Message::Desktop(DesktopMsg::FlushDrops));
                }
                Task::none()
            }
            DesktopMsg::FlushDrops => self.upload_dropped(),
            DesktopMsg::Open(path) => self.open_local(path),
            DesktopMsg::OpenFailed(why) => {
                self.notify(Notification::new(Level::Error, "Cannot open").detail(why))
            }
        }
    }

    /// Uploads the dropped files into the active tab's visible panel directory. winit reports
    /// no drop position on X11, so the panel of the active tab is the target whenever it is
    /// open.
    fn upload_dropped(&mut self) -> Task<Message> {
        let local = std::mem::take(&mut self.desktop.dropped);
        let tab = self.active;
        let Some((panel, tx)) = self.panel_of(tab).filter(|(p, _)| p.visible) else {
            let note = Notification::new(Level::Info, "Dropped files")
                .detail("Open the SFTP panel of an SSH tab to upload dropped files.");
            return self.notify(note);
        };
        let Some(remote_dir) = panel.path.clone() else {
            return Task::none();
        };
        let cmd = SftpCmd::Upload {
            local: local.clone(),
            remote_dir: remote_dir.clone(),
            overwrite: OverwritePolicy::Ask,
            resume: false,
        };
        let label = transfer_label(&cmd);
        let origin = Origin::Upload { local, remote_dir };
        match panel.issue(&tx, cmd, Direction::Upload, label, origin) {
            Ok(_) => Task::none(),
            Err(err) => self.notify(Notification::new(Level::Error, "SFTP").detail(err)),
        }
    }

    /// Once per run on Wayland, when a panel opens: drops need X11 for now.
    pub(super) fn wayland_drop_hint(&mut self) -> Task<Message> {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if !wayland || self.desktop.hinted {
            return Task::none();
        }
        self.desktop.hinted = true;
        let note = Notification::new(Level::Info, "Drag and drop")
            .detail("Dropping files onto the panel needs X11 for now \u{2014} use Upload\u{2026}");
        self.notify(note)
    }

    /// Opens `path` with `xdg-open` (detached; the exit status is collected on the session
    /// runtime). Only for local paths the app wrote or chose, only on an explicit click, and
    /// never an executable file (see [`refuse_open`]).
    pub(crate) fn open_local(&self, path: PathBuf) -> Task<Message> {
        if let Err(why) = refuse_open(&path) {
            return Task::done(Message::Desktop(DesktopMsg::OpenFailed(why)));
        }
        let run = self
            .runtime
            .handle()
            .spawn(async move { xdg_open(&path).await });
        Task::future(run).then(|result| {
            let why = match result {
                Ok(Ok(())) => return Task::none(),
                Ok(Err(why)) => why,
                Err(_) => "the opener task failed".to_owned(),
            };
            Task::done(Message::Desktop(DesktopMsg::OpenFailed(why)))
        })
    }
}

/// Why `path` must not be handed to `xdg-open`: not absolute, missing, or a regular file with
/// an executable bit. Downloads are `0644` unless `keep_executable_bit` is on; with it, a
/// script, `.desktop` file or `AppImage` the server marked executable would be *run* by some
/// openers (KDE asks, others don't), so such files are only shown in the file manager.
fn refuse_open(path: &Path) -> Result<(), String> {
    let shown = || display_name(&path.to_string_lossy());
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) if path.is_absolute() => meta,
        _ => return Err(format!("{} does not exist", shown())),
    };
    if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
        return Err(format!(
            "{} is executable and is not opened from here; use \u{201c}Show in file \
             manager\u{201d}",
            shown()
        ));
    }
    Ok(())
}

/// Runs `xdg-open <path>` and waits for it (it returns once the application started).
async fn xdg_open(path: &Path) -> Result<(), String> {
    let status = tokio::process::Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| format!("xdg-open: {e}"))?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "xdg-open could not open {} ({status})",
        display_name(&path.to_string_lossy())
    ))
}

/// Whether a file picker can open: `zenity` on `PATH`, or a session bus with the desktop
/// portal service installed.
pub(crate) fn picker_available() -> bool {
    in_path("zenity") || portal_installed()
}

/// The toast shown instead of a picker that cannot open.
pub(crate) fn no_picker_note() -> Notification {
    Notification::new(Level::Error, "No file picker available").detail(
        "Install xdg-desktop-portal (with a backend such as xdg-desktop-portal-gtk) or zenity. \
         Double-click downloads still work, and on X11 files can be dropped onto the panel.",
    )
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

fn portal_installed() -> bool {
    let runtime_bus =
        std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|dir| Path::new(&dir).join("bus").exists());
    let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() || runtime_bus;
    let service = "dbus-1/services/org.freedesktop.portal.Desktop.service";
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")));
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    bus && data_home
        .into_iter()
        .chain(std::env::split_paths(&data_dirs))
        .any(|dir| dir.join(service).is_file())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use std::os::unix::fs::PermissionsExt as _;

    use super::refuse_open;

    #[test]
    fn executables_and_missing_paths_are_not_opened() {
        let dir = std::env::temp_dir().join(format!("fogu-open-{}", std::process::id()));
        let _stale = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("report.txt");
        std::fs::write(&plain, "x").unwrap();
        let script = dir.join("run.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        assert_eq!(refuse_open(&plain), Ok(()));
        assert_eq!(refuse_open(&dir), Ok(()), "directories open (file manager)");
        assert!(refuse_open(&script).unwrap_err().contains("executable"));
        assert!(
            refuse_open(&dir.join("missing"))
                .unwrap_err()
                .contains("does not exist")
        );
        assert!(refuse_open(std::path::Path::new("report.txt")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
