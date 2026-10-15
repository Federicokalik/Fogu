//! Tab lifecycle and keymap actions: new / close / switch / rename / colour / restart / new tab
//! here, and session events routed to their pane. Pane-level operations live in `panes.rs`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use fogu_core::{Action, EffectiveProfile, Profile, ProfileKind, Side, Size, resolve};
use fogu_session::{SessionHandle, SessionInput, SessionOutput};
use fogu_term::{Scroll, TermHandle, input, sanitize_title};
use iced::widget::operation;
use iced::{Task, clipboard, window};

use crate::app::App;
use crate::appearance::cursor_shape;
use crate::context_menu::{ContextMenu, MENU_INPUT, parse_color};
use crate::message::{Message, PasteSource};
use crate::notifications::{Level, Notification};
use crate::panes::{CloseTarget, CwdFor, CwdQuery};
use crate::tab::{Pane, PaneId, PaneState, Tab, TabId};
use crate::tab_bar::{RENAME_INPUT, Rename};

/// Grid of the first tab, before the first layout resizes it.
const INITIAL_SIZE: Size = Size {
    cols: 80,
    rows: 24,
    px_w: 0,
    px_h: 0,
};
/// How long closing a pane waits for `Exited` before dropping it anyway.
pub(crate) const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// How long a cwd query stays valid.
const CWD_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

