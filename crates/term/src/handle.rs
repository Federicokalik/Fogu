//! `TermHandle`: sync terminal core. Feed bytes in, take snapshots and drained events out.
//! No async, no PTY: the read loop lives in `fogu-session`.

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::Term;
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::Processor;

use crate::event_proxy::{EventProxy, Palette, Shared, TermEvent, default_palette, lock};
use crate::snapshot::RenderSnapshot;

/// Maximum bytes parsed per `Term` lock, so the renderer can always grab it within a frame.
const MAX_BYTES_PER_LOCK: usize = 64 * 1024;

/// Grid size in cells.
#[derive(Clone, Copy, Debug)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// One terminal: `alacritty_terminal::Term` behind a `FairMutex`, plus the VT parser that the
/// feeding side (session task) owns.
pub struct TermHandle {
    term: Arc<FairMutex<Term<EventProxy>>>,
    processor: Processor,
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
}

impl fmt::Debug for TermHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = lock(&self.shared).window_size;
        f.debug_struct("TermHandle")
            .field("cols", &size.num_cols)
            .field("rows", &size.num_lines)
            .finish_non_exhaustive()
    }
}

impl TermHandle {
    /// Creates a terminal of `cols` x `rows` cells (each at least 1) keeping `scrollback_lines`
    /// lines of history.
    pub fn new(cols: u16, rows: u16, scrollback_lines: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        let shared = Arc::new(Mutex::new(Shared {
            events: Vec::new(),
            palette: default_palette(),
            window_size: WindowSize {
                num_lines: rows,
                num_cols: cols,
                cell_width: 0,
                cell_height: 0,
            },
        }));
        let dirty = Arc::new(AtomicBool::new(false));
        let proxy = EventProxy::new(Arc::clone(&shared), Arc::clone(&dirty));
        // OSC 52 read is also disabled here (`OnlyCopy`); the proxy drops it regardless.
        let config = Config {
            scrolling_history: scrollback_lines,
            ..Config::default()
        };
        let size = Size {
            cols: cols.into(),
            rows: rows.into(),
        };
        let term = Term::new(config, &size, proxy);
        Self {
            term: Arc::new(FairMutex::new(term)),
            processor: Processor::new(),
            shared,
            dirty,
        }
    }

    /// Parses `bytes`, locking the `Term` once per chunk of at most 64 KiB. `feed(&[])` flushes a
    /// synchronized update (DEC 2026) whose [`sync_timeout`](Self::sync_timeout) has passed.
    pub fn feed(&mut self, bytes: &[u8]) {
        if self
            .processor
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|t| t <= Instant::now())
        {
            self.processor.stop_sync(&mut *self.term.lock());
        }
        for chunk in bytes.chunks(MAX_BYTES_PER_LOCK) {
            self.processor.advance(&mut *self.term.lock(), chunk);
        }
    }

    /// Deadline of a pending synchronized update; call `feed(&[])` once it passes.
    pub fn sync_timeout(&self) -> Option<Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    /// Resizes (and reflows) the grid to `cols` x `rows` cells, each at least 1.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        self.term.lock().resize(Size {
            cols: cols.into(),
            rows: rows.into(),
        });
        let mut shared = lock(&self.shared);
        shared.window_size.num_cols = cols;
        shared.window_size.num_lines = rows;
    }

    /// Sets the cell size in pixels, used to answer `CSI 14 t` text-area size queries.
    pub fn set_cell_size_px(&mut self, width: u16, height: u16) {
        let mut shared = lock(&self.shared);
        shared.window_size.cell_width = width;
        shared.window_size.cell_height = height;
    }

    /// Replaces the palette used to answer OSC 4/10/11/12 color queries.
    pub fn set_palette(&mut self, palette: Box<Palette>) {
        lock(&self.shared).palette = palette;
    }

    /// Scrolls the viewport through history.
    pub fn scroll_display(&mut self, scroll: Scroll) {
        self.term.lock().scroll_display(scroll);
    }

    /// Copies the visible rows under a short lock.
    pub fn snapshot(&self) -> RenderSnapshot {
        RenderSnapshot::capture(&self.term.lock())
    }

    /// Takes all events produced since the last call.
    pub fn drain_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut lock(&self.shared).events)
    }

    /// Shared "needs redraw" flag used to coalesce wakeups between session and UI.
    pub fn dirty(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.dirty)
    }

    /// Current terminal modes (for the input encoder).
    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Visible rows as text, trailing spaces trimmed (test/debug helper).
    pub fn dump_grid(&self) -> String {
        self.snapshot().to_text()
    }
}
