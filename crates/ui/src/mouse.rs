//! Local mouse gestures of the terminal widget: selection (simple / word / line / block), wheel
//! scrollback and auto-scroll while drag-selecting past an edge. Everything stays local;
//! there is no mouse reporting yet.

use std::time::{Duration, Instant};

use fogu_term::{Scroll, SelectionType};
use iced::advanced::Shell;
use iced::advanced::mouse::{self, Button, Click, ScrollDelta, click};
use iced::keyboard::{self, Modifiers};
use iced::{Event, Point, Rectangle, window};

use crate::message::{Message, SelectEvent};
use crate::metrics::{CellMetrics, CellPos, pixel_to_cell};

/// Scrollback lines per wheel notch.
const LINES_PER_NOTCH: f32 = 3.0;
/// Auto-scroll pace while drag-selecting past the top/bottom edge.
const AUTOSCROLL_EVERY: Duration = Duration::from_millis(50);

/// Where the grid sits on screen.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    /// Widget bounds; the grid starts at the top-left corner.
    pub bounds: Rectangle,
    /// Cell size.
    pub metrics: CellMetrics,
    /// Columns.
    pub cols: usize,
    /// Visible rows.
    pub rows: usize,
}

impl Grid {
    fn cell_at(&self, p: Point) -> CellPos {
        pixel_to_cell(
            p - self.bounds.position(),
            &self.metrics,
            self.cols,
            self.rows,
        )
    }
}

/// A selection gesture in progress (left button held).
#[derive(Clone, Copy, Debug)]
struct Drag {
    kind: SelectionType,
    anchor: CellPos,
    /// The term has a selection for this gesture (a plain click starts none until it moves).
    started: bool,
    /// Last known pointer position, possibly outside the widget.
    last: Point,
}

/// Per-widget mouse state, kept in the widget tree across `view`s.
#[derive(Debug, Default)]
pub struct MouseState {
    modifiers: Modifiers,
    last_click: Option<Click>,
    drag: Option<Drag>,
    /// Fractional wheel lines not yet scrolled (trackpads send pixels).
    wheel_lines: f32,
    last_autoscroll: Option<Instant>,
}

impl MouseState {
    /// Handles one event; returns `true` when it was consumed.
    pub fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        grid: &Grid,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => self.modifiers = *m,
            Event::Mouse(mouse::Event::ButtonPressed(Button::Left)) => {
                if let Some(p) = cursor.position_over(grid.bounds) {
                    self.press(p, grid, shell);
                    return true;
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                if self.drag.is_some() {
                    self.motion(*position, grid, shell);
                    return true;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(Button::Left)) => {
                if let Some(drag) = self.drag.take() {
                    self.last_autoscroll = None;
                    if drag.started {
                        shell.publish(Message::Select(SelectEvent::Finish));
                    }
                    return true;
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(grid.bounds) => {
                self.wheel(*delta, grid, shell);
                return true;
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                self.autoscroll(*now, grid, shell);
            }
            _ => {}
        }
        false
    }

    fn press(&mut self, p: Point, grid: &Grid, shell: &mut Shell<'_, Message>) {
        let click = Click::new(p, Button::Left, self.last_click);
        self.last_click = Some(click);
        let at = grid.cell_at(p);
        let block = self.modifiers.alt() || self.modifiers.control();
        let kind = match click.kind() {
            click::Kind::Single if block => SelectionType::Block,
            click::Kind::Single => SelectionType::Simple,
            click::Kind::Double => SelectionType::Semantic,
            click::Kind::Triple => SelectionType::Lines,
        };
        // A plain click clears; the selection starts once the pointer leaves the anchor.
        let started = !matches!(kind, SelectionType::Simple | SelectionType::Block);
        let event = if started {
            SelectEvent::Start { kind, at }
        } else {
            SelectEvent::Clear
        };
        shell.publish(Message::Select(event));
        self.drag = Some(Drag {
            kind,
            anchor: at,
            started,
            last: p,
        });
    }

    fn motion(&mut self, p: Point, grid: &Grid, shell: &mut Shell<'_, Message>) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        drag.last = p;
        let at = grid.cell_at(p);
        if !drag.started {
            if at == drag.anchor {
                return;
            }
            drag.started = true;
            let (kind, at) = (drag.kind, drag.anchor);
            shell.publish(Message::Select(SelectEvent::Start { kind, at }));
        }
        shell.publish(Message::Select(SelectEvent::Update(at)));
        self.autoscroll(Instant::now(), grid, shell);
    }

    fn wheel(&mut self, delta: ScrollDelta, grid: &Grid, shell: &mut Shell<'_, Message>) {
        self.wheel_lines += match delta {
            ScrollDelta::Lines { y, .. } => y * LINES_PER_NOTCH,
            ScrollDelta::Pixels { y, .. } => y / grid.metrics.cell_h,
        };
        if !self.wheel_lines.is_finite() {
            self.wheel_lines = 0.0;
        }
        let whole = self.wheel_lines.trunc();
        self.wheel_lines -= whole;
        // Positive (wheel up) scrolls back into history.
        let lines = whole as i32;
        if lines != 0 {
            shell.publish(Message::Scroll(Scroll::Delta(lines)));
        }
    }

    /// While a started drag sits above/below the widget, scrolls one line per 50 ms and keeps
    /// the selection's free end on the edge row.
    fn autoscroll(&mut self, now: Instant, grid: &Grid, shell: &mut Shell<'_, Message>) {
        let Some(drag) = self.drag.filter(|d| d.started) else {
            return;
        };
        let b = grid.bounds;
        let dir = if drag.last.y < b.y {
            1
        } else if drag.last.y >= b.y + b.height {
            -1
        } else {
            self.last_autoscroll = None;
            return;
        };
        let due = self
            .last_autoscroll
            .is_none_or(|t| now.saturating_duration_since(t) >= AUTOSCROLL_EVERY);
        if due {
            shell.publish(Message::Scroll(Scroll::Delta(dir)));
            shell.publish(Message::Select(SelectEvent::Update(
                grid.cell_at(drag.last),
            )));
            self.last_autoscroll = Some(now);
        }
        let last = self.last_autoscroll.unwrap_or(now);
        shell.request_redraw_at(last + AUTOSCROLL_EVERY);
    }
}
