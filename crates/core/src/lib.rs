//! `fogu-core`: pure config/model crate (config schema, profiles, `SplitTree`, colour
//! schemes, keymap parsing, `Action` enum, recovery-state model). No tokio, no GUI, no I/O.

mod profile;
mod scheme;
mod size;

pub use profile::LocalProfile;
pub use scheme::{ColorScheme, RgbColor};
pub use size::Size;
