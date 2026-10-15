//! Paste: clipboard read completion, the multi-line confirmation modal and the sanitised write to
//! the session.

use fogu_term::{TermMode, input};

use crate::app::App;
use crate::message::Message;
use crate::modal::{Modal, ModalButton};
use crate::tab::PaneId;

/// A multi-line/large paste waiting on the user's "Paste"/"Cancel" choice in the modal.
#[derive(Debug)]
pub(crate) struct PendingPaste {
    pub pane: PaneId,
    pub text: String,
    pub warning: input::PasteWarning,
}

impl App {
    /// A clipboard read for [`Message::Paste`] completed: confirm multi-line/large pastes,
    /// otherwise send it straight away. `None`/empty means nothing to paste.
    pub(crate) fn on_paste_ready(&mut self, pane: PaneId, text: Option<String>) {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return;
        };
        let warning = self
            .config
            .terminal
            .warn_on_multiline_paste
            .then(|| input::needs_confirmation(&text))
            .flatten();
        match warning {
            Some(warning) => {
                self.pending_paste = Some(PendingPaste {
                    pane,
                    text,
                    warning,
                });
            }
            None => self.paste_now(pane, &text),
        }
    }

    /// Sanitises and sends `text` to the session. `terminal.bracketed_paste = false` forces plain
    /// paste even if the application asked for it.
    pub(crate) fn paste_now(&mut self, pane: PaneId, text: &str) {
        let terminal = &self.config.terminal;
        let (bracketed, trim, newlines, scroll) = (
            terminal.bracketed_paste,
            terminal.trim_whitespace_on_paste,
            terminal.replace_newlines_on_paste,
            terminal.scroll_on_input,
        );
        let Some(pane) = self.pane_mut(pane) else {
            return;
        };
        let opts = input::PasteOpts {
            bracketed_requested: bracketed && pane.term.mode().contains(TermMode::BRACKETED_PASTE),
            trim_whitespace: trim,
            newlines_to_spaces: newlines,
        };
        pane.send_input(input::encode_paste(text, opts), scroll);
    }

    /// The pending paste's confirmation modal, if any.
    pub(crate) fn paste_modal(&self) -> Option<Modal> {
        let pending = self.pending_paste.as_ref()?;
        let body = if pending.warning.lines > 1 {
            format!(
                "Paste {} lines ({} bytes)?",
                pending.warning.lines, pending.warning.bytes
            )
        } else {
            format!("Paste {} bytes with no line breaks?", pending.warning.bytes)
        };
        Some(Modal {
            title: "Paste",
            body,
            buttons: vec![
                ModalButton {
                    label: "Paste",
                    on_press: Message::ConfirmPaste,
                },
                ModalButton {
                    label: "Cancel",
                    on_press: Message::CancelPaste,
                },
            ],
        })
    }
}
