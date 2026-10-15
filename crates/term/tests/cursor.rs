//! The configured cursor shape is the default until the application picks one (DECSCUSR).

use fogu_term::{CursorShape, TermHandle, WORD_SEPARATORS};

#[test]
fn configured_shape_applies_until_decscusr_overrides_it() {
    let mut term = TermHandle::new(20, 5, 100, WORD_SEPARATORS);
    term.set_default_cursor_shape(CursorShape::Beam);
    assert_eq!(term.snapshot().cursor.shape, CursorShape::Beam);
    term.feed(b"\x1b[4 q"); // steady underline
    assert_eq!(term.snapshot().cursor.shape, CursorShape::Underline);
    term.feed(b"\x1b[0 q"); // back to the default
    assert_eq!(term.snapshot().cursor.shape, CursorShape::Beam);
}
