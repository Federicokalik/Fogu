//! `fogu-core`: pure config/model crate (config schema, profiles, `SplitTree`, colour
//! schemes, keymap parsing, `Action` enum, recovery-state model). No tokio, no GUI, no I/O
//! (except [`config::load`]/[`config::set_profile_background`], plain `std::fs`).

mod action;
mod background;
pub mod config;
pub mod fsutil;
mod keychord;
mod profile;
mod quick_connect;
mod scheme;
mod size;
mod split;

pub use action::{Action, Dir, Hotkeys};
pub use background::{Background, EffectiveBackground, Fit, TabBackground, effective_background};
pub use config::Config;
pub use config::ssh_opts::{
    Algorithms, Backspace, EffectiveSsh, ForwardKind, LoginScript, PortForward, SshAuth, SshOpts,
};
pub use keychord::{KeyChord, KeyName, Mods, NamedKey};
pub use profile::{
    EffectiveProfile, Group, GroupId, LocalOpts, LocalProfile, Osc52Policy, Profile,
    ProfileDefaults, ProfileId, ProfileKind, SessionEndBehavior, effective_osc52_write, resolve,
};
pub use quick_connect::{QuickConnect, QuickConnectError};
pub use scheme::{ColorScheme, RgbColor};
pub use size::Size;
pub use split::{Divider, Node, NodePath, Orientation, Rect, Side, SplitTree};
