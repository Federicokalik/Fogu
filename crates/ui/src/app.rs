//! Application state, `update` dispatch, `view` and subscriptions. Tab lifecycle and actions live
//! in `actions.rs`, split/pane operations and resizing in `panes.rs`; pane-level input (keys,
//! paste, selection) here.

use std::sync::Arc;

use iced::keyboard::{self, key::Named};
use std::time::Duration;

use fogu_core::{ColorScheme, Config};
use fogu_session::SessionRuntime;
use fogu_term::input::KeyCode;
use fogu_term::{TermMode, input};
use iced::widget::column;
use iced::{Element, Point, Subscription, Task, clipboard, event, window};

use crate::context_menu::{self, ContextMenu};
use crate::message::{Message, PasteSource, SelectEvent};
use crate::metrics::{self, CellMetrics, FontSpec, ResolvedFont, viewport_to_point};
use crate::modal::{Modal, ModalButton, overlay};
use crate::mouse::MouseConfig;
use crate::palette::{Colors, to_iced};
use crate::panes::CwdQuery;
use crate::split_container::SplitContainer;
use crate::subscription::session_events;
use crate::tab::{PaneId, PaneState, Tab, TabId};
use crate::tab_bar::{Rename, TabBar};
use crate::terminal_view::TerminalView;

pub(crate) const DEFAULT_TITLE: &str = "Fogu";
/// Smallest font size zoom allows, in points.
const MIN_FONT_PT: f32 = 6.0;
/// Largest font size zoom allows, in points.
const MAX_FONT_PT: f32 = 40.0;
/// Initial window size (also the menu-clamping size until the first resize event).
pub(crate) const WINDOW_SIZE: iced::Size = iced::Size::new(1000.0, 650.0);

/// A multi-line/large paste waiting on the user's "Paste"/"Cancel" choice in the modal.
#[derive(Debug)]
struct PendingPaste {
    pane: PaneId,
    text: String,
    warning: input::PasteWarning,
}

/// The whole application state.
#[derive(Debug)]
pub struct App {
    pub(crate) runtime: Arc<SessionRuntime>,
    pub(crate) config: Arc<Config>,
    pub(crate) tabs: Vec<Tab>,
    /// The visible tab; meaningless only while `tabs` is empty (the app is exiting).
    pub(crate) active: TabId,
    pub(crate) menu: Option<ContextMenu>,
    pub(crate) renaming: Option<Rename>,
    pub(crate) hovered_tab: Option<TabId>,
    /// Last pointer position over the tab bar: the tab menu's anchor.
    pub(crate) tab_bar_cursor: Point,
    pub(crate) window_size: iced::Size,
    pub(crate) fullscreen: bool,
    /// The cwd query in flight (`NewTabHere`, splits).
    pub(crate) pending_cwd: Option<CwdQuery>,
    /// Source of every `TabId`/`PaneId`.
    pub(crate) next_id: u64,
    pub(crate) font: FontSpec,
    /// `config.appearance.font_size` at boot, in points; `ZoomReset`'s target.
    default_font_size_pt: f32,
    resolved: ResolvedFont,
    pub(crate) metrics: CellMetrics,
    scale: f32,
    pub(crate) colors: Colors,
    pub(crate) focused: bool,
    pending_paste: Option<PendingPaste>,
    /// Test builds: input ends of the stub sessions, to inspect what the UI sent.
    #[cfg(test)]
    pub(crate) stub_inputs: Vec<tokio::sync::mpsc::Receiver<fogu_session::SessionInput>>,
}

impl App {
    /// State with no tabs yet.
    pub(crate) fn new(
        runtime: Arc<SessionRuntime>,
        config: Arc<Config>,
        resolved: ResolvedFont,
    ) -> Self {
        let font = FontSpec {
            family: config.appearance.font_family.clone(),
            size_pt: config.appearance.font_size,
        };
        let metrics = CellMetrics::new(resolved.units, font.size_px(), 1.0);
        let scheme = ColorScheme::by_name(&config.appearance.color_scheme).unwrap_or_default();
        Self {
            runtime,
            tabs: Vec::new(),
            active: TabId(0),
            menu: None,
            renaming: None,
            hovered_tab: None,
            tab_bar_cursor: Point::ORIGIN,
            window_size: WINDOW_SIZE,
            fullscreen: false,
            pending_cwd: None,
            next_id: 1,
            default_font_size_pt: font.size_pt,
            font,
            resolved,
            metrics,
            scale: 1.0,
            colors: Colors::new(&scheme),
            focused: true,
            pending_paste: None,
            config,
            #[cfg(test)]
            stub_inputs: Vec::new(),
        }
    }

    /// Opens the default profile in the first tab and asks for the window's scale factor.
    pub fn boot(runtime: Arc<SessionRuntime>, config: Arc<Config>) -> (Self, Task<Message>) {
        let resolved = metrics::resolve(&config.appearance.font_family);
        let mut app = Self::new(runtime, config, resolved);
        let profile = app.default_profile();
        app.open_tab(profile, None);
        let scale = window::oldest()
            .and_then(window::scale_factor)
            .map(Message::ScaleFactor);
        (app, scale)
    }

