//! Application state, `update`, `view` and subscriptions: one pane running the default shell.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fogu_core::{ColorScheme, LocalProfile, Size};
use fogu_session::{SessionInput, SessionOutput, SessionRuntime};
use fogu_term::{RenderSnapshot, Scroll, TermHandle, TermMode, input};
use iced::{Element, Subscription, Task, clipboard, event, window};
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::message::{Message, PasteSource, SelectEvent};
use crate::metrics::{self, CellMetrics, FontSpec, ResolvedFont, viewport_to_point};
use crate::modal::{Modal, ModalButton, overlay};
use crate::palette::Colors;
use crate::subscription::{EventSlot, session_events};
use crate::terminal_view::TerminalView;

/// Initial grid before the first layout, and scrollback length (fixed for now).
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;
const SCROLLBACK_LINES: usize = 10_000;
/// Transport resize debounce during window drags.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);
const DEFAULT_TITLE: &str = "Fogu";
/// Alt sends an ESC prefix ("Alt is Meta"); fixed for now.
const ALT_IS_META: bool = true;
/// Finishing a mouse selection copies it to PRIMARY.
const COPY_ON_SELECT: bool = true;
/// Ask before pasting text that spans several lines.
const WARN_ON_MULTILINE_PASTE: bool = true;
/// Trim whitespace around pasted text.
const TRIM_WHITESPACE_ON_PASTE: bool = true;
/// Replace newlines in pasted text with spaces.
const REPLACE_NEWLINES_WITH_SPACES_ON_PASTE: bool = false;
/// Smallest font size zoom allows, in points.
const MIN_FONT_PT: f32 = 6.0;
/// Largest font size zoom allows, in points.
const MAX_FONT_PT: f32 = 40.0;

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
    term: TermHandle,
    dirty: Arc<AtomicBool>,
    session_input: Sender<SessionInput>,
    events: EventSlot,
    state: PaneState,
    snapshot: RenderSnapshot,
}

/// A multi-line/large paste waiting on the user's "Paste"/"Cancel" choice in the modal.
#[derive(Debug)]
struct PendingPaste {
    text: String,
    warning: input::PasteWarning,
}

/// The whole application state.
#[derive(Debug)]
pub struct App {
    runtime: Arc<SessionRuntime>,
    pane: Pane,
    font: FontSpec,
    resolved: ResolvedFont,
    metrics: CellMetrics,
    scale: f32,
    colors: Colors,
    title: String,
    focused: bool,
    resize_generation: u64,
    pending_resize: Option<Size>,
    pending_paste: Option<PendingPaste>,
}

impl App {
    /// Opens the default shell in one pane and asks for the window's scale factor.
    pub fn boot(runtime: Arc<SessionRuntime>) -> (Self, Task<Message>) {
        let font = FontSpec::default();
        let resolved = metrics::resolve(&font.family);
        let metrics = CellMetrics::new(resolved.units, font.size_px(), 1.0);
        let colors = Colors::new(&ColorScheme::default());

        let mut term = TermHandle::new(INITIAL_COLS, INITIAL_ROWS, SCROLLBACK_LINES);
        term.set_palette(colors.palette());
        term.set_cell_size_px(metrics.cell_w_px, metrics.cell_h_px);
        let size = Size {
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
            ..Size::default()
        };
        let session_term = term.share();
        // The profile is fixed for now.
        let handle = fogu_session::open(
            runtime.handle(),
            &LocalProfile::default(),
            size,
            session_term,
        );
        let pane = Pane {
            dirty: term.dirty(),
            snapshot: term.snapshot(),
            term,
            session_input: handle.input,
            events: EventSlot::new(0, handle.events),
            state: PaneState::Running,
        };
        let app = Self {
            runtime,
            pane,
            font,
            resolved,
            metrics,
            scale: 1.0,
            colors,
            title: DEFAULT_TITLE.to_owned(),
            focused: true,
            resize_generation: 0,
            pending_resize: None,
            pending_paste: None,
        };
        let scale = window::oldest()
            .and_then(window::scale_factor)
            .map(Message::ScaleFactor);
        (app, scale)
    }

    /// Window title (the shell's OSC title, or "Fogu").
    pub fn title(&self) -> String {
        self.title.clone()
    }

