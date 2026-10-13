//! Input encoding: GUI-agnostic key events → bytes for the PTY. Pure functions only.

mod keys;
mod paste;

pub use keys::{KeyCode, KeyInput, Mods, encode_key, encode_text};
pub use paste::{PasteOpts, PasteWarning, encode_paste, needs_confirmation};
