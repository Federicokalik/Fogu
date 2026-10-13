use super::*;

const N: Mods = Mods::NONE;
const S: Mods = Mods {
    shift: true,
    ..Mods::NONE
};
const A: Mods = Mods {
    alt: true,
    ..Mods::NONE
};
const C: Mods = Mods {
    ctrl: true,
    ..Mods::NONE
};
const CA: Mods = Mods {
    ctrl: true,
    alt: true,
    ..Mods::NONE
};

const NONE: TermMode = TermMode::NONE;
const CLICK: TermMode = TermMode::MOUSE_REPORT_CLICK;
const DRAG: TermMode = TermMode::MOUSE_DRAG;
const MOTION: TermMode = TermMode::MOUSE_MOTION;
const SGR: TermMode = TermMode::SGR_MOUSE;
const UTF8: TermMode = TermMode::UTF8_MOUSE;
const ALT: TermMode = TermMode::ALT_SCREEN;
const ASCROLL: TermMode = TermMode::ALTERNATE_SCROLL;

use Button::{Back, Forward, Left, Middle, Right};
use MouseAction::{Motion, Press, Release, Wheel};

fn m(a: TermMode, b: TermMode) -> TermMode {
    a | b
}

/// action, col, row, mods, mode, expected bytes.
type Row = (
    MouseAction,
    usize,
    usize,
    Mods,
    TermMode,
    Option<&'static [u8]>,
);

#[test]
fn encode_mouse_table() {
    let up = Wheel(Direction::Up);
    let down = Wheel(Direction::Down);
    #[rustfmt::skip]
    let rows: Vec<Row> = vec![
        // Legacy X10 byte form, click tracking (1000).
        (Press(Left), 0, 0, N, CLICK, Some(b"\x1b[M !!")),
        (Release(Left), 0, 0, N, CLICK, Some(b"\x1b[M#!!")),
        (Release(Right), 0, 0, N, CLICK, Some(b"\x1b[M#!!")),
        (Press(Right), 4, 9, N, CLICK, Some(b"\x1b[M\"%*")),
        (Press(Middle), 0, 0, N, CLICK, Some(b"\x1b[M!!!")),
        (Press(Left), 0, 0, C, CLICK, Some(b"\x1b[M0!!")),
        (Press(Left), 0, 0, A, CLICK, Some(b"\x1b[M(!!")),
        (Press(Left), 0, 0, CA, CLICK, Some(b"\x1b[M8!!")),
        (Press(Left), 0, 0, S, CLICK, Some(b"\x1b[M !!")),
        (up, 0, 0, N, CLICK, Some(b"\x1b[M`!!")),
        (down, 0, 0, N, CLICK, Some(b"\x1b[Ma!!")),
        (Press(Back), 0, 0, N, CLICK, Some(b"\x1b[M\xa0!!")),
        // Motion filters.
        (Motion(None), 0, 0, N, CLICK, None),
        (Motion(Some(Left)), 0, 0, N, CLICK, None),
        (Motion(Some(Left)), 0, 0, N, DRAG, Some(b"\x1b[M@!!")),
        (Motion(None), 0, 0, N, DRAG, None),
        (Motion(None), 0, 0, N, MOTION, Some(b"\x1b[MC!!")),
        (Motion(Some(Right)), 0, 0, N, MOTION, Some(b"\x1b[MB!!")),
        // No mouse mode: nothing, whatever the screen.
        (Press(Left), 0, 0, N, NONE, None),
        (up, 0, 0, N, m(ALT, ASCROLL), None),
        // Legacy clamps at 223.
        (Press(Left), 300, 0, N, CLICK, Some(b"\x1b[M \xff!")),
        (Press(Left), 0, 500, N, DRAG, Some(b"\x1b[M !\xff")),
        (Press(Left), 222, 0, N, CLICK, Some(b"\x1b[M \xff!")),
        // SGR (1006).
        (Press(Left), 0, 0, N, m(CLICK, SGR), Some(b"\x1b[<0;1;1M")),
        (Release(Left), 0, 0, N, m(CLICK, SGR), Some(b"\x1b[<0;1;1m")),
        (Release(Right), 299, 0, N, m(CLICK, SGR), Some(b"\x1b[<2;300;1m")),
        (Press(Left), 300, 0, N, m(CLICK, SGR), Some(b"\x1b[<0;301;1M")),
        (up, 0, 0, C, m(CLICK, SGR), Some(b"\x1b[<80;1;1M")),
        (down, 2, 3, A, m(DRAG, SGR), Some(b"\x1b[<73;3;4M")),
        (Motion(Some(Left)), 4, 4, N, m(DRAG, SGR), Some(b"\x1b[<32;5;5M")),
        (Motion(None), 4, 4, N, m(MOTION, SGR), Some(b"\x1b[<35;5;5M")),
        (Motion(None), 4, 4, N, m(DRAG, SGR), None),
        (Press(Back), 0, 0, N, m(CLICK, SGR), Some(b"\x1b[<128;1;1M")),
        (Press(Forward), 0, 0, N, m(CLICK, SGR), Some(b"\x1b[<129;1;1M")),
        (Press(Left), 0, 0, N, SGR, None),
        // UTF-8 (1005).
        (Press(Left), 50, 0, N, m(CLICK, UTF8), Some(b"\x1b[M S!")),
        (Press(Left), 300, 0, N, m(CLICK, UTF8), Some(b"\x1b[M \xc5\x8d!")),
        (Press(Left), 3000, 0, N, m(CLICK, UTF8), Some(b"\x1b[M \xdf\xbf!")),
        (Press(Back), 0, 0, N, m(CLICK, UTF8), Some(b"\x1b[M\xc2\xa0!!")),
    ];
    for (i, (action, col, row, mods, mode, want)) in rows.into_iter().enumerate() {
        let got = encode_mouse(action, col, row, mods, mode);
        assert_eq!(
            got.as_deref(),
            want,
            "row {i}: {action:?} at ({col},{row}) {mods:?} {mode:?}"
        );
    }
}

