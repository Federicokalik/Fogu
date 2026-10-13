//! Mouse → bytes for applications that enable mouse reporting: report encodings,
//! the Shift override and motion events.
//!
//! Tracking modes (mutually exclusive in `TermMode`): `MOUSE_REPORT_CLICK` (1000, presses,
//! releases, wheel), `MOUSE_DRAG` (1002, plus motion while a button is held), `MOUSE_MOTION`
//! (1003, plus all motion). Encodings: SGR (1006) `CSI < b;x;y M/m`, UTF-8 (1005) and the legacy
//! X10 byte form `CSI M b x y` (coordinates clamped at 223). Pure and GUI-agnostic.
#![expect(
    clippy::similar_names,
    reason = "`mode` and `mods` are the domain terms"
)]

use alacritty_terminal::term::TermMode;

use super::Mods;

/// Largest 1-based coordinate the legacy byte form can carry (`32 + 223 = 255`).
const LEGACY_MAX: usize = 223;
/// Largest 1-based coordinate the UTF-8 form can carry (`32 + 2015 = 2047`, two UTF-8 bytes).
const UTF8_MAX: usize = 2015;

/// A mouse button the terminal can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Primary button.
    Left,
    /// Middle button / wheel click.
    Middle,
    /// Secondary button.
    Right,
    /// Extra button "back" (xterm button 8).
    Back,
    /// Extra button "forward" (xterm button 9).
    Forward,
}

/// Wheel direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Away from the user: scroll back / up.
    Up,
    /// Towards the user: scroll forward / down.
    Down,
}

/// One mouse event, independent of the GUI toolkit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down.
    Press(Button),
    /// A button went up.
    Release(Button),
    /// The pointer entered a new cell; carries the button held, if any.
    Motion(Option<Button>),
    /// One wheel notch.
    Wheel(Direction),
}

/// Who handles a mouse event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Encode it with [`encode_mouse`] and send it to the application.
    Report,
    /// Local behaviour: selection, scrollback, paste, menus.
    Local,
    /// Wheel in the alternate screen without mouse mode: send arrow keys ([`alternate_scroll`]).
    AlternateScroll,
}

/// The one "report vs local" decision for every mouse gesture. Shift always means local.
pub fn mouse_disposition(action: MouseAction, mods: Mods, mode: TermMode) -> Disposition {
    if mods.shift {
        Disposition::Local
    } else if mode.intersects(TermMode::MOUSE_MODE) {
        Disposition::Report
    } else if matches!(action, MouseAction::Wheel(_))
        && mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
    {
        Disposition::AlternateScroll
    } else {
        Disposition::Local
    }
}

fn button_code(button: Button) -> usize {
    match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::Back => 128,
        Button::Forward => 129,
    }
}

/// Bytes reporting `action` at viewport cell (`col`, `row`, 0-based), or `None` when the active
/// tracking mode doesn't report it. Shift is never encoded (Shift events stay local).
pub fn encode_mouse(
    action: MouseAction,
    col: usize,
    row: usize,
    mods: Mods,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let code = match action {
        MouseAction::Press(b) => button_code(b),
        MouseAction::Release(b) if sgr => button_code(b),
        MouseAction::Release(_) => 3,
        MouseAction::Motion(held) => {
            let wanted = mode.contains(TermMode::MOUSE_MOTION)
                || (mode.contains(TermMode::MOUSE_DRAG) && held.is_some());
            if !wanted {
                return None;
            }
            32 + held.map_or(3, button_code)
        }
        MouseAction::Wheel(Direction::Up) => 64,
        MouseAction::Wheel(Direction::Down) => 65,
    } + 8 * usize::from(mods.alt)
        + 16 * usize::from(mods.ctrl);
    let (x, y) = (col.saturating_add(1), row.saturating_add(1));
    if sgr {
        let end = if matches!(action, MouseAction::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    let mut out = b"\x1b[M".to_vec();
    if mode.contains(TermMode::UTF8_MOUSE) {
        for v in [code, x.min(UTF8_MAX), y.min(UTF8_MAX)] {
            push_utf8(&mut out, 32 + v);
        }
    } else {
        for v in [code, x.min(LEGACY_MAX), y.min(LEGACY_MAX)] {
            out.push(u8::try_from(32 + v).unwrap_or(u8::MAX));
        }
    }
    Some(out)
}

/// Appends `v` (< 2048) as a UTF-8 encoded code point.
fn push_utf8(out: &mut Vec<u8>, v: usize) {
    let c = u32::try_from(v)
        .ok()
        .and_then(char::from_u32)
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    let mut buf = [0; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// `CSI I` / `CSI O` when the application asked for focus events (mode 1004).
pub fn encode_focus(focused: bool, mode: TermMode) -> Option<Vec<u8>> {
    mode.contains(TermMode::FOCUS_IN_OUT).then(|| {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    })
}

/// `lines` cursor-up/down key presses for a wheel scroll in the alternate screen (DECCKM-aware).
pub fn alternate_scroll(direction: Direction, lines: usize, mode: TermMode) -> Vec<u8> {
    let prefix = if mode.contains(TermMode::APP_CURSOR) {
        b'O'
    } else {
        b'['
    };
    let key = match direction {
        Direction::Up => b'A',
        Direction::Down => b'B',
    };
    [0x1b, prefix, key].repeat(lines)
}

#[cfg(test)]
#[path = "mouse_tests.rs"]
mod tests;
