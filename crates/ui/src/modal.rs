//! Modal layers: each dims the view behind it and blocks every event from reaching it.
//! [`modal_layer`] shows a plain title / body / buttons dialog (paste confirmation, close
//! confirmation); [`dialog_layer`] / [`over`] + [`frame`] carry richer content (the SSH prompt
//! dialogs); [`centered`] holds the large settings and SFTP panels.
//!
//! Small dialogs follow the UI style: a centred card, a sheet hanging from the top edge or a
//! full-width command line at the bottom, with their buttons inline or in a footer bar.

use fogu_core::style::DialogPlacement;
use iced::widget::{column, container, mouse_area, opaque, row, space, stack, text};
use iced::{Alignment, Element, Length};

use crate::controls::{self, ButtonKind};
use crate::message::Message;
use crate::palette::{Look, mix};

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

/// Labels of first buttons that destroy something (drawn as danger buttons).
const DESTRUCTIVE: [&str; 5] = ["Close", "Delete", "Discard", "Clear", "Reset"];

/// `modal` as a full-window layer (see [`layer`]). The first button is the default action (a
/// danger button when it destroys something), the others are secondary.
pub fn modal_layer<'a>(modal: Modal, look: Look) -> Element<'a, Message> {
    let mut buttons = row![].spacing(8);
    for (i, b) in modal.buttons.into_iter().enumerate() {
        let role = match i {
            0 if DESTRUCTIVE.contains(&b.label) => ButtonKind::Danger,
            0 => ButtonKind::Primary,
            _ => ButtonKind::Secondary,
        };
        buttons = buttons.push(controls::action(b.label, role, look, Some(b.on_press)));
    }
    let content = column![
        title(modal.title, look),
        text(modal.body).font(look.text.chrome)
    ]
    .spacing(12);
    dialog_layer(
        frame(content.into(), Some(buttons.into()), 380.0, look),
        look,
    )
}

/// A dialog title in the style's heading font.
pub fn title<'a>(label: &str, look: Look) -> Element<'a, Message> {
    let label = if look.text.mono_chrome && look.text.upper {
        label.to_uppercase()
    } else {
        label.to_owned()
    };
    text(label).size(18).font(look.text.strong).into()
}

/// Whether `look` places dialogs in the command bar (the frame then fills the width).
fn command_bar(look: Look) -> bool {
    look.dialog.placement == DialogPlacement::CommandBar
}

/// The dialog box around `content` and its `actions` row, at most `max_width` wide (a command
/// bar takes the full width). With a footer style the actions sit in a separate bar, right
/// aligned; otherwise they follow the content.
pub fn frame<'a>(
    content: Element<'a, Message>,
    actions: Option<Element<'a, Message>>,
    max_width: f32,
    look: Look,
) -> Element<'a, Message> {
    let pad = if command_bar(look) { 12 } else { 16 };
    let body: Element<'a, Message> = match actions {
        Some(actions) if look.dialog.footer => {
            let footer = container(actions)
                .padding([10, pad])
                .width(Length::Fill)
                .align_x(Alignment::End)
                .style(move |_theme: &iced::Theme| {
                    container::background(mix(
                        look.bg,
                        look.fg,
                        if look.dark { 0.04 } else { 0.03 },
                    ))
                });
            column![
                container(content).padding(pad).width(Length::Fill),
                controls::hairline(look.border),
                footer,
            ]
            .into()
        }
        Some(actions) => container(column![content, actions].spacing(14))
            .padding(pad)
            .into(),
        None => container(content).padding(pad).into(),
    };
    let marker: Element<'a, Message> = if command_bar(look) {
        // A prompt glyph down the left edge: the dialog reads as a command line.
        let glyph = if look.text.mono_chrome { ">" } else { "›" };
        container(text(glyph).size(16).font(look.text.mono).color(look.accent))
            .padding([pad, 0])
            .into()
    } else {
        space().width(0).into()
    };
    let framed = container(row![marker, body].spacing(if command_bar(look) { 2 } else { 0 }))
        .style(move |_theme: &iced::Theme| controls::dialog_frame(look));
    if command_bar(look) {
        framed.width(Length::Fill).padding([0, 0]).into()
    } else {
        framed.max_width(max_width).into()
    }
}

/// Places a small `dialog` over `base` where the style puts dialogs; `base` is dimmed and
/// receives no events.
pub fn over<'a>(
    base: Element<'a, Message>,
    dialog: Element<'a, Message>,
    look: Look,
) -> Element<'a, Message> {
    stack![base, dialog_layer(dialog, look)].into()
}

/// A small `dialog` where the style puts dialogs, as a full-window layer (see [`layer`]).
pub fn dialog_layer(dialog: Element<'_, Message>, look: Look) -> Element<'_, Message> {
    let placed = container(dialog).width(Length::Fill).height(Length::Fill);
    let placed = match look.dialog.placement {
        DialogPlacement::Center => placed.center(Length::Fill),
        DialogPlacement::TopSheet => placed.align_x(Alignment::Center).align_y(Alignment::Start),
        DialogPlacement::CommandBar => placed.align_y(Alignment::End),
    };
    layer(placed.into(), look, None)
}

/// A large `panel` (settings, SFTP) centred whatever the style's dialog placement, as a
/// full-window layer. Presses on the panel stay in it; a press around it publishes
/// `on_outside` when given.
pub fn centered(
    panel: Element<'_, Message>,
    look: Look,
    on_outside: Option<Message>,
) -> Element<'_, Message> {
    layer(
        container(opaque(panel)).center(Length::Fill).into(),
        look,
        on_outside,
    )
}

/// `placed` (already positioned in a full-window container) over a scrim that dims the window
/// and keeps every event from what lies below. A press on the scrim publishes `on_outside`
/// when given.
fn layer<'a>(
    placed: Element<'a, Message>,
    look: Look,
    on_outside: Option<Message>,
) -> Element<'a, Message> {
    let scrim = controls::scrim(look);
    let shade = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::background(scrim));
    let shade: Element<'a, Message> = match on_outside {
        Some(message) => mouse_area(shade).on_press(message).into(),
        None => shade.into(),
    };
    stack![opaque(shade), placed].into()
}
