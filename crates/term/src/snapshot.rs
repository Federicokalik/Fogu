//! `RenderSnapshot`: an owned copy of the visible rows, taken under a short lock and drawn
//! without holding the `Term` lock.

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, CursorShape};

/// Cell attribute flags (bold/dim/italic/inverse/hidden/strikeout, underline kinds, wide-char and
/// spacer flags); alacritty's own bitflags.
pub type CellFlags = Flags;

/// One visible cell. Colors keep alacritty's `Color` semantics (named/indexed/spec); palette
/// resolution happens in the UI with the active scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Base character (a space for empty cells and wide-char spacers).
    pub ch: char,
    /// Combining characters attached to `ch`.
    pub zerowidth: Option<Box<[char]>>,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Underline color (SGR 58); `None` means "use `fg`".
    pub underline_color: Option<Color>,
    /// Attribute flags.
    pub flags: CellFlags,
}

/// Cursor position (viewport coordinates) and look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Column, 0-based.
    pub col: usize,
    /// Viewport row, 0-based.
    pub row: usize,
    /// Shape requested by the app (DECSCUSR) or the default.
    pub shape: CursorShape,
    /// False when hidden (DECTCEM off) or scrolled out of the viewport.
    pub visible: bool,
}

/// Everything the UI needs to draw one frame of a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderSnapshot {
    /// Number of columns.
    pub cols: usize,
    /// Number of visible rows.
    pub rows: usize,
    /// `rows * cols` cells, row-major, top row first.
    pub cells: Vec<Cell>,
    /// Cursor state.
    pub cursor: Cursor,
    /// How many lines the viewport is scrolled back into history (0 = bottom).
    pub display_offset: usize,
    /// Lines currently stored in scrollback history.
    pub history_len: usize,
    /// Active selection in grid coordinates.
    pub selection: Option<SelectionRange>,
    /// Terminal modes at snapshot time.
    pub mode: TermMode,
}

impl RenderSnapshot {
    /// Copies the visible rows of `term`. Call with the lock held, then drop the lock.
    pub(crate) fn capture<T: EventListener>(term: &Term<T>) -> Self {
        let content = term.renderable_content();
        let cols = term.columns();
        let rows = term.screen_lines();
        let display_offset = content.display_offset;

        let cells = content
            .display_iter
            .map(|indexed| {
                let cell = indexed.cell;
                Cell {
                    ch: cell.c,
                    zerowidth: cell.zerowidth().map(Box::from),
                    fg: cell.fg,
                    bg: cell.bg,
                    underline_color: cell.underline_color(),
                    flags: cell.flags,
                }
            })
            .collect();

        let point = content.cursor.point;
        // Grid line 0 is the top of the live screen; the viewport starts `display_offset` above.
        let row = usize::try_from(point.line.0)
            .ok()
            .map(|line| line + display_offset)
            .filter(|r| *r < rows);
        let cursor = Cursor {
            col: point.column.0,
            row: row.unwrap_or(0),
            shape: content.cursor.shape,
            visible: row.is_some() && content.cursor.shape != CursorShape::Hidden,
        };

        Self {
            cols,
            rows,
            cells,
            cursor,
            display_offset,
            history_len: term.grid().history_size(),
            selection: content.selection,
            mode: content.mode,
        }
    }

    /// Iterates over the visible rows.
    pub fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        self.cells.chunks(self.cols.max(1))
    }

    /// Visible rows as plain text, one line per row, trailing spaces trimmed. Wide-char spacers
    /// are skipped so a CJK char reads as one char.
    pub fn to_text(&self) -> String {
        let spacers = CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER;
        let mut out = String::new();
        for row in self.rows() {
            let mut line = String::new();
            for cell in row.iter().filter(|c| !c.flags.intersects(spacers)) {
                line.push(cell.ch);
                line.extend(cell.zerowidth.iter().flat_map(|z| z.iter()));
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}
