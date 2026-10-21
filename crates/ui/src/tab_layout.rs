//! Tab header geometry: widths per the style's sizing policy and hit-testing on them, shared by
//! the tab bar (drawing) and the drag / scroll code (pointer → header), so both always agree.
//! Also where the tab strip sits in the window, which depends on the style (top title bar or
//! bottom status line, docked or islands).

use fogu_core::style::{BarPlacement, Geometry, TabClose, TabMarker, TabSizing, TabSpec};
use iced::{Point, Rectangle};

use crate::app::App;
use crate::tab_bar::{ADD_W, TOGGLE_W, WINDOW_BUTTONS_W, truncate};

/// Gap between two headers (empty, or a separator line).
pub(crate) const GAP: f32 = 1.0;
/// Inner horizontal padding of an island bar.
pub(crate) const ISLAND_PAD: f32 = 6.0;

/// Average glyph advance as a share of the font size (proportional and monospace chrome).
const ADVANCE_UI: f32 = 0.56;
const ADVANCE_MONO: f32 = 0.61;

/// The estimated width a header needs for `title` (fit sizing).
fn fit_width(spec: &TabSpec, title: &str, index: usize, mono: bool, gutter: f32) -> f32 {
    let advance = if mono { ADVANCE_MONO } else { ADVANCE_UI };
    let glyphs = crate::metrics::cells_f32(truncate(title).chars().count());
    let text = glyphs * spec.title_size * advance;
    let marker = match spec.marker {
        TabMarker::Dot => 6.0 + 8.0,
        TabMarker::Index => {
            crate::metrics::cells_f32((index + 1).to_string().len()) * spec.title_size * advance
                + 8.0
        }
        TabMarker::None => 0.0,
    };
    let close = match spec.close {
        TabClose::Never => 0.0,
        TabClose::Always | TabClose::Hover => 20.0 + 8.0,
    };
    text + marker + close + 2.0 * gutter + 2.0 * spec.inset
}

/// Header widths for `titles` with `room` px available to the strip.
pub(crate) fn widths(
    spec: &TabSpec,
    titles: &[&str],
    room: f32,
    mono: bool,
    gutter: f32,
) -> Vec<f32> {
    let (lo, hi) = (spec.min_width, spec.max_width.max(spec.min_width));
    match spec.sizing {
        TabSizing::Fixed => vec![spec.width; titles.len()],
        TabSizing::Fill => {
            let n = crate::metrics::cells_f32(titles.len().max(1));
            let each = (room / n - GAP).clamp(lo, hi);
            vec![each; titles.len()]
        }
        TabSizing::Fit => titles
            .iter()
            .enumerate()
            .map(|(i, title)| fit_width(spec, title, i, mono, gutter).clamp(lo, hi))
            .collect(),
    }
}

/// Left edge of each header, plus the end of the last one.
fn edges(widths: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(widths.len() + 1);
    let mut x = 0.0;
    out.push(x);
    for w in widths {
        x += w + GAP;
        out.push(x);
    }
    out
}

/// Left edge and width of header `index`.
pub(crate) fn span(widths: &[f32], index: usize) -> Option<(f32, f32)> {
    let left: f32 = widths.iter().take(index).map(|w| w + GAP).sum();
    widths.get(index).map(|w| (left, *w))
}

/// The gap between headers nearest to strip-local `x` (0 = before the first, `len` = after the
/// last). Ignores the strip's horizontal scroll, fine until a window holds more tabs than fit.
pub(crate) fn insertion_index(x: f32, widths: &[f32]) -> usize {
    edges(widths)
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
        .map_or(0, |(i, _)| i)
}

/// The header under strip-local `x` (`len` past the last one; same scroll shortcut).
pub(crate) fn header_at(x: f32, widths: &[f32]) -> usize {
    if x < 0.0 {
        return 0;
    }
    let edges = edges(widths);
    edges
        .windows(2)
        .position(|pair| matches!(pair, [a, b] if x >= *a && x < *b))
        .unwrap_or(widths.len())
}

impl App {
    /// The gap around islands (0 when docked).
    pub(crate) fn island_margin(&self) -> f32 {
        match self.look.layout.geometry {
            Geometry::Islands => self.look.layout.margin,
            Geometry::Docked => 0.0,
        }
    }

    /// The SFTP / port-forward toggles of the bar for the active tab: `(sftp, forwards)`.
    pub(crate) fn bar_flags(&self) -> (Option<bool>, Option<usize>) {
        let Some(tab) = self.active_tab() else {
            return (None, None);
        };
        let sftp = tab
            .panes
            .values()
            .any(|p| p.sftp.is_some())
            .then(|| tab.sftp.as_ref().is_some_and(|p| p.visible));
        let forwards = tab
            .panes
            .values()
            .any(crate::tab::Pane::is_ssh)
            .then(|| crate::forwards_dialog::listening(tab.panes.values()));
        (sftp, forwards)
    }