impl App {
    pub(crate) fn active_index(&self) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == self.active)
    }

    pub(crate) fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.active)
    }

    /// The active tab's focused pane.
    pub(crate) fn active_pane(&self) -> Option<&Pane> {
        self.active_tab().and_then(Tab::focused_pane)
    }

    pub(crate) fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        let active = self.active;
        self.tabs
            .iter_mut()
            .find(|t| t.id == active)
            .and_then(Tab::focused_pane_mut)
    }

    /// Index of the tab holding pane `id`.
    pub(crate) fn tab_of(&self, id: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.panes.contains_key(&id))
    }

    pub(crate) fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.tabs.iter().find_map(|t| t.panes.get(&id))
    }

    pub(crate) fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.tabs.iter_mut().find_map(|t| t.panes.get_mut(&id))
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// `config.default_profile`, or the built-in default shell.
    pub(crate) fn default_profile(&self) -> Profile {
        let config = &self.config;
        config
            .default_profile
            .as_ref()
            .and_then(|id| config.profiles.iter().find(|p| &p.id == id))
            .cloned()
            .unwrap_or_else(Profile::default_shell)
    }

    /// Opens `profile` (local `cwd` override for "new tab here") in a new last tab and
    /// activates it.
    pub(crate) fn open_tab(&mut self, profile: &Profile, cwd: Option<PathBuf>) -> TabId {
        let pane = self.open_pane(profile, cwd);
        let id = TabId(self.take_id());
        self.tabs.push(Tab::new(id, pane));
        self.activate(id);
        id
    }

    /// A new pane running `profile`, sized like the active pane.
    pub(crate) fn open_pane(&mut self, profile: &Profile, cwd: Option<PathBuf>) -> Pane {
        let size = self.active_pane().map_or(INITIAL_SIZE, Pane::size);
        let term = self.new_term(size);
        self.start_pane(profile, cwd, (term, false), size)
    }

    /// A new pane running `profile` on `term` of `size` cells: blank, or (`kept_history`) a
    /// restarted pane's grid already reset with [`TermHandle::reset_for_new_session`], which an
    /// SSH session's `clear_service_messages_on_connect` must not wipe.
    pub(crate) fn start_pane(
        &mut self,
        profile: &Profile,
        cwd: Option<PathBuf>,
        (mut term, kept_history): (TermHandle, bool),
        size: Size,
    ) -> Pane {
        term.set_palette(self.colors.palette());
        term.set_default_cursor_shape(cursor_shape(self.config.appearance.cursor_shape));
        term.set_cell_size_px(self.metrics.cell_w_px, self.metrics.cell_h_px);
        let mut effective = resolve(profile, &self.config.profile_defaults);
        if let (Some(cwd), EffectiveProfile::Local(local)) = (cwd, &mut effective) {
            local.cwd = Some(cwd);
        }
        if let (true, EffectiveProfile::Ssh(ssh)) = (kept_history, &mut effective) {
            ssh.clear_service_messages_on_connect = false;
        }
        let size = Size {
            px_w: size.cols.saturating_mul(self.metrics.cell_w_px),
            px_h: size.rows.saturating_mul(self.metrics.cell_h_px),
            ..size
        };
        let default_title = match &effective {
            EffectiveProfile::Ssh(ssh) => sanitize_title(&format!("{}@{}", ssh.user, ssh.host)),
            EffectiveProfile::Local(_) => profile.name.clone(),
        };
        let handle = self.open_session(&effective, size, term.share());
        let id = PaneId(self.take_id());
        Pane::new(id, profile.clone(), default_title, term, handle)
    }

    /// SSH sessions keep their `known_hosts` next to the config (`--config-dir`).
    #[cfg(not(test))]
    fn open_session(
        &mut self,
        profile: &EffectiveProfile,
        size: Size,
        term: TermHandle,
    ) -> SessionHandle {
        let rt = self.runtime.handle();
        #[cfg(feature = "ssh")]
        if let (EffectiveProfile::Ssh(ssh), Some(dir)) = (profile, &self.config_dir) {
            let paths = fogu_session::SshPaths::in_config_dir(dir);
            return fogu_session::open_ssh(rt, ssh, paths, size, term);
        }
        fogu_session::open(rt, profile, size, term)
    }

    /// Test builds: an inert session (no process). Its input end is kept in `stub_inputs`; its
    /// event sender is dropped, so the event stream is empty.
    #[cfg(test)]
    fn open_session(&mut self, p: &EffectiveProfile, _: Size, _: TermHandle) -> SessionHandle {
        self.stub_profiles.push(p.clone());
        let (input, input_rx) = tokio::sync::mpsc::channel(8);
        let (_, events) = tokio::sync::mpsc::channel(8);
        self.stub_inputs.push(input_rx);
        SessionHandle { input, events }
    }

    /// Makes `id` the visible tab: focus out/in reports, fresh snapshots (inactive panes skip
    /// wakeup refreshes).
    pub(crate) fn activate(&mut self, id: TabId) {
        if id == self.active && self.active_pane().is_some() {
            return;
        }
        self.focus_report(false);
        self.active = id;
        self.focus_report(true);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.panes.values_mut().for_each(Pane::refresh);
        }
    }

    /// Sends a focus in/out report to the active pane, if the app asked for them and the
    /// window is focused.
    pub(crate) fn focus_report(&self, focused: bool) {
        if let Some(pane) = self.active_pane().filter(|_| self.focused)
            && let Some(bytes) = input::encode_focus(focused, pane.term.mode())
        {
            pane.write_input(bytes);
        }
    }

    /// Handles a keymap / menu action.
    pub(crate) fn on_action(&mut self, action: Action) -> Task<Message> {
        self.menu = None;
        match action {
            Action::NewTab => {
                let profile = self.default_profile();
                self.open_tab(&profile, None);
            }
            Action::NewTabHere => {
                if let Some(pane) = self.active_pane().map(|p| p.id) {
                    self.ask_cwd(pane, CwdFor::Tab);
                }
            }
            Action::CloseTab => return self.request_close(CloseTarget::Tab(self.active)),
            Action::NextTab | Action::PrevTab | Action::GoToTab(_) => self.switch(action),
            Action::RenameTab => return self.start_rename(self.active),
            Action::RestartTab => {
                if let Some(pane) = self.active_pane().map(|p| p.id) {
                    self.restart_pane(pane);
                }
            }
            Action::Copy => {
                if let Some(text) = self.active_pane().and_then(|p| p.term.selection_text()) {
                    return clipboard::write(text);
                }
            }
            Action::Paste => return self.update(Message::Paste(PasteSource::Clipboard)),
            Action::PastePrimary => return self.update(Message::Paste(PasteSource::Primary)),
            Action::ZoomIn => self.set_font_size(self.font.size_pt + 1.0),
            Action::ZoomOut => self.set_font_size(self.font.size_pt - 1.0),
            Action::ZoomReset => self.zoom_reset(),
            Action::Clear => {
                if let Some(pane) = self.active_pane_mut() {
                    pane.term.clear();
                    pane.refresh();
                }
            }
            Action::ScrollPageUp => return self.update(Message::Scroll(Scroll::PageUp)),
            Action::ScrollPageDown => return self.update(Message::Scroll(Scroll::PageDown)),
            Action::ScrollTop => return self.update(Message::Scroll(Scroll::Top)),
            Action::ScrollBottom => return self.update(Message::Scroll(Scroll::Bottom)),
            Action::ToggleFullscreen => {
                self.fullscreen = !self.fullscreen;
                let mode = if self.fullscreen {
                    window::Mode::Fullscreen
                } else {
                    window::Mode::Windowed
                };
                return window::oldest().and_then(move |id| window::set_mode(id, mode));
            }
            Action::SplitRight => return self.split_focused(Side::Right),
            Action::SplitDown => return self.split_focused(Side::Bottom),
            Action::ClosePane => {
                if let Some(pane) = self.active_pane().map(|p| p.id) {
                    return self.request_close(CloseTarget::Pane(pane));
                }
            }
            Action::PaneNav(dir) => self.pane_nav(dir),
            Action::Reconnect => {
                // A new session for an SSH pane or any ended pane (history: `restart_pane`).
                if let Some(pane) = self
                    .active_pane()
                    .filter(|p| p.is_ssh() || !p.state.is_running())
                {
                    self.restart_pane(pane.id);
                }
            }
            Action::OpenProfilePicker => return self.open_picker(),
            // Not implemented yet; the user is told rather than left guessing.
            Action::SelectAll => {
                tracing::debug!(%action, "action not implemented yet");
                let note = Notification::new(Level::Info, "Not available yet")
                    .detail(format!("\"{action}\" arrives with context menus"));
                return self.notify(note);
            }
        }
        Task::none()
    }

    /// `NextTab` / `PrevTab` (wrapping) / `GoToTab(n)` (1-based; out of range is ignored).
    fn switch(&mut self, action: Action) {
        let (Some(index), len) = (self.active_index(), self.tabs.len()) else {
            return;
        };
        let target = match action {
            Action::NextTab => (index + 1) % len,
            Action::PrevTab => (index + len - 1) % len,
            Action::GoToTab(n) => usize::from(n).saturating_sub(1),
            _ => return,
        };
        if let Some(id) = self.tabs.get(target).map(|t| t.id) {
            self.activate(id);
        }
    }

    /// Asks pane `pane` for its cwd on behalf of `purpose`; `false` when the session is gone.
    pub(crate) fn ask_cwd(&mut self, pane: PaneId, purpose: CwdFor) -> bool {
        let asked = self
            .pane(pane)
            .is_some_and(|p| p.control(SessionInput::QueryCwd));
        if asked {
            self.pending_cwd = Some(CwdQuery {
                pane,
                asked: Instant::now(),
                purpose,
            });
        }
        asked
    }

    /// Closes every pane of tab `id`; the tab goes with its last pane.
    pub(crate) fn close_tab(&mut self, id: TabId) -> Task<Message> {
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return Task::none();
        };
        let panes: Vec<PaneId> = tab.panes.keys().copied().collect();
        Task::batch(panes.into_iter().map(|pane| self.close_pane(pane)))
    }

    /// Drops tab `id` (its panes close their sessions); exits the app after the last tab.
    pub(crate) fn remove_tab(&mut self, id: TabId) -> Task<Message> {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return Task::none();
        };
        self.tabs.remove(index);
        if self.renaming.as_ref().is_some_and(|r| r.tab == id) {
            self.renaming = None;
        }
        if self.hovered_tab == Some(id) {
            self.hovered_tab = None;
        }
        if self.tabs.is_empty() {
            return iced::exit();
        }
        if self.active == id {
            self.menu = None;
            let next = index.min(self.tabs.len() - 1);
            if let Some(next) = self.tabs.get(next).map(|t| t.id) {
                self.activate(next);
            }
        }
        Task::none()
    }

    fn start_rename(&mut self, id: TabId) -> Task<Message> {
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return Task::none();
        };
        let value = tab.title().to_owned();
        self.activate(id);
        self.menu = None;
        self.renaming = Some(Rename { tab: id, value });
        Task::batch([
            operation::focus(RENAME_INPUT),
            operation::select_all(RENAME_INPUT),
        ])
    }

    /// Applies the inline rename: sanitized, empty clears the override.
    pub(crate) fn commit_rename(&mut self) {
        let Some(rename) = self.renaming.take() else {
            return;
        };
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == rename.tab) {
            let title = sanitize_title(rename.value.trim());
            tab.title_override = (!title.is_empty()).then_some(title);
        }
    }

    /// Tab bar, rename and context-menu messages.
    pub(crate) fn on_tab_ui(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ActivateTab(id) => self.activate(id),
            Message::CloseTab(id) => return self.request_close(CloseTarget::Tab(id)),
            Message::HoverTab(id) => self.hovered_tab = id,
            Message::TabBarCursor(at) => self.tab_bar_cursor = at,
            Message::StartRename(id) => return self.start_rename(id),
            Message::RenameInput(value) => {
                if let Some(rename) = &mut self.renaming {
                    rename.value = value;
                }
            }
            Message::CommitRename => self.commit_rename(),
            Message::OpenTabMenu(id) => {
                self.commit_rename();
                self.activate(id);
                self.menu = Some(ContextMenu::tab(self.tab_bar_cursor));
            }
            Message::MenuColors => {
                let anchor = self.menu.as_ref().map_or(self.tab_bar_cursor, |m| m.anchor);
                self.menu = Some(ContextMenu::tab_colors(anchor));
                return operation::focus(MENU_INPUT);
            }
            Message::MenuInput(value) => {
                if let Some(input) = self.menu.as_mut().and_then(|m| m.input.as_mut()) {
                    input.value = value;
                    input.invalid = false;
                }
            }
            Message::MenuSubmit => return self.submit_menu_input(),
            Message::SetTabColor(color) => {
                self.menu = None;
                let active = self.active;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == active) {
                    tab.color = color;
                }
            }
            Message::CloseMenu => self.menu = None,
            Message::Escape => self.on_escape(),
            _ => {}
        }
        Task::none()
    }

    /// "Custom hex…": a valid colour applies to the active tab, anything else marks the field.
    fn submit_menu_input(&mut self) -> Task<Message> {
        let Some(input) = self.menu.as_mut().and_then(|m| m.input.as_mut()) else {
            return Task::none();
        };
        if let Some(color) = parse_color(&input.value) {
            return self.update(Message::SetTabColor(Some(color)));
        }
        input.invalid = true;
        Task::none()
    }

    /// Routes a session event to its pane; events from panes that no longer exist are dropped.
    pub(crate) fn on_session(&mut self, pane_id: PaneId, event: SessionOutput) -> Task<Message> {
        let active = self.active;
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| t.panes.contains_key(&pane_id))
        else {
            return Task::none();
        };
        let is_active = tab.id == active;
        let Some(pane) = tab.panes.get_mut(&pane_id) else {
            return Task::none();
        };
        match event {
            // Inactive panes keep `dirty` set (no further wakeups) until activated.
            SessionOutput::Wakeup if is_active => pane.refresh(),
            SessionOutput::Title(title) => {
                pane.title_from_osc = (!title.is_empty()).then_some(title);
            }
            SessionOutput::Exited(info) => {
                tracing::info!(?info, "session exited");
                return self.on_pane_ended(pane_id, PaneState::Exited(info));
            }
            SessionOutput::Error(err) => return self.on_session_error(pane_id, &err),
            SessionOutput::ClipboardStore(request) => {
                return self.on_clipboard_store(pane_id, request);
            }
            SessionOutput::InputDropped => {
                let note = Notification::new(Level::Warn, "Input dropped").detail(
                    "The program stopped reading its input; further typing or paste was discarded.",
                );
                return self.notify(note);
            }
            SessionOutput::Cwd { path, remote } => self.on_cwd(pane_id, path, remote),
            SessionOutput::Prompt(id, kind) => return self.on_prompt(pane_id, id, kind),
            SessionOutput::Warning(warning) => {
                if !pane.warnings.contains(&warning) {
                    pane.warnings.push(warning.clone());
                }
                return self.on_session_warning(&warning);
            }
            // No UI for these yet; Progress/Notify/PromptMark are logged by the session.
            SessionOutput::Wakeup
            | SessionOutput::Bell
            | SessionOutput::Progress(_)
            | SessionOutput::Notify { .. }
            | SessionOutput::PromptMark(_) => {}
        }
        Task::none()
    }

    /// A cwd report. Attacker-controlled text: only logged sanitized, and only a local
    /// (`remote: false`) path answering a fresh query opens a tab or a split.
    fn on_cwd(&mut self, pane_id: PaneId, path: PathBuf, remote: bool) {
        tracing::debug!(
            cwd = %sanitize_title(&path.to_string_lossy()),
            remote,
            "cwd reported"
        );
        let Some(query) = self.pending_cwd else {
            return;
        };
        if query.pane != pane_id || remote || query.asked.elapsed() > CWD_QUERY_TIMEOUT {
            return;
        }
        self.pending_cwd = None;
        match query.purpose {
            CwdFor::Tab => {
                let profile = self
                    .pane(pane_id)
                    .map(|p| p.profile.clone())
                    .filter(|p| matches!(p.kind, ProfileKind::Local(_)))
                    .unwrap_or_else(Profile::default_shell);
                self.open_tab(&profile, Some(path));
            }
            CwdFor::Split(side) => self.split_now(pane_id, side, Some(path), None),
        }
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
