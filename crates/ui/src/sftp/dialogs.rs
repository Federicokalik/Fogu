//! The panel's modal dialogs: "file exists" (`conflict.rs`), delete confirmation and
//! permissions. One at a time, in that order.

use iced::widget::{Space, checkbox, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};

use super::chmod::CLASSES;
use super::ops::delete_text;
use super::panel::msg;
use super::{PanelMsg, SftpPanelState};
use crate::controls::{self, ButtonKind};
use crate::message::Message;
use crate::modal;
use crate::palette::Look;
use crate::tab::TabId;
use fogu_session::display_name;

/// The dialog the panel needs answered, if any.
pub fn dialog(tab: TabId, panel: &SftpPanelState, look: Look) -> Option<Element<'_, Message>> {
    super::conflict::conflict_dialog(tab, panel, look)
        .or_else(|| delete_dialog(tab, panel, look))
        .or_else(|| chmod_dialog(tab, panel, look))
}

fn delete_dialog(tab: TabId, panel: &SftpPanelState, look: Look) -> Option<Element<'_, Message>> {
    let d = panel.ops.delete.as_ref()?;
    let ready = d.waiting.is_empty();
    // No default button: Enter never deletes.
    let buttons = row![
        controls::action(
            "Delete",
            ButtonKind::Danger,
            look,
            ready.then(|| msg(tab, PanelMsg::DeleteConfirm(true)))
        ),
        controls::action(
            "Cancel",
            ButtonKind::Secondary,
            look,
            Some(msg(tab, PanelMsg::DeleteConfirm(false)))
        ),
    ]
    .spacing(8);
    let content = column![
        modal::title("Delete", look),
        text(delete_text(d))
            .font(look.text.chrome)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(12);
    Some(modal::frame(
        content.into(),
        Some(buttons.into()),
        440.0,
        look,
    ))
}

fn chmod_dialog(tab: TabId, panel: &SftpPanelState, look: Look) -> Option<Element<'_, Message>> {
    let d = panel.ops.chmod.as_ref()?;
    let cell = |content: Element<'static, Message>| container(content).width(64);
    let mut grid = column![row![
        cell(Space::new().into()),
        cell(text("Read").into()),
        cell(text("Write").into()),
        cell(text("Execute").into()),
    ]]
    .spacing(6);
    for (class, bits) in CLASSES {
        let mut line = row![cell(text(class).into())].align_y(Alignment::Center);
        for bit in bits {
            let boxed =
                checkbox(d.mode & bit != 0).on_toggle(move |_| msg(tab, PanelMsg::ChmodBit(bit)));
            line = line.push(cell(boxed.into()));
        }
        grid = grid.push(line);
    }
    let valid = d.result().is_some();
    let octal = row![
        text("Octal"),
        text_input("644", &d.octal)
            .on_input(move |v| msg(tab, PanelMsg::ChmodOctal(v)))
            .on_submit_maybe(valid.then(|| msg(tab, PanelMsg::ChmodDone(true))))
            .width(80)
            .padding(4)
            .style(move |_theme: &iced::Theme, status| controls::field(look, status)),
        text(if valid { "" } else { "1\u{2013}4 octal digits" }).size(12),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let buttons = row![
        controls::action(
            "Apply",
            ButtonKind::Primary,
            look,
            valid.then(|| msg(tab, PanelMsg::ChmodDone(true)))
        ),
        controls::action(
            "Cancel",
            ButtonKind::Secondary,
            look,
            Some(msg(tab, PanelMsg::ChmodDone(false)))
        ),
    ]
    .spacing(8);
    let content = column![
        modal::title("Permissions", look),
        text(format!("\u{201c}{}\u{201d}", display_name(&d.name)))
            .wrapping(text::Wrapping::WordOrGlyph),
        grid,
        octal,
    ]
    .spacing(12)
    .width(Length::Shrink);
    Some(modal::frame(
        content.into(),
        Some(buttons.into()),
        420.0,
        look,
    ))
}
