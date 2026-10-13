//! Application state, `update`, `view` and subscriptions: one pane running the default shell.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fogu_core::{ColorScheme, LocalProfile, Size};
use fogu_session::{SessionInput, SessionOutput, SessionRuntime};
use fogu_term::{RenderSnapshot, TermHandle};
use iced::{Element, Subscription, Task, event, window};
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::message::Message;
use crate::metrics::{self, CellMetrics, FontSpec, ResolvedFont};
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
            Message::Focused(focused) => self.focused = focused,
        }
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
            // Bell, OSC 52, cwd and progress are not handled yet.
            SessionOutput::Bell
            | SessionOutput::ClipboardStore(_)
            | SessionOutput::Cwd(_)
            | SessionOutput::Progress(_) => {}
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
        let overlay = match self.pane.state {
            PaneState::Running => None,
            PaneState::Exited => Some("[process exited]"),
            PaneState::Crashed => Some("[crashed — see log]"),
        };
        TerminalView::new(
            &self.pane.snapshot,
            &self.colors,
            self.metrics,
            self.resolved.font,
            self.focused,
            overlay,
        )
        .into()
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