    /// Handles one message.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Session(event) => self.on_session(event),
            Message::GridResized { cols, rows } => return self.on_grid_resized(cols, rows),
            Message::ResizeTimer(generation) => return self.on_resize_timer(generation),
            Message::ScaleFactor(scale) => {
                self.scale = scale;
                self.metrics = CellMetrics::new(self.resolved.units, self.font.size_px(), scale);
                self.pane
                    .term
                    .set_cell_size_px(self.metrics.cell_w_px, self.metrics.cell_h_px);
            }
            Message::Focused(focused) => {
                if focused != self.focused
                    && let Some(bytes) = input::encode_focus(focused, self.pane.term.mode())
                {
                    self.write_input(bytes);
                }
                self.focused = focused;
            }
            Message::Key(key) => {
                let mode = self.pane.term.mode();
                if let Some(bytes) = input::encode_key(&key, mode, ALT_IS_META) {
                    self.send_input(bytes);
                }
            }
            Message::Text(text) => {
                if let Some(bytes) = input::encode_text(&text) {
                    self.send_input(bytes);
                }
            }
            Message::MouseInput(bytes) => self.write_input(bytes),
            Message::Select(event) => return self.on_select(event),
            Message::Scroll(scroll) => {
                self.pane.term.scroll_display(scroll);
                self.refresh();
            }
            Message::Copy => {
                if let Some(text) = self.pane.term.selection_text() {
                    return clipboard::write(text);
                }
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
            Message::ZoomIn => self.set_font_size(self.font.size_pt + 1.0),
            Message::ZoomOut => self.set_font_size(self.font.size_pt - 1.0),
            Message::ZoomReset => self.set_font_size(FontSpec::default().size_pt),
            Message::Clear => {
                self.pane.term.clear();
                self.refresh();
            }
        }
        Task::none()
    }

    /// A clipboard read for [`Message::Paste`] completed: confirm multi-line/large pastes,
    /// otherwise send it straight away. `None`/empty means nothing to paste.
    fn on_paste_ready(&mut self, text: Option<String>) {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return;
        };
        let warning = WARN_ON_MULTILINE_PASTE
            .then(|| input::needs_confirmation(&text))
            .flatten();
        match warning {
            Some(warning) => self.pending_paste = Some(PendingPaste { text, warning }),
            None => self.paste_now(&text),
        }
    }

    /// Sanitises and sends `text` to the session..
    fn paste_now(&mut self, text: &str) {
        let opts = input::PasteOpts {
            bracketed_requested: self.pane.term.mode().contains(TermMode::BRACKETED_PASTE),
            trim_whitespace: TRIM_WHITESPACE_ON_PASTE,
            newlines_to_spaces: REPLACE_NEWLINES_WITH_SPACES_ON_PASTE,
        };
        self.send_input(input::encode_paste(text, opts));
    }

    /// Clamps to [`MIN_FONT_PT`]..=[`MAX_FONT_PT`] and recomputes cell metrics; the next frame's
    /// `GridResized` picks up the new grid size and resizes the
    /// PTY, same as a window resize.
    fn set_font_size(&mut self, size_pt: f32) {
        self.font.size_pt = size_pt.clamp(MIN_FONT_PT, MAX_FONT_PT);
        self.metrics = CellMetrics::new(self.resolved.units, self.font.size_px(), self.scale);
        self.pane
            .term
            .set_cell_size_px(self.metrics.cell_w_px, self.metrics.cell_h_px);
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

    /// Sends encoded keys to the session without ever blocking the UI, then scrolls to the
    /// bottom.
    fn send_input(&mut self, bytes: Vec<u8>) {
        if self.pane.state != PaneState::Running {
            return;
        }
        self.write_input(bytes);
        if self.pane.snapshot.display_offset != 0 {
            self.pane.term.scroll_display(Scroll::Bottom);
            self.refresh();
        }
    }

    /// Writes `bytes` to a running session without blocking the UI (drops them when full).
    fn write_input(&mut self, bytes: Vec<u8>) {
        if self.pane.state != PaneState::Running {
            return;
        }
        match self
            .pane
            .session_input
            .try_send(SessionInput::Data(bytes.into()))
        {
            Ok(()) | Err(TrySendError::Closed(_)) => {}
            Err(TrySendError::Full(_)) => tracing::warn!("session input full; input dropped"),
        }
    }

    /// Applies a selection gesture; viewport cells map to grid lines with the current scroll.
    fn on_select(&mut self, event: SelectEvent) -> Task<Message> {
        let offset = self.pane.snapshot.display_offset;
        let term = &mut self.pane.term;
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
                    Some(text) if COPY_ON_SELECT => clipboard::write_primary(text),
                    _ => Task::none(),
                };
            }
        }
        self.refresh();
        Task::none()
    }

    fn on_session(&mut self, event: SessionOutput) {
        match event {
            SessionOutput::Wakeup => self.refresh(),
            SessionOutput::Title(title) => {
                self.title = if title.is_empty() {
                    DEFAULT_TITLE.to_owned()
                } else {
                    title
                };
            }
            SessionOutput::Exited(info) => {
                tracing::info!(?info, "shell exited");
                self.pane.state = PaneState::Exited;
                self.refresh();
            }
            SessionOutput::Error(err) => {
                tracing::error!(error = %err, "session failed");
                self.pane.state = PaneState::Crashed;
                self.refresh();
            }
            SessionOutput::Prompt(_, kind) => match kind {},
            // Not handled yet: bell and OSC 52. Cwd, progress, notifications and prompt marks
            // are logged by the session and ignored here.
            SessionOutput::Bell
            | SessionOutput::ClipboardStore(_)
            | SessionOutput::Cwd { .. }
            | SessionOutput::Progress(_)
            | SessionOutput::Notify { .. }
            | SessionOutput::PromptMark(_) => {}
        }
    }

    /// Clears `dirty` *before* snapshotting, so output arriving meanwhile sends a new wakeup.
    fn refresh(&mut self) {
        self.pane.dirty.store(false, Ordering::SeqCst);
        self.pane.snapshot = self.pane.term.snapshot();
    }

    /// Resizes the grid now (correct rendering) and the PTY after a 50 ms quiet period.
    fn on_grid_resized(&mut self, cols: u16, rows: u16) -> Task<Message> {
        self.pane.term.resize(cols, rows);
        self.refresh();
        let m = &self.metrics;
        self.pending_resize = Some(Size {
            cols,
            rows,
            px_w: cols.saturating_mul(m.cell_w_px),
            px_h: rows.saturating_mul(m.cell_h_px),
        });
        self.resize_timer()
    }

    /// Starts a new debounce period; older timers become stale.
    fn resize_timer(&mut self) -> Task<Message> {
        self.resize_generation += 1;
        let generation = self.resize_generation;
        let timer = self.runtime.handle().spawn(async {
            tokio::time::sleep(RESIZE_DEBOUNCE).await;
        });
        Task::perform(timer, move |_| Message::ResizeTimer(generation))
    }

    fn on_resize_timer(&mut self, generation: u64) -> Task<Message> {
        if generation != self.resize_generation {
            return Task::none();
        }
        let Some(size) = self.pending_resize.take() else {
            return Task::none();
        };
        match self.pane.session_input.try_send(SessionInput::Resize(size)) {
            Ok(()) | Err(TrySendError::Closed(_)) => Task::none(),
            Err(TrySendError::Full(_)) => {
                // The session is busy; retry after another debounce period.
                tracing::debug!("session input full; PTY resize deferred");
                self.pending_resize = Some(size);
                self.resize_timer()
            }
        }
    }

    /// The single pane.
    pub fn view(&self) -> Element<'_, Message> {
        let status = match self.pane.state {
            PaneState::Running => None,
            PaneState::Exited => Some("[process exited]"),
            PaneState::Crashed => Some("[crashed — see log]"),
        };
        let term_view = TerminalView::new(
            &self.pane.snapshot,
            &self.colors,
            self.metrics,
            self.resolved.font,
            self.focused,
            status,
        )
        .into();
        overlay(term_view, self.paste_modal())
    }

    /// Session events plus window focus/scale changes.
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            session_events(&self.pane.events),
            event::listen_with(|event, _status, _window| match event {
                iced::Event::Window(window::Event::Focused) => Some(Message::Focused(true)),
                iced::Event::Window(window::Event::Unfocused) => Some(Message::Focused(false)),
                iced::Event::Window(window::Event::Rescaled(scale)) => {
                    Some(Message::ScaleFactor(scale))
                }
                _ => None,
            }),
        ])
    }
}
