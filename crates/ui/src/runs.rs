//! Style runs: consecutive cells of one row with equal resolved style. Non-ASCII, wide and
//! combining cells get a run of their own so a fallback font's advance can never push later
//! cells off the grid.

use fogu_term::{Cell, CellFlags, Rgb};

use crate::palette::{Colors, blend_half};

/// Underline variants drawn as quads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Underline {
    /// SGR 4 (curly/dotted/dashed are drawn as single for now).
    Single,
    /// SGR 21 / 4:2.
    Double,
}

/// Resolved style shared by all cells of a run.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent SGR attributes, compared as a whole for run grouping"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    /// Text colour (after inverse/dim/bold-as-bright).
    pub fg: Rgb,
    /// Background colour (after inverse).
    pub bg: Rgb,
    /// Underline colour.
    pub underline_color: Rgb,
    /// Bold weight.
    pub bold: bool,
    /// Italic style.
    pub italic: bool,
    /// Underline kind.
    pub underline: Option<Underline>,
    /// Strikethrough.
    pub strike: bool,
    /// SGR 8: no text drawn.
    pub hidden: bool,
}

/// Consecutive cells of one row with equal style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// First column.
    pub col: usize,
    /// Width in cells (2 for a wide char).
    pub cols: usize,
    /// Text of the cells, combining marks included.
    pub text: String,
    /// Shared style.
    pub style: Style,
    /// Holds one non-ASCII, wide or combining cell; never extended.
    pub solo: bool,
}

/// Resolves a cell's colours and attributes.
pub fn cell_style(cell: &Cell, colors: &Colors) -> Style {
    let flags = cell.flags;
    let bold = flags.contains(CellFlags::BOLD);
    let mut fg = colors.resolve(cell.fg, bold);
    let mut bg = colors.resolve(cell.bg, false);
    if flags.contains(CellFlags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(CellFlags::DIM) {
        fg = blend_half(fg, bg);
    }
    let underline = if flags.contains(CellFlags::DOUBLE_UNDERLINE) {
        Some(Underline::Double)
    } else if flags.intersects(CellFlags::ALL_UNDERLINES) {
        Some(Underline::Single)
    } else {
        None
    };
    Style {
        fg,
        bg,
        underline_color: cell
            .underline_color
            .map_or(fg, |c| colors.resolve(c, false)),
        bold,
        italic: flags.contains(CellFlags::ITALIC),
        underline,
        strike: flags.contains(CellFlags::STRIKEOUT),
        hidden: flags.contains(CellFlags::HIDDEN),
    }
}

/// Groups one row into style runs (into `runs`, which is cleared first). Wide-char spacers are
/// skipped (the wide char's run already covers 2 cells).
pub fn row_runs(row: &[Cell], colors: &Colors, runs: &mut Vec<Run>) {
    runs.clear();
    for (col, cell) in row.iter().enumerate() {
        if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
            continue;
        }
        let style = cell_style(cell, colors);
        let wide = cell.flags.contains(CellFlags::WIDE_CHAR);
        let solo = wide || !cell.ch.is_ascii() || cell.zerowidth.is_some();
        if let Some(last) = runs.last_mut()
            && !solo
            && !last.solo
            && last.style == style
            && last.col + last.cols == col
        {
            last.text.push(cell.ch);
            last.cols += 1;
            continue;
        }
        let mut text = String::new();
        text.push(cell.ch);
        text.extend(cell.zerowidth.iter().flat_map(|z| z.iter()));
        runs.push(Run {
            col,
            cols: if wide { 2 } else { 1 },
            text,
            style,
            solo,
        });
    }
}

#[cfg(test)]
mod tests {
    use fogu_core::ColorScheme;
    use fogu_term::{Color, NamedColor};

    use super::*;

    fn cell(ch: char) -> Cell {
        Cell {
            ch,
            zerowidth: None,
            fg: Color::Named(NamedColor::Foreground),
            bg: Color::Named(NamedColor::Background),
            underline_color: None,
            flags: CellFlags::empty(),
        }
    }

    fn styled(ch: char, flags: CellFlags) -> Cell {
        Cell { flags, ..cell(ch) }
    }

    fn summary(row: &[Cell]) -> Vec<(usize, usize, String)> {
        let colors = Colors::new(&ColorScheme::default());
        let mut runs = Vec::new();
        row_runs(row, &colors, &mut runs);
        runs.into_iter().map(|r| (r.col, r.cols, r.text)).collect()
    }

    #[test]
    fn groups_equal_styles() {
        let row = [
            cell('a'),
            cell('b'),
            styled('c', CellFlags::BOLD),
            cell('d'),
            cell(' '),
        ];
        assert_eq!(
            summary(&row),
            [(0, 2, "ab".into()), (2, 1, "c".into()), (3, 2, "d ".into())]
        );
    }

    #[test]
    fn wide_chars_span_two_cells_and_spacers_are_skipped() {
        let row = [
            cell('a'),
            styled('中', CellFlags::WIDE_CHAR),
            styled(' ', CellFlags::WIDE_CHAR_SPACER),
            cell('b'),
            cell('c'),
        ];
        assert_eq!(
            summary(&row),
            [(0, 1, "a".into()), (1, 2, "中".into()), (3, 2, "bc".into())]
        );
    }

    #[test]
    fn non_ascii_and_combining_cells_stand_alone() {
        let mut e = cell('e');
        e.zerowidth = Some(Box::from(['\u{301}']));
        let row = [cell('x'), cell('─'), cell('─'), e, cell('y')];
        assert_eq!(
            summary(&row),
            [
                (0, 1, "x".into()),
                (1, 1, "─".into()),
                (2, 1, "─".into()),
                (3, 1, "e\u{301}".into()),
                (4, 1, "y".into()),
            ]
        );
    }

    #[test]
    fn inverse_dim_and_hidden_styles() {
        let colors = Colors::new(&ColorScheme::default());
        let fg = colors.foreground();
        let bg = colors.background();
        let inv = cell_style(&styled('a', CellFlags::INVERSE), &colors);
        assert_eq!((inv.fg, inv.bg), (bg, fg));
        let dim = cell_style(&styled('a', CellFlags::DIM), &colors);
        assert_eq!(dim.fg, blend_half(fg, bg));
        assert!(cell_style(&styled('a', CellFlags::HIDDEN), &colors).hidden);
        let ul = cell_style(&styled('a', CellFlags::UNDERCURL), &colors);
        assert_eq!(ul.underline, Some(Underline::Single));
        let dbl = cell_style(&styled('a', CellFlags::DOUBLE_UNDERLINE), &colors);
        assert_eq!(dbl.underline, Some(Underline::Double));
    }
}
