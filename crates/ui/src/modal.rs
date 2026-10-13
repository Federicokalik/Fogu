//! A tiny modal overlay: dims the view behind it and shows a title, body text and buttons that
//! each publish a `Message` when clicked. Kept generic (not paste-specific) so later prompts
//! (such as host-key dialogs) can reuse it.

use iced::widget::{button, center, column, container, opaque, row, stack, text};
use iced::{Color, Element};

use crate::message::Message;

/// One modal button; clicking it publishes `on_press`.
#[derive(Debug)]
pub struct ModalButton {
    /// Button label.
    pub label: &'static str,
    /// Message published on click.
    pub on_press: Message,
}

/// A modal's content: a title, body text, and buttons left to right.
#[derive(Debug)]
pub struct Modal {
    /// Title line.
    pub title: &'static str,
    /// Body text.
    pub body: String,
    /// Buttons, left to right.
    pub buttons: Vec<ModalButton>,
}

/// Stacks `modal` over `base`, dimmed and blocking every event from reaching it; just `base`
/// when `modal` is `None`.
pub fn overlay(base: Element<'_, Message>, modal: Option<Modal>) -> Element<'_, Message> {
    let Some(modal) = modal else { return base };
    let mut buttons = row![].spacing(8);
    for b in modal.buttons {
        buttons = buttons.push(button(text(b.label)).on_press(b.on_press));
    }
    let dialog =
        container(column![text(modal.title).size(18), text(modal.body), buttons].spacing(12))
            .padding(16)
            .max_width(380.0)
            .style(container::rounded_box);
    let backdrop = center(dialog).style(|_theme: &iced::Theme| {
        Color {
            a: 0.5,
            ..Color::BLACK
        }
        .into()
    });
    stack![base, opaque(backdrop)].into()
}
