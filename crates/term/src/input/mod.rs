//! Input encoding: GUI-agnostic key events → bytes for the PTY. Pure functions only.

mod keys;

pub use keys::{KeyCode, KeyInput, Mods, encode_key, encode_text};