    /// Window title = the active tab's title.
    pub fn title(&self) -> String {
        match self.active_tab().map(Tab::title) {
            Some(title) if !title.is_empty() => title.to_owned(),
            _ => DEFAULT_TITLE.to_owned(),
        }
    }

    /// Handles one message.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Session(pane, event) => self.on_session(pane, event),
            Message::Action(action) => self.on_action(action),
            Message::GridResized { pane, cols, rows } => self.on_grid_resized(pane, cols, rows),
            Message::ResizeTimer(pane, generation) => self.on_resize_timer(pane, generation),
            Message::CloseTimeout(pane) => self.on_close_timeout(pane),
            Message::ScaleFactor(scale) => {
                self.scale = scale;
                self.apply_metrics();
                Task::none()
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                Task::none()
            }
            Message::Focused(focused) => {
                self.on_focused(focused);
                Task::none()
            }
            Message::SplitRatio(..)
            | Message::Equalize(_)
            | Message::FocusPane(_)
            | Message::CwdTimeout(_) => self.on_split_ui(message),
            Message::Pane(pane, inner) => self.on_pane_input(Some(pane), *inner),
            Message::Key(_)
            | Message::Text(_)
            | Message::MouseInput(_)
            | Message::Select(_)
            | Message::Scroll(_)
            | Message::Paste(_)
            | Message::PasteReady(_)
            | Message::ConfirmPaste
            | Message::CancelPaste => self.on_pane_input(None, message),
            other => self.on_tab_ui(other),
        }
    }

    /// Keys, mouse and paste for pane `target` (default: the active tab's focused pane).
    fn on_pane_input(&mut self, target: Option<PaneId>, message: Message) -> Task<Message> {
        let alt_is_meta = self.config.terminal.alt_is_meta;
        let Some(id) = target.or_else(|| self.active_pane().map(|p| p.id)) else {
            return Task::none();
        };
        let Some(pane) = self.pane_mut(id) else {
            return Task::none();
        };
        match message {
            Message::Key(key) if pane.state != PaneState::Running => {
                if matches!(key.key, KeyCode::Enter | KeyCode::NumpadEnter) && !pane.closing {
                    self.restart_pane(id);
                }
            }
            Message::Key(key) => {
                if let Some(bytes) = input::encode_key(&key, pane.term.mode(), alt_is_meta) {
                    pane.send_input(bytes);
                }
            }
            Message::Text(text) => {
                if let Some(bytes) = input::encode_text(&text) {
                    pane.send_input(bytes);
                }
            }
            Message::MouseInput(bytes) => pane.write_input(bytes),
            Message::Select(event) => return self.on_select(id, event),
            Message::Scroll(scroll) => {
                pane.term.scroll_display(scroll);
                pane.refresh();
            }
            Message::Paste(source) => {
                return match source {
                    PasteSource::Clipboard => clipboard::read(),
                    PasteSource::Primary => clipboard::read_primary(),
                }
                .map(move |text| Message::Pane(id, Box::new(Message::PasteReady(text))));
            }
            Message::PasteReady(text) => self.on_paste_ready(id, text),
            Message::ConfirmPaste => {
                if let Some(pending) = self.pending_paste.take() {
                    self.paste_now(pending.pane, &pending.text);
                }
            }
            Message::CancelPaste => self.pending_paste = None,
            _ => {}
        }
        Task::none()
    }

    /// Window focus, or a click into a pane (which also commits an inline rename).
    pub(crate) fn on_focused(&mut self, focused: bool) {
        if focused && self.renaming.is_some() {
            self.commit_rename();
        }
        if focused != self.focused
            && let Some(pane) = self.active_pane()
            && let Some(bytes) = input::encode_focus(focused, pane.term.mode())
        {
            pane.write_input(bytes);
        }
        self.focused = focused;
    }

    /// A clipboard read for [`Message::Paste`] completed: confirm multi-line/large pastes,
    /// otherwise send it straight away. `None`/empty means nothing to paste.
    fn on_paste_ready(&mut self, pane: PaneId, text: Option<String>) {
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
    fn paste_now(&mut self, pane: PaneId, text: &str) {
        let terminal = &self.config.terminal;
        let (bracketed, trim, newlines) = (
            terminal.bracketed_paste,
            terminal.trim_whitespace_on_paste,
            terminal.replace_newlines_on_paste,
        );
        let Some(pane) = self.pane_mut(pane) else {
            return;
        };
        let opts = input::PasteOpts {
            bracketed_requested: bracketed && pane.term.mode().contains(TermMode::BRACKETED_PASTE),
            trim_whitespace: trim,
            newlines_to_spaces: newlines,
        };
        pane.send_input(input::encode_paste(text, opts));
    }

    /// Clamps to [`MIN_FONT_PT`]..=[`MAX_FONT_PT`] and recomputes cell metrics; the next frame's
    /// `GridResized` picks up the new grid size and resizes the
    /// PTY, same as a window resize.
    pub(crate) fn set_font_size(&mut self, size_pt: f32) {
        self.font.size_pt = size_pt.clamp(MIN_FONT_PT, MAX_FONT_PT);
        self.apply_metrics();
    }

    pub(crate) fn zoom_reset(&mut self) {
        self.set_font_size(self.default_font_size_pt);
    }

    /// Recomputes cell metrics from font + scale and tells every pane's terminal.
    fn apply_metrics(&mut self) {
        self.metrics = CellMetrics::new(self.resolved.units, self.font.size_px(), self.scale);
        let m = &self.metrics;
        for pane in self.tabs.iter_mut().flat_map(|t| t.panes.values_mut()) {
            pane.term.set_cell_size_px(m.cell_w_px, m.cell_h_px);
        }
    }

    /// The pending paste's confirmation modal, if any.
    fn paste_modal(&self) -> Option<Modal> {
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

    /// Applies a selection gesture; viewport cells map to grid lines with the current scroll.
    fn on_select(&mut self, pane: PaneId, event: SelectEvent) -> Task<Message> {
        let copy_on_select = self.config.terminal.copy_on_select;
        let Some(pane) = self.pane_mut(pane) else {
            return Task::none();
        };
        let offset = pane.snapshot.display_offset;
        let term = &mut pane.term;
        match event {
            SelectEvent::Start { kind, at } => {
                term.start_selection(kind, viewport_to_point(at, offset), at.side);
            }
            SelectEvent::Update(at) => {
                term.update_selection(viewport_to_point(at, offset), at.side);
            }
            SelectEvent::Clear => term.clear_selection(),
            SelectEvent::Finish => {
                return match term.selection_text() {
                    Some(text) if copy_on_select => clipboard::write_primary(text),
                    _ => Task::none(),
                };
            }
        }
        pane.refresh();
        Task::none()
    }

    /// `message` after `delay` (timer on the session runtime).
    pub(crate) fn after(&self, delay: Duration, message: Message) -> Task<Message> {
        // The sleep is created inside the task: it needs the runtime's timer.
        let timer = self
            .runtime
            .handle()
            .spawn(async move { tokio::time::sleep(delay).await });
        Task::perform(timer, move |_| message.clone())
    }

    /// Tab bar above the active tab's split container, plus modal / menu overlays.
    pub fn view(&self) -> Element<'_, Message> {
        let Some(tab) = self.active_tab() else {
            return column![].into();
        };
        let typing_elsewhere = self.renaming.is_some() || self.menu.is_some();
        let mouse = MouseConfig {
            wheel_lines: self.config.terminal.wheel_lines,
            paste_on_middle_click: self.config.terminal.paste_on_middle_click,
        };
        let panes = tab
            .layout
            .leaves()
            .into_iter()
            .filter_map(|id| tab.panes.get(id))
            .map(|pane| {
                let id = pane.id;
                let focused = id == tab.focused && self.focused && !typing_elsewhere;
                let view: Element<'_, Message> = TerminalView::new(
                    id,
                    &pane.snapshot,
                    &self.colors,
                    self.metrics,
                    self.resolved.font,
                    focused,
                    mouse,
                )
                .status(pane.status.as_deref())
                .hotkeys(&self.config.hotkeys)
                .into();
                (id, view.map(move |message| message.for_pane(id)))
            })
            .collect();
        let mut divider = to_iced(self.colors.foreground());
        divider.a = 0.3;
        let mut split = SplitContainer::new(
            tab.id,
            &tab.layout,
            panes,
            to_iced(self.colors.background()),
            divider,
        );
        // Focused pane border: the tab colour, else the accent once there is more than one pane.
        let border = tab
            .color
            .map(context_menu::to_iced)
            .or_else(|| (tab.panes.len() > 1).then(|| to_iced(self.colors.cursor())));
        if let Some(color) = border {
            split = split.border(tab.focused, color);
        }
        let bar = TabBar {
            tabs: &self.tabs,
            active: self.active,
            hovered: self.hovered_tab,
            renaming: self.renaming.as_ref(),
            colors: &self.colors,
        }
        .view();
        let base = overlay(column![bar, split].into(), self.paste_modal());
        context_menu::overlay(base, self.menu.as_ref(), self.window_size)
    }

    /// Every pane's session events plus window focus/scale/size changes and Escape.
    pub fn subscription(&self) -> Subscription<Message> {
        let sessions = self
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.values())
            .map(|pane| session_events(&pane.events));
        let window_events = event::listen_with(|event, _status, _window| match event {
            iced::Event::Window(window::Event::Focused) => Some(Message::Focused(true)),
            iced::Event::Window(window::Event::Unfocused) => Some(Message::Focused(false)),
            iced::Event::Window(window::Event::Rescaled(scale)) => {
                Some(Message::ScaleFactor(scale))
            }
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::Escape),
                ..
            }) => Some(Message::Escape),
            _ => None,
        });
        Subscription::batch(sessions.chain([window_events]))
    }
}
