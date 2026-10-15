//! `TermHandle::reset_for_new_session`: a restarted / reconnected pane keeps its primary screen
//! and scrollback, but nothing the old (attacker-controlled) session set reaches the new one.

use fogu_term::{
    CellFlags, Color, CursorShape, NamedColor, Scroll, TermEvent, TermHandle, TermMode,
    WORD_SEPARATORS,
};

const SEPARATOR: &str = "── restarted ──";

fn term() -> TermHandle {
    TermHandle::new(30, 5, 100, WORD_SEPARATORS)
}

/// Visible rows with the separator line's index.
fn separator_row(t: &TermHandle) -> Option<usize> {
    t.dump_grid().lines().position(|l| l == SEPARATOR)
}

#[test]
fn keeps_primary_screen_and_history_drops_the_alternate_screen() {
    let mut t = term();
    for i in 1..=12 {
        t.feed(format!("line{i}\r\n").as_bytes());
    }
    t.feed(b"$ vim notes");
    t.feed(b"\x1b[?1049h\x1b[HVIM SCREEN");
    assert!(t.mode().contains(TermMode::ALT_SCREEN));
    t.scroll_display(Scroll::Delta(2));
    t.reset_for_new_session(SEPARATOR);

    assert!(!t.mode().contains(TermMode::ALT_SCREEN));
    let screen = t.dump_grid();
    assert!(!screen.contains("VIM SCREEN"), "{screen}");
    let lines: Vec<&str> = screen.lines().collect();
    let sep = separator_row(&t).unwrap_or_default();
    let above = sep.checked_sub(1).and_then(|i| lines.get(i));
    assert_eq!(above, Some(&"$ vim notes"), "{screen}");
    assert_eq!(t.snapshot().display_offset, 0, "back at the bottom");
    // The cursor waits on the line after the separator, at column 0.
    let cursor = t.snapshot().cursor;
    assert_eq!((cursor.row, cursor.col), (sep + 1, 0));
    t.scroll_display(Scroll::Top);
    assert!(t.dump_grid().starts_with("line1\n"), "history kept");
}

#[test]
fn a_cursor_at_column_zero_gets_no_blank_line_before_the_separator() {
    let mut t = term();
    t.feed(b"bye\r\n");
    t.reset_for_new_session(SEPARATOR);
    assert_eq!(separator_row(&t), Some(1), "{}", t.dump_grid());
}

#[test]
fn every_mode_the_old_session_set_is_reset() {
    let fresh = term().mode();
    let mut t = term();
    // App cursor keys + keypad, all mouse modes and encodings, bracketed paste, focus reports,
    // hidden cursor, no wrap, insert, LNM, origin, no alternate scroll.
    t.feed(b"\x1b[?1h\x1b=\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1005h\x1b[?1006h");
    t.feed(b"\x1b[?2004h\x1b[?1004h\x1b[?25l\x1b[?7l\x1b[4h\x1b[20h\x1b[?6h\x1b[?1007l");
    assert_ne!(t.mode(), fresh);
    t.reset_for_new_session(SEPARATOR);
    assert_eq!(t.mode(), fresh);
}

#[test]
fn attributes_charsets_cursor_style_and_scroll_region_are_reset() {
    let mut t = term();
    // Bold red reverse, DEC line drawing in G0, steady bar cursor, a two-line scroll region.
    t.feed(b"\x1b[1;31;7m\x1b(0\x1b[6 q\x1b[2;3r");
    t.reset_for_new_session(SEPARATOR);
    assert_eq!(t.snapshot().cursor.shape, CursorShape::Block);
    t.feed(b"qx");
    let snap = t.snapshot();
    let row = snap.cursor.row;
    let cells = snap.rows().nth(row).unwrap_or_default();
    let first = cells.first().map(|c| (c.ch, c.fg, c.flags));
    assert_eq!(
        first,
        Some((
            'q',
            Color::Named(NamedColor::Foreground),
            CellFlags::empty()
        )),
        "no line drawing ('─'), no SGR"
    );
    // Full-screen scroll region again: new lines push the screen into history.
    let before = t.snapshot().history_len;
    t.feed(b"\r\n\r\n\r\n\r\n\r\n\r\n");
    assert!(t.snapshot().history_len > before);
}

#[test]
fn pending_events_and_the_title_stack_do_not_leak() {
    let mut t = term();
    // Title, pushed title, OSC 52 write, a DSR query: all undelivered at the reset.
    t.feed(b"\x1b]0;evil\x07\x1b[22;0t\x1b]0;evil2\x07\x1b]52;c;aGk=\x07\x1b[6n");
    t.reset_for_new_session(SEPARATOR);
    assert_eq!(t.drain_events(), Vec::new());
    // The new session pops a title: the old session's pushed title must not come back.
    t.feed(b"\x1b[23;0t");
    let titles: Vec<TermEvent> = t
        .drain_events()
        .into_iter()
        .filter(|e| matches!(e, TermEvent::Title(_)))
        .collect();
    assert_eq!(titles, Vec::new());
}
