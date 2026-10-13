//! Tabs and their panes: one pane per tab for now.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fogu_core::{Profile, RgbColor, Size};
use fogu_session::{SessionHandle, SessionInput};
use fogu_term::{RenderSnapshot, Scroll, TermHandle};
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::subscription::EventSlot;

/// Identifies a tab; monotonic, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Identifies a pane (and its session subscription); monotonic, never reused, so events from a
/// restarted tab's old session can't reach the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(pub u64);

/// Lifecycle of a pane's session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneState {
    /// The session is running.
    Running,
    /// The process ended.
    Exited,
    /// The session failed or its task panicked.
    Crashed,
}

/// One terminal pane: its grid, the session's input side and the last drawn snapshot.
#[derive(Debug)]
pub struct Pane {
    pub id: PaneId,
    pub term: TermHandle,
    dirty: Arc<AtomicBool>,
    session_input: Sender<SessionInput>,
    pub events: EventSlot,
    pub state: PaneState,
    pub snapshot: RenderSnapshot,
    /// Bumped per resize; older debounce timers are stale.
    pub resize_generation: u64,
    /// PTY size waiting for the debounce timer.
    pub pending_resize: Option<Size>,
}

impl Pane {
    /// Wraps a freshly opened session `handle` feeding `term`.
    pub fn new(id: PaneId, term: TermHandle, handle: SessionHandle) -> Self {
        Self {
            id,
            dirty: term.dirty(),
            snapshot: term.snapshot(),
            term,
            session_input: handle.input,
            events: EventSlot::new(id.0, handle.events),
            state: PaneState::Running,
            resize_generation: 0,
            pending_resize: None,
        }
    }

    /// Clears `dirty` *before* snapshotting, so output arriving meanwhile sends a new wakeup.
    pub fn refresh(&mut self) {
        self.dirty.store(false, Ordering::SeqCst);
        self.snapshot = self.term.snapshot();
    }

    /// Sends a control message without blocking the UI. `false` when the session is gone
    /// (closed channel) or busy (full; logged).
    pub fn control(&self, input: SessionInput) -> bool {
        match self.session_input.try_send(input) {
            Ok(()) => true,
            Err(TrySendError::Closed(_)) => false,
            Err(TrySendError::Full(_)) => {
                tracing::warn!("session input full; control message dropped");
                false
            }
        }
    }

    /// Writes `bytes` to a running session without blocking the UI (drops them when full).
    pub fn write_input(&self, bytes: Vec<u8>) {
        if self.state == PaneState::Running {
            self.control(SessionInput::Data(bytes.into()));
        }
    }

    /// Writes encoded keys, then scrolls to the bottom.
    pub fn send_input(&mut self, bytes: Vec<u8>) {
        if self.state != PaneState::Running {
            return;
        }
        self.write_input(bytes);
        if self.snapshot.display_offset != 0 {
            self.term.scroll_display(Scroll::Bottom);
            self.refresh();
        }
    }

    /// Current grid size (for opening a sibling tab at the same size).
    pub fn size(&self) -> Size {
        Size {
            cols: u16::try_from(self.snapshot.cols).unwrap_or(u16::MAX),
            rows: u16::try_from(self.snapshot.rows).unwrap_or(u16::MAX),
            ..Size::default()
        }
    }
}

/// One tab: a profile, its title sources, colour and (for now) a single pane.
#[derive(Debug)]
pub struct Tab {
    pub id: TabId,
    /// The profile the tab was opened from (`RestartTab` reopens it).
    pub profile: Profile,
    /// Last OSC 0/2 title (sanitized by the session; `None` after a reset).
    pub title_from_osc: Option<String>,
    /// The user's rename; wins over everything until cleared.
    pub title_override: Option<String>,
    /// Header tint + pane border; defaults to the profile's colour.
    pub color: Option<RgbColor>,
    pub pane: Pane,
    /// `CloseTab` sent `Close`; removed on `Exited` or after the grace period.
    pub closing: bool,
}

impl Tab {
    /// A new tab for `profile` around `pane`.
    pub fn new(id: TabId, profile: Profile, pane: Pane) -> Self {
        Self {
            id,
            color: profile.color,
            profile,
            title_from_osc: None,
            title_override: None,
            pane,
            closing: false,
        }
    }

    /// The title shown in the header and the window title.
    pub fn title(&self) -> &str {
        resolve_title(
            self.title_override.as_deref(),
            self.title_from_osc.as_deref(),
            self.profile.disable_dynamic_title,
            &self.profile.name,
        )
    }
}

/// Title precedence: user override > OSC title (unless the profile disables dynamic titles) >
/// profile name.
pub fn resolve_title<'a>(
    title_override: Option<&'a str>,
    from_osc: Option<&'a str>,
    disable_dynamic_title: bool,
    profile_name: &'a str,
) -> &'a str {
    title_override
        .or(from_osc.filter(|_| !disable_dynamic_title))
        .unwrap_or(profile_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_precedence_table() {
        let rows = [
            // (override, osc, disable_dynamic, want)
            (Some("work"), Some("vim"), false, "work"),
            (Some("work"), None, true, "work"),
            (None, Some("vim"), false, "vim"),
            (None, Some("vim"), true, "Profile"),
            (None, None, false, "Profile"),
            (None, None, true, "Profile"),
        ];
        for (ovr, osc, disable, want) in rows {
            assert_eq!(
                resolve_title(ovr, osc, disable, "Profile"),
                want,
                "{ovr:?} {osc:?} {disable}"
            );
        }
    }
}
