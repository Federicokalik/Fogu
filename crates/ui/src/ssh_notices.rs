//! SSH session prompts and warnings: every prompt is declined at once (so the session never waits for its timeout) and explained
//! in a toast; warnings become toasts. Prompt texts are never logged.

use fogu_session::{PromptId, PromptKind, PromptReply, SessionInput, SessionWarning};
use iced::Task;

use crate::app::App;
use crate::message::Message;
use crate::notifications::{Level, Notification};
use crate::tab::PaneId;

impl App {
    /// Declines `kind` (no prompt UI yet) and says what would have been asked.
    pub(crate) fn on_prompt(
        &mut self,
        pane_id: PaneId,
        id: PromptId,
        kind: &PromptKind,
    ) -> Task<Message> {
        if let Some(pane) = self.tabs.iter().find_map(|t| t.panes.get(&pane_id)) {
            pane.control(SessionInput::PromptReply(id, PromptReply::Reject));
        }
        let detail = match kind {
            PromptKind::HostKeyUnknown {
                host,
                port,
                key_type,
                fingerprint,
            } => format!(
                "Unknown host key for {host}:{port} ({key_type} {fingerprint}). Host-key prompts \
                 are not available yet; connect once with `ssh` to record it in ~/.ssh/known_hosts."
            ),
            PromptKind::HostKeyChanged {
                host,
                port,
                known_hosts_path,
                line,
                ..
            } => format!(
                "The host key of {host}:{port} CHANGED (recorded at {}:{line}). Connection refused.",
                known_hosts_path.display()
            ),
            PromptKind::Password { .. }
            | PromptKind::KeyPassphrase { .. }
            | PromptKind::KeyboardInteractive { .. } => {
                "Password, passphrase and keyboard-interactive prompts are not available yet; use \
                 an SSH agent or an unencrypted key."
                    .to_owned()
            }
        };
        self.notify(Notification::new(Level::Warn, "SSH prompt declined").detail(detail))
    }

    /// A non-fatal session condition as a toast.
    pub(crate) fn on_session_warning(&mut self, warning: &SessionWarning) -> Task<Message> {
        let note = match warning {
            SessionWarning::HostKeyNotVerified => {
                Notification::new(Level::Warn, "Host key not verified")
                    .detail("verify_host_keys = false: the server's identity was not checked.")
            }
            SessionWarning::Unsupported(options) => {
                Notification::new(Level::Info, "SSH options ignored")
                    .detail(format!("Not supported yet: {}", options.join(", ")))
            }
        };
        self.notify(note)
    }
}
