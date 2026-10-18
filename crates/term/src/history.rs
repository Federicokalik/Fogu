//! Scrollback capture/restore as inert style runs for tab recovery: the primary screen's last lines
//! become rows of `{text, fg, bg, flags}` runs, and a restore feeds back only text and SGR
//! sequences built here from those enumerated attributes, never the original escape sequences.
//! Files are handled by the caller.

use alacritty_terminal::Term;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::Color;
use serde::{Deserialize, Serialize};

/// Format version written by [`capture`].
pub const VERSION: u32 = 1;
/// Default cap on the restored text, in bytes.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// Style bits of a [`Run`] (our own numbering, stable on disk).
pub mod flag {
    /// SGR 1.
    pub const BOLD: u16 = 1;
    /// SGR 2.
    pub const DIM: u16 = 1 << 1;
    /// SGR 3.
    pub const ITALIC: u16 = 1 << 2;
    /// SGR 4 (any underline style).
    pub const UNDERLINE: u16 = 1 << 3;
    /// SGR 7.
    pub const INVERSE: u16 = 1 << 4;
    /// SGR 8.
    pub const HIDDEN: u16 = 1 << 5;
    /// SGR 9.
    pub const STRIKE: u16 = 1 << 6;
    /// Every known bit.
    pub const ALL: u16 = (1 << 7) - 1;
}

/// A terminal's saved history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    /// [`VERSION`].
    pub version: u32,
    /// Width the rows were captured at.
    pub cols: u16,
    /// Unix seconds.
    pub captured_at: u64,
    /// Oldest first.
    pub rows: Vec<SavedRow>,
}

/// One grid row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRow {
    /// Consecutive cells of one style.
    pub runs: Vec<Run>,
    /// The row continues on the next one (soft wrap).
    pub wrapped: bool,
}

/// Text of one style.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Printable text (wide chars once, combining marks kept).
    pub text: String,
    /// Foreground.
    pub fg: SavedColor,
    /// Background.
    pub bg: SavedColor,
    /// [`flag`] bits.
    pub flags: u16,
}

/// A cell colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedColor {
    /// The scheme's default foreground / background.
    #[default]
    Default,
    /// One of the 16 named colours (0..=15).
    Named(u8),
    /// 256-colour palette index.
    Indexed(u8),
    /// True colour.
    Rgb(u8, u8, u8),
}

/// Why a saved history is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    /// Unknown format version.
    #[error("unsupported history version {0}")]
    Version(u32),
    /// More rows than allowed.
    #[error("too many rows ({0})")]
    TooManyRows(usize),
    /// More text than allowed.
    #[error("too much text ({0} bytes)")]
    TooLarge(usize),
    /// Width outside 2..=1000.
    #[error("bad width {0}")]
    Cols(u16),
    /// Unknown style bits or a named colour above 15.
    #[error("bad style")]
    Style,
    /// Control characters in the text.
    #[error("control characters in the text")]
    Control,
}

/// Checks a loaded history before [`TermHandle::restore_history`](crate::TermHandle::restore_history): version, size bounds, width, known style bits, no
/// control characters.
pub fn validate(
    saved: &SavedHistory,
    max_lines: usize,
    max_bytes: usize,
) -> Result<(), HistoryError> {
    if saved.version != VERSION {
        return Err(HistoryError::Version(saved.version));
    }
    if saved.rows.len() > max_lines {
        return Err(HistoryError::TooManyRows(saved.rows.len()));
    }
    if !(2..=1000).contains(&saved.cols) {
        return Err(HistoryError::Cols(saved.cols));
    }
    let runs = || saved.rows.iter().flat_map(|r| &r.runs);
    let bytes: usize = runs().map(|r| r.text.len()).sum();
    if bytes > max_bytes {
        return Err(HistoryError::TooLarge(bytes));
    }
    let named_ok = |c: SavedColor| !matches!(c, SavedColor::Named(n) if n > 15);
    if runs().any(|r| r.flags & !flag::ALL != 0 || !named_ok(r.fg) || !named_ok(r.bg)) {
        return Err(HistoryError::Style);
    }
    if runs().any(|r| r.text.chars().any(is_control)) {
        return Err(HistoryError::Control);
    }
    Ok(())
}

/// The last `max_lines` rows of `term`'s primary screen and history (the primary screen even
/// while the alternate one is shown), trailing empty rows left out.
pub(crate) fn capture<T: EventListener>(
    term: &mut Term<T>,
    max_lines: usize,
    now: u64,
) -> SavedHistory {
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    // The inactive grid is private: swap to the primary screen and back, then put the alternate
    // screen's content back (swapping into it clears it). The primary's DECSC cursor becomes
    // its current cursor, which is where 1049 left it anyway.
    let alt_grid = alt.then(|| term.grid().clone());
    // `swap_alt` also drops the selection: keep the user's across the round trip.
    let selection = term.selection.take();
    if alt {
        term.swap_alt();
    }
    let rows = capture_rows(term, max_lines);
    if let Some(grid) = alt_grid {
        term.swap_alt();
        *term.grid_mut() = grid;
    }
    term.selection = selection;
    SavedHistory {
        version: VERSION,
        cols: u16::try_from(term.columns()).unwrap_or(u16::MAX),
        captured_at: now,
        rows,
    }
}

