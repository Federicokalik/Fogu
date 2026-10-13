//! Input encoding: GUI-agnostic key events → bytes for the PTY. Pure functions only.

mod keys;
mod mouse;
mod paste;

pub use keys::{KeyCode, KeyInput, Mods, encode_key, encode_text};
pub use mouse::{
    Button, Direction, Disposition, MouseAction, alternate_scroll, encode_focus, encode_mouse,
    mouse_disposition,
};
pub use paste::{PasteOpts, PasteWarning, encode_paste, needs_confirmation};
