//! iced key events → GUI-agnostic [`KeyInput`] for `fogu_term::input::encode_key`.

use fogu_term::Scroll;
use fogu_term::input::{KeyCode, KeyInput, Mods};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Location, Modifiers};

use crate::message::PasteSource;

/// A key chord the terminal handles itself instead of sending it to the shell.
#[derive(Clone, Copy, Debug)]
pub enum LocalKey {
    /// Scroll the view through history.
    Scroll(Scroll),
    /// Copy the selection to CLIPBOARD.
    Copy,
    /// Paste from `source`.
    Paste(PasteSource),
    /// Grow the font by 1 pt.
    ZoomIn,
    /// Shrink the font by 1 pt.
    ZoomOut,
    /// Reset the font to its default size.
    ZoomReset,
    /// Clear the screen and scrollback.
    Clear,
}

/// Hardcoded chords until a keymap exists: Shift+PageUp/PageDown/Home/End scroll,
/// Ctrl+Shift+C copies, Ctrl+Shift+V / Shift+Insert pastes CLIPBOARD, Ctrl+Shift+L clears,
/// Ctrl+=/Ctrl+-/Ctrl+0 zoom in/out/reset.
pub fn local_key(key: &Key, modifiers: Modifiers) -> Option<LocalKey> {
    if modifiers.alt() || modifiers.logo() {
        return None;
    }
    let (shift, ctrl) = (modifiers.shift(), modifiers.control());
    match key {
        Key::Character(c) if ctrl && shift && c.eq_ignore_ascii_case("c") => Some(LocalKey::Copy),
        Key::Character(c) if ctrl && shift && c.eq_ignore_ascii_case("v") => {
            Some(LocalKey::Paste(PasteSource::Clipboard))
        }
        Key::Character(c) if ctrl && shift && c.eq_ignore_ascii_case("l") => Some(LocalKey::Clear),
        Key::Character(c) if ctrl && !shift && c.as_str() == "=" => Some(LocalKey::ZoomIn),
        Key::Character(c) if ctrl && !shift && c.as_str() == "-" => Some(LocalKey::ZoomOut),
        Key::Character(c) if ctrl && !shift && c.as_str() == "0" => Some(LocalKey::ZoomReset),
        Key::Named(Named::Insert) if shift && !ctrl => {
            Some(LocalKey::Paste(PasteSource::Clipboard))
        }
        Key::Named(named) if shift && !ctrl => Some(LocalKey::Scroll(match named {
            Named::PageUp => Scroll::PageUp,
            Named::PageDown => Scroll::PageDown,
            Named::Home => Scroll::Top,
            Named::End => Scroll::Bottom,
            _ => return None,
        })),
        _ => None,
    }
}

/// Converts a key press. `modified_key` is the layout's key with Shift applied (iced's
/// `KeyPressed::modified_key`); keys the terminal has no encoding for give `None`.
pub fn key_input(
    modified_key: &Key,
    location: Location,
    modifiers: Modifiers,
    text: Option<&str>,
) -> Option<KeyInput> {
    let numpad = location == Location::Numpad;
    let key = match modified_key {
        // Multi-char keysyms carry their full content in `text`; the first char names the key.
        Key::Character(s) => {
            let c = s.chars().next()?;
            if numpad {
                KeyCode::Numpad(c)
            } else {
                KeyCode::Char(c)
            }
        }
        Key::Named(named) => named_key(*named, numpad)?,
        Key::Unidentified => return None,
    };
    Some(KeyInput {
        key,
        text: text.map(str::to_owned),
        mods: mods(modifiers),
    })
}

/// iced modifiers as the encoder's [`Mods`].
pub fn mods(modifiers: Modifiers) -> Mods {
    Mods {
        shift: modifiers.shift(),
        alt: modifiers.alt(),
        ctrl: modifiers.control(),
        logo: modifiers.logo(),
    }
}