#[test]
fn mouse_disposition_table() {
    use Disposition::{AlternateScroll, Local, Report};
    let up = Wheel(Direction::Up);
    #[rustfmt::skip]
    let rows = [
        (Press(Left), N, NONE, Local),
        (Press(Left), N, CLICK, Report),
        (Release(Left), N, DRAG, Report),
        (Motion(None), N, CLICK, Report),
        (up, N, NONE, Local),
        (up, N, ASCROLL, Local),
        (up, N, ALT, Local),
        (up, N, m(ALT, ASCROLL), AlternateScroll),
        (up, N, m(m(ALT, ASCROLL), CLICK), Report),
        (up, C, m(ALT, ASCROLL), AlternateScroll),
        (Press(Left), N, m(ALT, ASCROLL), Local),
        (Press(Middle), N, m(MOTION, SGR), Report),
    ];
    for (i, (action, mods, mode, want)) in rows.into_iter().enumerate() {
        assert_eq!(mouse_disposition(action, mods, mode), want, "row {i}");
    }
}

#[test]
fn shift_is_local_in_every_mode() {
    let modes = [
        NONE,
        CLICK,
        DRAG,
        MOTION,
        m(CLICK, SGR),
        m(MOTION, UTF8),
        m(ALT, ASCROLL),
        m(m(ALT, ASCROLL), DRAG),
    ];
    let actions = [
        Press(Left),
        Press(Right),
        Release(Middle),
        Motion(None),
        Motion(Some(Left)),
        Wheel(Direction::Up),
        Wheel(Direction::Down),
    ];
    let shifted = [
        S,
        Mods {
            shift: true,
            ctrl: true,
            alt: true,
            logo: true,
        },
    ];
    for mode in modes {
        for action in actions {
            for mods in shifted {
                assert_eq!(
                    mouse_disposition(action, mods, mode),
                    Disposition::Local,
                    "{action:?} {mods:?} {mode:?}"
                );
            }
        }
    }
}

#[test]
fn focus_and_alternate_scroll() {
    let focus = TermMode::FOCUS_IN_OUT;
    assert_eq!(encode_focus(true, focus).as_deref(), Some(&b"\x1b[I"[..]));
    assert_eq!(encode_focus(false, focus).as_deref(), Some(&b"\x1b[O"[..]));
    assert_eq!(encode_focus(true, NONE), None);
    assert_eq!(alternate_scroll(Direction::Up, 2, NONE), b"\x1b[A\x1b[A");
    assert_eq!(
        alternate_scroll(Direction::Down, 1, TermMode::APP_CURSOR),
        b"\x1bOB"
    );
    assert_eq!(alternate_scroll(Direction::Up, 0, NONE), Vec::<u8>::new());
}
