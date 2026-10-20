//! Headless tests of scrollback and selection through `TermHandle`.

use fogu_term::{Column, Line, Point, Scroll, SelectionType, Side, TermHandle, WORD_SEPARATORS};

fn fed(cols: u16, rows: u16, bytes: &[u8]) -> TermHandle {
    let mut t = TermHandle::new(cols, rows, 200, WORD_SEPARATORS);
    t.feed(bytes);
    t
}

fn at(line: i32, col: usize) -> Point {
    Point::new(Line(line), Column(col))
}

fn select(t: &mut TermHandle, kind: SelectionType, from: Point, to: Point) -> Option<String> {
    t.start_selection(kind, from, Side::Left);
    t.update_selection(to, Side::Right);
    t.selection_text()
}

#[test]
fn scroll_back_fifty_of_hundred_lines() {
    let mut t = fed(10, 5, b"");
    for i in 1..=100 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    t.scroll_display(Scroll::Delta(50));
    let snap = t.snapshot();
    assert_eq!(snap.display_offset, 50);
    insta::assert_snapshot!(t.dump_grid());
}

#[test]
fn simple_selection_spans_lines_and_trims() {
    let mut t = fed(20, 3, b"hello world   \r\nsecond line");
    let text = select(&mut t, SelectionType::Simple, at(0, 6), at(1, 5));
    assert_eq!(text.as_deref(), Some("world\nsecond"));
}

#[test]
fn lines_selection_takes_whole_lines() {
    let mut t = fed(20, 3, b"hello world  \r\nsecond line");
    let text = select(&mut t, SelectionType::Lines, at(0, 3), at(1, 0));
    assert_eq!(text.as_deref(), Some("hello world\nsecond line\n"));
}

#[test]
fn block_selection_takes_columns() {
    let mut t = fed(10, 3, b"abcdef\r\nghijkl\r\nmnopqr");
    let text = select(&mut t, SelectionType::Block, at(0, 1), at(2, 3));
    assert_eq!(text.as_deref(), Some("bcd\nhij\nnop"));
    let spans = t.snapshot().selection_spans();
    assert_eq!(spans, [(0, 1, 3), (1, 1, 3), (2, 1, 3)]);
}

#[test]
fn semantic_selection_stops_at_default_separators() {
    let mut t = fed(30, 2, b"foo (bar.baz) qux");
    let text = select(&mut t, SelectionType::Semantic, at(0, 7), at(0, 7));
    assert_eq!(text.as_deref(), Some("bar.baz"));
}

#[test]
fn selection_in_history_maps_to_viewport_rows() {
    let mut t = fed(8, 3, b"");
    for i in 0..10 {
        t.feed(format!("row{i}\r\n").as_bytes());
    }
    t.scroll_display(Scroll::Delta(4));
    // Viewport row 1 at offset 4 is grid line -3.
    let text = select(&mut t, SelectionType::Simple, at(-3, 0), at(-3, 3));
    assert_eq!(text.as_deref(), Some("row5"));
    assert_eq!(t.snapshot().selection_spans(), [(1, 0, 3)]);
}

#[test]
fn clear_and_out_of_range_points() {
    let mut t = fed(8, 2, b"abc");
    assert_eq!(t.selection_text(), None);
    // Points far outside the grid are clamped (to the whole 2-row grid), never panic.
    let text = select(&mut t, SelectionType::Simple, at(-500, 0), at(90, 90));
    assert_eq!(text.as_deref(), Some("abc\n"));
    t.clear_selection();
    assert_eq!(t.selection_text(), None);
    assert_eq!(t.snapshot().selection_spans(), []);
}

#[test]
fn search_walks_matches_up_and_wraps() {
    let mut t = fed(20, 3, b"");
    for line in ["foo 1", "bar", "Foo 2", "x.y", "foo 3"] {
        t.feed(format!("{line}\r\n").as_bytes());
    }
    // From the cursor (below everything), upward: the newest match first, then older ones.
    assert!(t.search("foo", true));
    assert_eq!(t.selection_text().as_deref(), Some("foo"));
    assert_eq!(t.snapshot().display_offset, 0);
    assert!(t.search("foo", true));
    assert_eq!(t.selection_text().as_deref(), Some("Foo"));
    assert!(t.search("foo", true));
    assert!(
        t.snapshot().display_offset > 0,
        "scrolled to the oldest match"
    );
    assert!(t.search("foo", true), "wraps around");
    // An uppercase letter makes it case-sensitive; regex characters are plain text.
    assert!(t.search("Foo", false));
    assert_eq!(t.selection_text().as_deref(), Some("Foo"));
    assert!(t.search("x.y", false));
    assert!(!t.search("x*y", false));
    assert_eq!(t.selection_text(), None);
}
