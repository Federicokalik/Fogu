//! `fogu-ui`: the iced application (tabs, split container, terminal widget, settings,
//! palette, prompts).
//!
//! For now this opens an empty window titled "Fogu".

use iced::widget::{Space, container};
use iced::{Element, Length, Size};

// Re-exported so callers of `run()` (the `fogu` binary) don't need their own `iced`
// dependency — `iced` stays an implementation detail of this crate.
pub use iced::Result;

/// Application state. Application state (empty).
#[derive(Debug, Default)]
struct App;

/// Application messages. Empty until the terminal widget and chrome send events.
#[derive(Debug, Clone, Copy)]
enum Message {}

fn update(_state: &mut App, message: Message) {
    match message {}
}

fn view(_state: &App) -> Element<'_, Message> {
    container(Space::new().width(Length::Fill).height(Length::Fill)).into()
}

/// Runs the Fogu GUI: an iced application that currently shows an empty window.
///
/// # Errors
///
/// Returns an error if iced fails to initialise the windowing or graphics backend.
pub fn run() -> Result {
    iced::application(App::default, update, view)
        .title("Fogu")
        .window_size(Size::new(1000.0, 650.0))
        .run()
}
