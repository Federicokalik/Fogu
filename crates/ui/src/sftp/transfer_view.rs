//! The transfers strip of the SFTP panel and the "file exists" dialog.

use iced::widget::{
    Space, button, checkbox, column, container, progress_bar, row, scrollable, text, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

use super::listing::human_bytes;
use super::panel::{Look, flat, label, msg};
use super::transfers::{ConflictChoice, Direction, Transfer, TransferState, conflict_text};
use super::{PanelMsg, SftpPanelState};
use crate::message::Message;
use crate::modal;
use crate::tab::TabId;

/// The transfers strip: header with collapse / clear, then one block per transfer.
pub(super) fn transfers(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'_, Message> {
    let list = &panel.transfers;
    let active = list.active_count();
    let arrow = if list.collapsed { "▸" } else { "▾" };
    let header = row![
        flat(
            format!("{arrow} Transfers ({active} running)"),
            look,
            Some(msg(tab, PanelMsg::ToggleTransfers))
        ),
        Space::new().width(Length::Fill),
        flat(
            "Clear",
            look,
            (list.items.len() > active).then(|| msg(tab, PanelMsg::ClearTransfers))
        ),
    ]
    .align_y(Alignment::Center);
    let mut strip = column![header].spacing(4);
    if !list.collapsed {
        let rows = column(list.items.iter().rev().map(|t| transfer_row(tab, t, look))).spacing(6);
        strip = strip.push(scrollable(rows).height(Length::Shrink));
    }
    let dim = look.dim;
    container(strip)
        .max_height(220)
        .padding([4, 6])
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                color: Color { a: 0.3, ..dim },
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn transfer_row(tab: TabId, t: &Transfer, look: Look) -> Element<'_, Message> {
    // Finished transfers fade until they are swept away.
    let fg = if t.is_active() { look.fg } else { look.dim };
    let arrow = match t.direction {
        Direction::Download => "↓",
        Direction::Upload => "↑",
    };
    let state = match &t.state {
        TransferState::Planning => "preparing…".to_owned(),
        TransferState::Running if t.rate > 0 => format!("{}/s", human_bytes(t.rate)),
        TransferState::Running => String::new(),
        TransferState::Done if t.failed > 0 && t.done_files == 0 => "failed".to_owned(),
        TransferState::Done if t.failed > 0 => format!("done, {} failed", t.failed),
        TransferState::Done => "done".to_owned(),
        TransferState::Failed(why) => format!("failed: {why}"),
        TransferState::Cancelled => "cancelled".to_owned(),
    };
    let action = if t.is_active() {
        flat("Cancel", look, Some(msg(tab, PanelMsg::Cancel(t.id))))
    } else {
        Space::new().into()
    };
    let top = row![
        label(arrow, look.accent),
        container(label(t.label.as_str(), fg))
            .width(Length::Fill)
            .clip(true),
        action,
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let accent = look.accent;
    let bar = progress_bar(0.0..=1.0, t.fraction())
        .girth(5)
        .style(move |_theme: &Theme| progress_bar::Style {
            background: Background::Color(Color { a: 0.15, ..fg }),
            bar: Background::Color(accent),
            border: Border::default().rounded(2),
        });
    let bytes = format!("{} / {}", human_bytes(t.bytes), human_bytes(t.total));
    let bottom = row![
        label(bytes, fg),
        container(label(state, fg)).width(Length::Fill).clip(true),
    ]
    .spacing(8);
    column![top, bar, bottom].spacing(2).into()
}

/// The "file exists" dialog for the panel's first queued conflict.
pub fn conflict_dialog(tab: TabId, panel: &SftpPanelState) -> Option<Element<'_, Message>> {
    let conflict = panel.conflicts.current()?;
    let direction = panel
        .transfers
        .get(conflict.transfer)
        .map_or(Direction::Download, |t| t.direction);
    let more = panel.conflicts.queue.len() - 1;
    let keep_both = tooltip(
        button(text("Keep both")),
        container(text("Not available yet").size(12))
            .padding(4)
            .style(container::rounded_box),
        tooltip::Position::Bottom,
    );
    let buttons = row![
        button(text("Overwrite")).on_press(msg(tab, PanelMsg::Conflict(ConflictChoice::Overwrite))),
        button(text("Skip")).on_press(msg(tab, PanelMsg::Conflict(ConflictChoice::Skip))),
        keep_both,
    ]
    .spacing(8);
    let mut content = column![
        text("File exists").size(18),
        text(conflict_text(conflict, direction)),
    ]
    .spacing(12);
    if more > 0 {
        content = content.push(
            checkbox(panel.conflicts.apply_all)
                .label(format!("Apply to all ({more} more)"))
                .on_toggle(move |on| msg(tab, PanelMsg::ConflictApplyAll(on))),
        );
    }
    content = content.push(buttons);
    Some(modal::frame(content.into(), 420.0))
}
