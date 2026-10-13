//! `fogu-ui`: the iced application (tabs, split container, terminal widget, settings,
//! palette, prompts).
//!
//! One window with tabs of split panes running local shells: keyboard input through the keymap,
//! scrollback, mouse selection + copy, sanitized/bracketed paste, zoom, clear, tab rename/colour,
//! a tab context menu and mouse-resizable splits.

mod actions;
mod app;
mod context_menu;
mod keys;
mod message;
mod metrics;
mod modal;
mod mouse;
mod palette;
mod panes;
mod runs;
mod split_container;
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

/// Runs the Fogu GUI: a window whose first tab runs the default profile, configured by
/// `config` (loaded by the `fogu` binary via `fogu_core::config::load`).
///
/// # Errors
///
/// Returns an error if the session runtime cannot start or iced fails to initialise the
/// windowing or graphics backend.
pub fn run(config: Arc<Config>) -> Result {
    let runtime = Arc::new(SessionRuntime::new().map_err(iced::Error::ExecutorCreationFailed)?);
    iced::application(
        move || app::App::boot(Arc::clone(&runtime), Arc::clone(&config)),
        app::App::update,
        app::App::view,
    )
    .title(app::App::title)
    .subscription(app::App::subscription)
    .window_size(app::WINDOW_SIZE)
    .run()
}
