//! The one modal overlay: dims the view behind it and blocks every event from reaching it.
//! [`overlay`] shows a plain title / body / buttons dialog (paste confirmation, close
//! confirmation); [`over`] + [`frame`] carry richer content (the SSH prompt dialogs).

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

/// Stacks `modal` over `base`; just `base` when `modal` is `None`.
pub fn overlay(base: Element<'_, Message>, modal: Option<Modal>) -> Element<'_, Message> {
    let Some(modal) = modal else { return base };
    let mut buttons = row![].spacing(8);
    for b in modal.buttons {
        buttons = buttons.push(button(text(b.label)).on_press(b.on_press));
    }
    let content = column![text(modal.title).size(18), text(modal.body), buttons].spacing(12);
    over(base, frame(content.into(), 380.0))
}

/// The dialog box around `content`, at most `max_width` wide.
pub fn frame(content: Element<'_, Message>, max_width: f32) -> Element<'_, Message> {
    container(content)
        .padding(16)
        .max_width(max_width)
        .style(container::rounded_box)
        .into()
}

/// Centers `dialog` over `base`, which is dimmed and receives no events.
pub fn over<'a>(base: Element<'a, Message>, dialog: Element<'a, Message>) -> Element<'a, Message> {
    let backdrop = center(dialog).style(|_theme: &iced::Theme| {
        Color {
            a: 0.5,
            ..Color::BLACK
        }
        .into()
    });
    stack![base, opaque(backdrop)].into()
}