    /// Width the tab strip may use (fill sizing divides it).
    fn strip_room(&self) -> f32 {
        let margin = self.island_margin();
        let pad = if margin > 0.0 { ISLAND_PAD } else { 0.0 };
        let mut room = self.window_size.width - 2.0 * (margin + pad) - ADD_W;
        if self.look.bar.placement == BarPlacement::Top {
            let (sftp, forwards) = self.bar_flags();
            let toggles = 2 + usize::from(sftp.is_some()) + usize::from(forwards.is_some());
            room -= self.bar_inset() + crate::metrics::cells_f32(toggles) * TOGGLE_W;
            if !cfg!(target_os = "macos") {
                room -= WINDOW_BUTTONS_W;
            }
        }
        room.max(0.0)
    }

    /// Every header's width, as the bar draws them.
    pub(crate) fn tab_widths(&self) -> Vec<f32> {
        let titles: Vec<&str> = self.tabs.iter().map(crate::tab::Tab::title).collect();
        widths(
            &self.look.tabs,
            &titles,
            self.strip_room(),
            self.look.text.mono_chrome,
            self.look.layout.gutter,
        )
    }

    /// The bar holding the tabs, in window coordinates.
    pub(crate) fn tab_bar_rect(&self) -> Rectangle {
        let margin = self.island_margin();
        let height = self.look.bar.height;
        let y = match self.look.bar.placement {
            BarPlacement::Top => margin,
            BarPlacement::Bottom => self.window_size.height - margin - height,
        };
        Rectangle {
            x: margin,
            y,
            width: (self.window_size.width - 2.0 * margin).max(0.0),
            height,
        }
    }

    /// Window x of the first header's left edge.
    pub(crate) fn strip_left(&self) -> f32 {
        let margin = self.island_margin();
        let pad = if margin > 0.0 { ISLAND_PAD } else { 0.0 };
        let inset = match self.look.bar.placement {
            BarPlacement::Top => self.bar_inset(),
            BarPlacement::Bottom => 0.0,
        };
        margin + pad + inset
    }

    /// Whether window point `at` is over the tab strip's bar.
    pub(crate) fn over_tab_bar(&self, at: Point) -> bool {
        let bar = self.tab_bar_rect();
        at.y >= bar.y && at.y < bar.y + bar.height
    }

    /// Height of the chrome above the panes (bar or top strip, its rule, island gaps).
    pub(crate) fn top_chrome_h(&self) -> f32 {
        let margin = self.island_margin();
        let bar = &self.look.bar;
        let rule = if bar.rule && margin == 0.0 { 1.0 } else { 0.0 };
        let strip = match bar.placement {
            BarPlacement::Top => bar.height,
            BarPlacement::Bottom => bar.strip,
        };
        2.0 * margin + strip + rule
    }
}

#[cfg(test)]
#[expect(clippy::indexing_slicing, reason = "tests")]
mod tests {
    use fogu_core::StyleCatalog;

    use super::*;

    fn spec(sizing: TabSizing) -> TabSpec {
        let mut spec = StyleCatalog::builtins().default_style().tabs;
        spec.sizing = sizing;
        spec.width = 200.0;
        spec.min_width = 80.0;
        spec.max_width = 240.0;
        spec
    }

    #[test]
    fn fixed_widths_keep_the_old_stride() {
        let w = widths(
            &spec(TabSizing::Fixed),
            &["a", "b", "c"],
            1000.0,
            false,
            12.0,
        );
        assert_eq!(w, vec![200.0; 3]);
        let stride = 200.0 + GAP;
        assert_eq!(insertion_index(-30.0, &w), 0);
        assert_eq!(insertion_index(stride * 0.4, &w), 0);
        assert_eq!(insertion_index(stride * 0.6, &w), 1);
        assert_eq!(insertion_index(stride * 2.5 + 1.0, &w), 3);
        assert_eq!(insertion_index(stride * 10.0, &w), 3);
        assert_eq!(header_at(stride * 1.5, &w), 1);
        assert_eq!(header_at(stride * 5.0, &w), 3);
        assert_eq!(header_at(-4.0, &w), 0);
        assert_eq!(span(&w, 2), Some((stride * 2.0, 200.0)));
    }

    #[test]
    fn fill_divides_the_room_within_bounds() {
        let s = spec(TabSizing::Fill);
        let w = widths(&s, &["a", "b"], 400.0, false, 12.0);
        assert!((w[0] - (200.0 - GAP)).abs() < 0.01, "{w:?}");
        let w = widths(&s, &["a"; 10], 400.0, false, 12.0);
        assert!(w.iter().all(|x| (*x - 80.0).abs() < 0.01), "{w:?}");
        let w = widths(&s, &["a"], 4000.0, false, 12.0);
        assert!((w[0] - 240.0).abs() < 0.01, "{w:?}");
    }

    #[test]
    fn fit_grows_with_the_title_and_hits_by_offsets() {
        let s = spec(TabSizing::Fit);
        let w = widths(&s, &["sh", "a much longer title"], 1000.0, false, 12.0);
        assert!(w[0] < w[1], "{w:?}");
        assert!(w[0] >= 80.0 && w[1] <= 240.0);
        let (left, width) = span(&w, 1).unwrap_or_default();
        assert_eq!(header_at(left + width / 2.0, &w), 1);
        assert_eq!(header_at(left - GAP / 2.0 - 1.0, &w), 0);
    }
}