fn named_key(named: Named, numpad: bool) -> Option<KeyCode> {
    Some(match named {
        Named::Enter if numpad => KeyCode::NumpadEnter,
        Named::Enter => KeyCode::Enter,
        Named::Tab => KeyCode::Tab,
        Named::Backspace => KeyCode::Backspace,
        Named::Escape => KeyCode::Escape,
        Named::Space => KeyCode::Char(' '),
        Named::ArrowUp => KeyCode::Up,
        Named::ArrowDown => KeyCode::Down,
        Named::ArrowLeft => KeyCode::Left,
        Named::ArrowRight => KeyCode::Right,
        Named::Home => KeyCode::Home,
        Named::End => KeyCode::End,
        Named::Insert => KeyCode::Insert,
        Named::Delete => KeyCode::Delete,
        Named::PageUp => KeyCode::PageUp,
        Named::PageDown => KeyCode::PageDown,
        Named::F1 => KeyCode::F(1),
        Named::F2 => KeyCode::F(2),
        Named::F3 => KeyCode::F(3),
        Named::F4 => KeyCode::F(4),
        Named::F5 => KeyCode::F(5),
        Named::F6 => KeyCode::F(6),
        Named::F7 => KeyCode::F(7),
        Named::F8 => KeyCode::F(8),
        Named::F9 => KeyCode::F(9),
        Named::F10 => KeyCode::F(10),
        Named::F11 => KeyCode::F(11),
        Named::F12 => KeyCode::F(12),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_characters_named_and_numpad() {
        let ctrl = Modifiers::CTRL;
        let k = key_input(
            &Key::Character("c".into()),
            Location::Standard,
            ctrl,
            Some("\x03"),
        );
        assert_eq!(
            k.map(|k| (k.key, k.mods.ctrl)),
            Some((KeyCode::Char('c'), true))
        );

        let k = key_input(
            &Key::Named(Named::Enter),
            Location::Numpad,
            Modifiers::empty(),
            None,
        );
        assert_eq!(k.map(|k| k.key), Some(KeyCode::NumpadEnter));

        let k = key_input(
            &Key::Character("5".into()),
            Location::Numpad,
            Modifiers::empty(),
            None,
        );
        assert_eq!(k.map(|k| k.key), Some(KeyCode::Numpad('5')));

        let k = key_input(&Key::Named(Named::Space), Location::Standard, ctrl, None);
        assert_eq!(k.map(|k| k.key), Some(KeyCode::Char(' ')));

        let none = key_input(
            &Key::Named(Named::Shift),
            Location::Left,
            Modifiers::SHIFT,
            None,
        );
        assert_eq!(none, None);
    }

    #[test]
    fn local_chords() {
        let shift = Modifiers::SHIFT;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;
        let named = |n| Key::Named(n);
        assert!(matches!(
            local_key(&named(Named::PageUp), shift),
            Some(LocalKey::Scroll(Scroll::PageUp))
        ));
        assert!(matches!(
            local_key(&named(Named::End), shift),
            Some(LocalKey::Scroll(Scroll::Bottom))
        ));
        assert!(matches!(
            local_key(&Key::Character("C".into()), ctrl_shift),
            Some(LocalKey::Copy)
        ));
        assert!(matches!(
            local_key(&Key::Character("V".into()), ctrl_shift),
            Some(LocalKey::Paste(PasteSource::Clipboard))
        ));
        assert!(matches!(
            local_key(&named(Named::Insert), shift),
            Some(LocalKey::Paste(PasteSource::Clipboard))
        ));
        assert!(matches!(
            local_key(&Key::Character("L".into()), ctrl_shift),
            Some(LocalKey::Clear)
        ));
        let ctrl = Modifiers::CTRL;
        assert!(matches!(
            local_key(&Key::Character("=".into()), ctrl),
            Some(LocalKey::ZoomIn)
        ));
        assert!(matches!(
            local_key(&Key::Character("-".into()), ctrl),
            Some(LocalKey::ZoomOut)
        ));
        assert!(matches!(
            local_key(&Key::Character("0".into()), ctrl),
            Some(LocalKey::ZoomReset)
        ));
        // Plain PageUp and Ctrl+C go to the shell; zoom chords need Ctrl without Shift; a plain
        // Insert (no Shift) is not a local paste.
        assert!(local_key(&named(Named::PageUp), Modifiers::empty()).is_none());
        assert!(local_key(&Key::Character("c".into()), Modifiers::CTRL).is_none());
        assert!(local_key(&named(Named::PageUp), ctrl_shift).is_none());
        assert!(local_key(&Key::Character("=".into()), ctrl_shift).is_none());
        assert!(local_key(&named(Named::Insert), Modifiers::empty()).is_none());
    }
}
