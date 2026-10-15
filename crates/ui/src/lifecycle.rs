//! What happens when something the user must know about happens: a session ends or crashes
//! (pane overlay + `behavior_on_session_end`), OSC 52 clipboard writes (policy + ask toast) and
//! the toast plumbing. Nothing here is silent.

use std::fmt;
use std::time::Instant;

use fogu_core::{Osc52Policy, effective_osc52_write};
use fogu_session::{ClipboardRequest, SessionError, SessionWarning};
use fogu_term::{ClipboardType, sanitize_title};
use iced::{Task, clipboard};

use crate::app::App;
use crate::message::Message;
use crate::notifications::{AUTO_DISMISS, Level, Notification, ToastAction};
use crate::tab::{PaneId, PaneState};

/// OSC 52 writes waiting for the user's decision at once; older ones are dropped.
const MAX_PENDING_CLIPBOARD: usize = 4;

/// An OSC 52 write behind an "ask" toast. `Debug` never shows the payload.
pub(crate) struct PendingClipboard {
    pub toast: u64,
    pub request: ClipboardRequest,
}

impl fmt::Debug for PendingClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingClipboard")
            .field("toast", &self.toast)
            .field("len", &self.request.len)
            .finish_non_exhaustive()
    }
}

/// One line of "what went wrong": the error plus its source, control characters removed.
pub(crate) fn describe(err: &SessionError) -> String {
    let mut text = err.to_string();
    if let Some(source) = std::error::Error::source(err) {
        text.push_str(": ");
        text.push_str(&source.to_string());
    }
    sanitize_title(&text)
}

fn write_clipboard(request: ClipboardRequest) -> Task<Message> {
    match request.target {
        ClipboardType::Clipboard => clipboard::write(request.payload),
        ClipboardType::Selection => clipboard::write_primary(request.payload),
    }
}

impl App {
    /// Shows `note`; the returned task removes it after [`AUTO_DISMISS`] when it is the
    /// self-dismissing kind.
    pub(crate) fn notify(&mut self, note: Notification) -> Task<Message> {
        let auto = note.auto_dismisses();
        self.toasts.push(note);
        self.sync_pending_clipboard();
        if auto {
            self.after(AUTO_DISMISS, Message::ToastExpire)
        } else {
            Task::none()
        }
    }

    /// Forgets OSC 52 writes whose toast is gone (dismissed, expired or evicted).
    fn sync_pending_clipboard(&mut self) {
        let toasts = &self.toasts;
        self.pending_clipboard
            .retain(|p| toasts.get(p.toast).is_some());
    }

    pub(crate) fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.sync_pending_clipboard();
    }

    pub(crate) fn expire_toasts(&mut self) {
        self.toasts.expire(Instant::now());
        self.sync_pending_clipboard();
    }

    /// "Copy" on an OSC 52 toast: applies the write.
    pub(crate) fn accept_toast(&mut self, id: u64) -> Task<Message> {
        let pending = self
            .pending_clipboard
            .iter()
            .position(|p| p.toast == id)
            .map(|i| self.pending_clipboard.remove(i));
        self.dismiss_toast(id);
        pending.map_or_else(Task::none, |p| write_clipboard(p.request))
    }

    /// A session failed: a sticky error toast plus the pane's crashed overlay.
    pub(crate) fn on_session_error(&mut self, pane: PaneId, err: &SessionError) -> Task<Message> {
        let detail = describe(err);
        tracing::error!(error = %detail, "session failed");
        let toast = self.notify(Notification::new(Level::Error, "Session failed").detail(&detail));
        Task::batch([toast, self.on_pane_ended(pane, PaneState::Crashed(detail))])
    }

    /// Marks the pane ended and applies the profile's `behavior_on_session_end`: a pane being
    /// closed, or whose exit the policy says to close, goes away now; otherwise it shows its
    /// status and restarts on any key (`restart.rs`). A crash is never closed by policy.
    pub(crate) fn on_pane_ended(&mut self, pane_id: PaneId, state: PaneState) -> Task<Message> {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| t.panes.contains_key(&pane_id))
        else {
            return Task::none();
        };
        let Some(pane) = tab.panes.get_mut(&pane_id) else {
            return Task::none();
        };
        let by_policy = matches!(state, PaneState::Exited(_))
            && pane
                .profile
                .behavior_on_session_end
                .closes(state.is_clean_exit());
        // A question the session can no longer take an answer to.
        pane.prompt = None;
        pane.status = state.status_line(pane.is_ssh());
        pane.state = state;
        pane.refresh();
        if pane.closing || by_policy {
            return self.remove_pane(pane_id);
        }
        Task::none()
    }

    /// A non-fatal session condition: a toast now (the tab badge comes from `Pane::warnings`).
    pub(crate) fn on_session_warning(&mut self, warning: &SessionWarning) -> Task<Message> {
        let note =
            Notification::new(Level::Warn, warning_title(warning)).detail(warning_detail(warning));
        self.notify(note)
    }

    /// OSC 52 write from pane `pane_id`, per `terminal.osc52_write` (profile override first):
    /// `Allow` copies, `Deny` says so, `Ask` shows a sticky toast with Copy / Deny. Only the
    /// length is ever shown or logged.
    pub(crate) fn on_clipboard_store(
        &mut self,
        pane_id: PaneId,
        request: ClipboardRequest,
    ) -> Task<Message> {
        let profile = self.pane(pane_id).map(|p| &p.profile);
        let default = self.config.terminal.osc52_write;
        let policy = profile.map_or(default, |p| effective_osc52_write(p, default));
        match policy {
            Osc52Policy::Allow => write_clipboard(request),
            Osc52Policy::Deny => self.notify(
                Notification::new(Level::Info, "Clipboard write denied").detail(format!(
                    "A program tried to set the clipboard ({} bytes)",
                    request.len
                )),
            ),
            Osc52Policy::Ask => {
                let note = Notification::new(Level::Warn, "Remote wants to set the clipboard")
                    .detail(format!("{} bytes", request.len))
                    .action(ToastAction::ClipboardWrite);
                let toast = self.toasts.push(note);
                self.pending_clipboard
                    .push(PendingClipboard { toast, request });
                if self.pending_clipboard.len() > MAX_PENDING_CLIPBOARD {
                    let oldest = self.pending_clipboard.remove(0).toast;
                    self.toasts.dismiss(oldest);
                }
                self.sync_pending_clipboard();
                Task::none()
            }
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

/// Short name of a session warning (toast title).
pub(crate) fn warning_title(warning: &SessionWarning) -> &'static str {
    match warning {
        SessionWarning::HostKeyNotVerified => "Host key not verified",
        SessionWarning::Unsupported(_) => "SSH options ignored",
    }
}

/// One-line explanation of a session warning (toast detail, tab badge tooltip).
pub(crate) fn warning_detail(warning: &SessionWarning) -> String {
    match warning {
        SessionWarning::HostKeyNotVerified => {
            "verify_host_keys = false: the server's identity was not checked.".to_owned()
        }
        SessionWarning::Unsupported(options) => {
            format!("Not supported yet: {}", options.join(", "))
        }
    }
}
