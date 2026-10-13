//! Application state, `update` dispatch, `view` and subscriptions. Tab lifecycle and actions live
//! in `actions.rs`; pane-level input (keys, paste, selection, resize) here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fogu_core::{ColorScheme, Config, Size};
use fogu_session::{SessionInput, SessionRuntime};
use fogu_term::{TermMode, input};
use iced::keyboard::{self, key::Named};
use iced::widget::{column, container};
use iced::{Element, Length, Point, Subscription, Task, clipboard, event, window};

use crate::context_menu::{self, ContextMenu};
use crate::message::{Message, PasteSource, SelectEvent};
use crate::metrics::{self, CellMetrics, FontSpec, ResolvedFont, viewport_to_point};
use crate::modal::{Modal, ModalButton, overlay};
use crate::mouse::MouseConfig;
use crate::palette::Colors;
use crate::subscription::session_events;
use crate::tab::{PaneId, PaneState, Tab, TabId};
use crate::tab_bar::{Rename, TabBar};
use crate::terminal_view::TerminalView;

/// Transport resize debounce during window drags.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);
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
    /// `NewTabHere` asked pane `.0` for its cwd at `.1`.
    pub(crate) pending_here: Option<(PaneId, Instant)>,
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
    pub(crate) stub_inputs: Vec<tokio::sync::mpsc::Receiver<SessionInput>>,
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
            pending_here: None,
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
            Message::GridResized { cols, rows } => self.on_grid_resized(cols, rows),
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
            Message::Focused(focused) => self.on_focused(focused),
            Message::Key(_)
            | Message::Text(_)
            | Message::MouseInput(_)
            | Message::Select(_)
            | Message::Scroll(_)
            | Message::Paste(_)
            | Message::PasteReady(_)
            | Message::ConfirmPaste
            | Message::CancelPaste => self.on_pane_input(message),
            other => self.on_tab_ui(other),
        }
    }

    /// Keys, mouse and paste for the active pane.
    fn on_pane_input(&mut self, message: Message) -> Task<Message> {
        let alt_is_meta = self.config.terminal.alt_is_meta;
        let Some(pane) = self.active_pane_mut() else {
            return Task::none();
        };
        match message {
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
            Message::Select(event) => return self.on_select(event),
            Message::Scroll(scroll) => {
                pane.term.scroll_display(scroll);
                pane.refresh();
            }
            Message::Paste(source) => {
                return match source {
                    PasteSource::Clipboard => clipboard::read(),
                    PasteSource::Primary => clipboard::read_primary(),
                }
                .map(Message::PasteReady);
            }
            Message::PasteReady(text) => self.on_paste_ready(text),
            Message::ConfirmPaste => {
                if let Some(pending) = self.pending_paste.take() {
                    self.paste_now(&pending.text);
                }
            }
            Message::CancelPaste => self.pending_paste = None,
            _ => {}
        }
        Task::none()
    }

    /// Window focus, or a click into the pane (which also commits an inline rename).
    fn on_focused(&mut self, focused: bool) -> Task<Message> {
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
        Task::none()
    }

    /// A clipboard read for [`Message::Paste`] completed: confirm multi-line/large pastes,
    /// otherwise send it straight away. `None`/empty means nothing to paste.
    fn on_paste_ready(&mut self, text: Option<String>) {
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
            Some(warning) => self.pending_paste = Some(PendingPaste { text, warning }),
            None => self.paste_now(&text),
        }
    }

    /// Sanitises and sends `text` to the session. `terminal.bracketed_paste = false` forces
    /// plain paste even if the application asked for it.
    fn paste_now(&mut self, text: &str) {
        let terminal = &self.config.terminal;
        let (bracketed, trim, newlines) = (
            terminal.bracketed_paste,
            terminal.trim_whitespace_on_paste,
            terminal.replace_newlines_on_paste,
        );
        let Some(pane) = self.active_pane_mut() else {
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
        for tab in &mut self.tabs {
            let m = &self.metrics;
            tab.pane.term.set_cell_size_px(m.cell_w_px, m.cell_h_px);
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
    fn on_select(&mut self, event: SelectEvent) -> Task<Message> {
        let copy_on_select = self.config.terminal.copy_on_select;
        let Some(pane) = self.active_pane_mut() else {
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

    /// Resizes the active grid now (correct rendering) and its PTY after a 50 ms quiet period.
    fn on_grid_resized(&mut self, cols: u16, rows: u16) -> Task<Message> {
        let (cell_w, cell_h) = (self.metrics.cell_w_px, self.metrics.cell_h_px);
        let Some(pane) = self.active_pane_mut() else {
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
        let message = Message::ResizeTimer(pane.id, pane.resize_generation);
        self.after(RESIZE_DEBOUNCE, message)
    }

    fn on_resize_timer(&mut self, pane_id: PaneId, generation: u64) -> Task<Message> {
        let Some(pane) = self.pane_mut(pane_id) else {
            return Task::none();
        };
        if generation != pane.resize_generation {
            return Task::none();
        }
        let Some(size) = pane.pending_resize.take() else {
            return Task::none();
        };
        if pane.control(SessionInput::Resize(size)) || pane.state != PaneState::Running {
            return Task::none();
        }
        // The session is busy; retry after another debounce period.
        tracing::debug!("PTY resize deferred");
        pane.pending_resize = Some(size);
        pane.resize_generation += 1;
        let message = Message::ResizeTimer(pane.id, pane.resize_generation);
        self.after(RESIZE_DEBOUNCE, message)
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

    /// Tab bar above the active pane (bordered in the tab colour), plus modal / menu overlays.
    pub fn view(&self) -> Element<'_, Message> {
        let Some(tab) = self.active_tab() else {
            return column![].into();
        };
        let status = match tab.pane.state {
            PaneState::Running => None,
            PaneState::Exited => Some("[process exited]"),
            PaneState::Crashed => Some("[crashed — see log]"),
        };
        let typing_elsewhere = self.renaming.is_some() || self.menu.is_some();
        let term_view: Element<'_, Message> = TerminalView::new(
            &tab.pane.snapshot,
            &self.colors,
            self.metrics,
            self.resolved.font,
            self.focused && !typing_elsewhere,
            status,
            MouseConfig {
                wheel_lines: self.config.terminal.wheel_lines,
                paste_on_middle_click: self.config.terminal.paste_on_middle_click,
            },
        )
        .hotkeys(&self.config.hotkeys)
        .into();
        let border = tab.color.map(context_menu::to_iced);
        let pane = container(term_view)
            .padding(u16::from(border.is_some()))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme: &iced::Theme| container::Style {
                background: border.map(Into::into),
                ..container::Style::default()
            });
        let bar = TabBar {
            tabs: &self.tabs,
            active: self.active,
            hovered: self.hovered_tab,
            renaming: self.renaming.as_ref(),
            colors: &self.colors,
        }
        .view();
        let base = overlay(column![bar, pane].into(), self.paste_modal());
        context_menu::overlay(base, self.menu.as_ref(), self.window_size)
    }

    /// Every pane's session events plus window focus/scale/size changes and Escape.
    pub fn subscription(&self) -> Subscription<Message> {
        let sessions = self.tabs.iter().map(|tab| session_events(&tab.pane.events));
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
