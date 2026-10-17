//! Restart (local) / reconnect (SSH) of a pane in place: any key that would type something into an
//! ended pane starts its profile again, and the pane keeps its screen and scrollback below a dim
//! separator (`terminal.keep_history_on_restart`).

use std::path::PathBuf;

use fogu_core::Size;
use fogu_term::input;
use fogu_term::{TermHandle, TermMode};
use iced::Task;

use crate::app::App;
use crate::message::Message;
use crate::tab::PaneId;

/// Separator written below a restarted local pane's old output.
pub(crate) const RESTARTED: &str = "\u{2500}\u{2500} restarted \u{2500}\u{2500}";
/// Separator written below a reconnected SSH pane's old output.
pub(crate) const RECONNECTED: &str = "\u{2500}\u{2500} reconnected \u{2500}\u{2500}";

/// `true` when `message` (a key or IME text) on an ended pane restarts it: "Press any key to
/// restart/reconnect", the key itself is not sent anywhere. Only input that would type something
/// counts (no bare modifiers, no key without an encoding); app shortcuts never get here, the
/// terminal widget turns them into actions first.
pub(crate) fn restarts(message: &Message, alt_is_meta: bool) -> bool {
    match message {
        Message::Key(key) => input::encode_key(key, TermMode::empty(), alt_is_meta)
            .is_some_and(|bytes| !bytes.is_empty()),
        Message::Text(text) => !text.is_empty(),
        _ => false,
    }
}

/// `true` for a plain Esc key press (cancels an automatic reconnect countdown).
pub(crate) fn is_escape(message: &Message, alt_is_meta: bool) -> bool {
    matches!(message, Message::Key(key)
        if input::encode_key(key, TermMode::empty(), alt_is_meta).as_deref() == Some(b"\x1b"))
}

impl App {
    /// A blank terminal of `size` with the configured scrollback and word separators.
    pub(crate) fn new_term(&self, size: Size) -> TermHandle {
        let terminal = &self.config.terminal;
        TermHandle::new(
            size.cols,
            size.rows,
            terminal.scrollback_lines,
            &terminal.word_separators,
        )
    }

    /// Replaces pane `id` with a new session of its profile, in the same place. An ended pane
    /// keeps its grid (reset to a clean state, see `TermHandle::reset_for_new_session`) when
    /// `terminal.keep_history_on_restart` is set; a running one (`RestartTab` / `Reconnect`)
    /// always gets a blank terminal, since its old session may still be writing.
    pub(crate) fn restart_pane(&mut self, id: PaneId) -> Task<Message> {
        let Some(index) = self.tab_of(id) else {
            return Task::none();
        };
        let keep = self.config.terminal.keep_history_on_restart;
        let Some(old) = self.tabs.get_mut(index).and_then(|t| t.panes.remove(&id)) else {
            return Task::none();
        };
        let (profile, size) = (old.profile.clone(), old.size());
        let remote_cwd = old.remote_cwd.clone().map(PathBuf::from);
        let kept = keep && !old.state.is_running();
        let term = if kept {
            let separator = if old.is_ssh() { RECONNECTED } else { RESTARTED };
            let mut term = old.term;
            term.reset_for_new_session(separator);
            term
        } else {
            // Dropping the old pane closes its input channel: the old session ends itself.
            drop(old);
            self.new_term(size)
        };
        let pane = self.start_pane(&profile, remote_cwd, (term, kept), size);
        let new = pane.id;
        let Some(tab) = self.tabs.get_mut(index) else {
            return Task::none();
        };
        tab.panes.insert(new, pane);
        let layout = std::mem::replace(&mut tab.layout, fogu_core::SplitTree::new(new));
        tab.layout = layout.map(|leaf| if leaf == id { new } else { leaf });
        if tab.focused == id {
            tab.focused = new;
        }
        self.rebind_panel(id, new)
    }
}

#[cfg(test)]
#[path = "restart_tests.rs"]
mod tests;
