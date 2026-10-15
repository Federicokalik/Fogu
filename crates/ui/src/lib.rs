//! `fogu-ui`: the iced application (tabs, split container, terminal widget, settings,
//! palette, prompts).
//!
//! One window with tabs of split panes running local shells: keyboard input through the keymap,
//! scrollback, mouse selection + copy, sanitized/bracketed paste, zoom, clear, tab rename/colour,
//! a tab context menu and mouse-resizable splits.

mod actions;
mod app;
mod appearance;
mod blink;
mod context_menu;
mod keys;
mod lifecycle;
mod message;
mod metrics;
mod modal;
mod mouse;
mod notifications;
mod palette;
mod panes;
mod paste;
mod runs;
mod split_container;
mod ssh_notices;
mod subscription;
mod tab;
mod tab_bar;
mod terminal_view;

use std::sync::Arc;

use fogu_core::Config;
use fogu_session::SessionRuntime;

// Re-exported so callers of `run()` (the `fogu` binary) don't need their own `iced`
// dependency — `iced` stays an implementation detail of this crate.
pub use iced::Result;

pub use notifications::{Level, Notification};

/// What the binary hands to [`run`] besides the config.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    /// Toasts to show at startup (config load errors, validation issues).
    pub notices: Vec<Notification>,
    /// Exit right after the first presented frame (benchmarks).
    pub exit_after_first_frame: bool,
}

/// `WM_CLASS` / Wayland app id; matches the `.desktop` file's `StartupWMClass`.
const APP_ID: &str = "fogu";
/// Window icon: `assets/fogu.png` at 64×64 as raw RGBA (`convert -resize 64x64 -depth 8
/// rgba:…`), so no image decoder is needed.
const ICON_RGBA: &[u8] = include_bytes!("../../../assets/icons/fogu-64.rgba");
const ICON_SIZE: u32 = 64;

/// Runs the Fogu GUI: a window whose first tab runs the default profile, configured by
/// `config` (loaded by the `fogu` binary via `fogu_core::config::load`).
///
/// # Errors
///
/// Returns an error if the session runtime cannot start or iced fails to initialise the
/// windowing or graphics backend.
pub fn run(config: Arc<Config>, options: RunOptions) -> Result {
    let runtime = Arc::new(SessionRuntime::new().map_err(iced::Error::ExecutorCreationFailed)?);
    let window = iced::window::Settings {
        size: app::WINDOW_SIZE,
        icon: iced::window::icon::from_rgba(ICON_RGBA.to_vec(), ICON_SIZE, ICON_SIZE).ok(),
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: APP_ID.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    };
    iced::application(
        move || app::App::boot(Arc::clone(&runtime), Arc::clone(&config), options.clone()),
        app::App::update,
        app::App::view,
    )
    .title(app::App::title)
    .subscription(app::App::subscription)
    .window(window)
    .run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_is_a_valid_rgba_square() {
        assert!(
            iced::window::icon::from_rgba(
                super::ICON_RGBA.to_vec(),
                super::ICON_SIZE,
                super::ICON_SIZE
            )
            .is_ok()
        );
    }
}