fn capture_rows<T: EventListener>(term: &Term<T>, max_lines: usize) -> Vec<SavedRow> {
    let grid = term.grid();
    let cols = grid.columns();
    let bottom = i32::try_from(grid.screen_lines()).unwrap_or(i32::MAX);
    let mut rows: Vec<SavedRow> = (grid.topmost_line().0..bottom)
        .map(|line| {
            let row = &grid[Line(line)];
            let wrapped = cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
            let cells = (0..cols).map(|c| &row[Column(c)]);
            SavedRow {
                runs: runs(cells, wrapped),
                wrapped,
            }
        })
        .collect();
    while rows.last().is_some_and(|r| r.runs.is_empty() && !r.wrapped) {
        rows.pop();
    }
    let skip = rows.len().saturating_sub(max_lines);
    rows.drain(..skip);
    rows
}

/// A row's cells grouped by style; trailing default-style blanks dropped unless the row wraps.
fn runs<'a>(cells: impl Iterator<Item = &'a Cell>, wrapped: bool) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for cell in cells {
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let (fg, bg, flags) = (saved(cell.fg), saved(cell.bg), saved_flags(cell.flags));
        let mut text = String::from(if is_control(cell.c) { ' ' } else { cell.c });
        text.extend(cell.zerowidth().unwrap_or_default());
        match out.last_mut() {
            Some(run) if run.fg == fg && run.bg == bg && run.flags == flags => {
                run.text.push_str(&text);
            }
            _ => out.push(Run {
                text,
                fg,
                bg,
                flags,
            }),
        }
    }
    if !wrapped {
        while let Some(run) = out.last_mut() {
            if run.bg != SavedColor::Default
                || run.flags & (flag::INVERSE | flag::UNDERLINE | flag::STRIKE) != 0
            {
                break;
            }
            let kept = run.text.trim_end_matches(' ').len();
            run.text.truncate(kept);
            if !run.text.is_empty() {
                break;
            }
            out.pop();
        }
    }
    out
}

fn saved(color: Color) -> SavedColor {
    match color {
        // The 16 ANSI colours are discriminants 0..=15; Foreground/Background/… are the defaults.
        Color::Named(named) => u8::try_from(named as usize)
            .ok()
            .filter(|n| *n <= 15)
            .map_or(SavedColor::Default, SavedColor::Named),
        Color::Indexed(i) => SavedColor::Indexed(i),
        Color::Spec(rgb) => SavedColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn saved_flags(flags: Flags) -> u16 {
    [
        (Flags::BOLD, flag::BOLD),
        (Flags::DIM, flag::DIM),
        (Flags::ITALIC, flag::ITALIC),
        (Flags::ALL_UNDERLINES, flag::UNDERLINE),
        (Flags::INVERSE, flag::INVERSE),
        (Flags::HIDDEN, flag::HIDDEN),
        (Flags::STRIKEOUT, flag::STRIKE),
    ]
    .into_iter()
    .filter(|(f, _)| flags.intersects(*f))
    .fold(0, |acc, (_, bit)| acc | bit)
}

/// C0, DEL and C1 controls.
fn is_control(c: char) -> bool {
    c < ' ' || ('\u{7f}'..='\u{9f}').contains(&c)
}

/// The bytes that redraw `saved`: per run an SGR built from its attributes plus its text with any
/// control character replaced by U+FFFD; `\r\n` after every row that does not wrap. Nothing else.
pub(crate) fn generate(saved: &SavedHistory) -> Vec<u8> {
    let mut out = Vec::new();
    for row in &saved.rows {
        for run in &row.runs {
            out.extend_from_slice(sgr(run).as_bytes());
            let text: String = run
                .text
                .chars()
                .map(|c| if is_control(c) { '\u{fffd}' } else { c })
                .collect();
            out.extend_from_slice(text.as_bytes());
        }
        if !row.wrapped {
            out.extend_from_slice(b"\x1b[0m\r\n");
        }
    }
    out.extend_from_slice(b"\x1b[0m");
    debug_assert!(
        !out.windows(2).any(|w| w == b"\x1b]" || w == b"\x1bP"),
        "generated history must hold no OSC/DCS"
    );
    out
}

fn sgr(run: &Run) -> String {
    let mut params = vec!["0".to_owned()];
    for (bit, code) in [
        (flag::BOLD, "1"),
        (flag::DIM, "2"),
        (flag::ITALIC, "3"),
        (flag::UNDERLINE, "4"),
        (flag::INVERSE, "7"),
        (flag::HIDDEN, "8"),
        (flag::STRIKE, "9"),
    ] {
        if run.flags & bit != 0 {
            params.push(code.to_owned());
        }
    }
    params.extend(color_param(run.fg, false));
    params.extend(color_param(run.bg, true));
    format!("\x1b[{}m", params.join(";"))
}

fn color_param(color: SavedColor, bg: bool) -> Option<String> {
    let (base, bright, ext) = if bg { (40, 100, 48) } else { (30, 90, 38) };
    match color {
        SavedColor::Default => None,
        SavedColor::Named(n @ 0..=7) => Some((base + u16::from(n)).to_string()),
        SavedColor::Named(n) => Some((bright + u16::from(n.min(15)) - 8).to_string()),
        SavedColor::Indexed(i) => Some(format!("{ext};5;{i}")),
        SavedColor::Rgb(r, g, b) => Some(format!("{ext};2;{r};{g};{b}")),
    }
}
