//! Pane-level operations inside a tab's split tree: split, close, navigate, focus,
//! divider ratios and the per-pane resize path.

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
const SPLIT_CWD_WAIT: Duration = Duration::from_millis(500);

/// What a pending cwd query is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CwdFor {
    /// `NewTabHere`: open a tab there.
    Tab,
    /// `SplitRight`/`SplitDown`: open the new pane there, on this side of the queried pane.
    Split(Side),
}

/// What a close request (keymap, `×`, middle-click) is for; kept while the "close live SSH
/// session?" modal is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseTarget {
    /// A whole tab.
    Tab(TabId),
    /// One pane.
    Pane(PaneId),
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
        let Some(profile) = profile.or_else(|| self.pane(target).map(|p| p.profile.clone())) else {
            return;
        };
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
        if let CwdFor::Split(side) = query.purpose {
            self.pending_cwd = None;
            self.split_now(pane, side, None, None);
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
            CloseTarget::Tab(id) => self
                .tabs
                .iter()
                .find(|t| t.id == id)
                .is_some_and(|t| t.panes.values().any(|p| self.warns_on_close(p))),
            CloseTarget::Pane(id) => self.pane(id).is_some_and(|p| self.warns_on_close(p)),
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
        match target {
            CloseTarget::Tab(id) => self.close_tab(id),
            CloseTarget::Pane(id) => self.close_pane(id),
        }
    }

    /// The "close live SSH session?" modal, if one is pending.
    pub(crate) fn close_modal(&self) -> Option<Modal> {
        self.pending_close?;
        Some(Modal {
            title: "Close SSH session?",
            body: "The connection is still open; closing ends it.".to_owned(),
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
    /// inline rename and takes keyboard focus back from the tab bar.
    pub(crate) fn focus_pane(&mut self, id: PaneId) {
        self.commit_rename();
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
