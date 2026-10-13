//! `fogu-ui`: the iced application (tabs, split container, terminal widget, settings,
//! palette, prompts).
//!
//! One window with one pane running a live local shell, with keyboard input, scrollback, mouse
//! selection + copy, sanitized/bracketed paste (CLIPBOARD, PRIMARY via middle-click), zoom and
//! clear.

mod app;
mod keys;
mod message;
mod metrics;
mod modal;
mod mouse;
mod palette;
mod runs;
mod subscription;
mod terminal_view;

use std::sync::Arc;

use fogu_core::Config;
use fogu_session::SessionRuntime;
use iced::Size;

// Re-exported so callers of `run()` (the `fogu` binary) don't need their own `iced`
// dependency — `iced` stays an implementation detail of this crate.
pub use iced::Result;

/// Runs the Fogu GUI: a window with one pane running the user's default shell, configured by
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
    .window_size(Size::new(1000.0, 650.0))
    .run()
}
