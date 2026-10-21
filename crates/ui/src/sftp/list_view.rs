//! The panel's entry list: column headers and a virtualized list (only rows in view are
//! built), with the inline name field for rename / new folder. Every remote string shown here
//! went through `display_name`.

use fogu_session::sftp::Entry;
use iced::widget::{
    Id, Space, column, container, mouse_area, row, scrollable, text, text_input, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Theme};

use super::listing::{self, SortKey, kind_icon, shown_name};
use super::panel::{LIST_ID, Look, ROW_H, SIZE, label, msg, px_rows, rows_px};
use super::{PanelMsg, SftpPanelState};
use crate::message::Message;
use crate::tab::TabId;

/// The inline name field (rename / new folder).
pub const EDIT_INPUT: &str = "fogu-sftp-edit";
/// Rows drawn before the first scroll event says how tall the list is.
const FIRST_ROWS: usize = 60;
/// Names longer than this (in characters) get a tooltip.
const LONG_NAME: usize = 24;
const SIZE_W: f32 = 62.0;
const DATE_W: f32 = 124.0;
const MODE_W: f32 = 82.0;

pub(super) fn column_headers<'a>(
    tab: TabId,
    panel: &SftpPanelState,
    look: Look,
) -> Element<'a, Message> {
    let l = &panel.listing;
    let head = |name: &str, key: SortKey| {
        let arrow = match (l.sort == key, l.descending) {
            (true, false) => " ↑",
            (true, true) => " ↓",
            (false, _) => "",
        };
        let color = if l.sort == key { look.fg } else { look.dim };
        iced::widget::button(
            text(format!("{}{arrow}", name.to_uppercase()))
                .size(10)
                .color(color)
                .wrapping(text::Wrapping::None),
        )
        .padding([3, 2])
        .style(move |_theme: &Theme, status| {
            let mut style = crate::controls::flat_button(look, status, false);
            style.text_color = color;
            style
        })
        .on_press(msg(tab, PanelMsg::Sort(key)))
    };
    row![
        container(head("Name", SortKey::Name)).width(Length::Fill),
        container(head("Size", SortKey::Size)).width(SIZE_W),
        container(head("Modified (UTC)", SortKey::Modified)).width(DATE_W),
        container(head("Perms", SortKey::Mode)).width(MODE_W),
    ]
    .into()
}

/// The entry list, drawing only the rows in view.
pub(super) fn entries(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'_, Message> {
    let list = rows(tab, panel, look);
    match panel.ops.edit.as_ref().filter(|e| e.old.is_none()) {
        // "New folder": the name field sits above the list.
        Some(edit) => {
            let field = row![
                label("\u{25aa}", look.dim).width(16),
                name_field(tab, &edit.value, "New folder name, Enter to create", look),
            ]
            .spacing(4)
            .align_y(Alignment::Center);
            column![container(field).padding([0, 4]), list]
                .spacing(2)
                .into()
        }
        None => list,
    }
}

/// The inline name field (Enter submits, Esc cancels).
fn name_field<'a>(tab: TabId, value: &str, placeholder: &str, look: Look) -> Element<'a, Message> {
    text_input(placeholder, value)
        .id(Id::new(EDIT_INPUT))
        .on_input(move |v| msg(tab, PanelMsg::NameInput(v)))
        .on_submit(msg(tab, PanelMsg::NameSubmit))
        .size(SIZE)
        .padding([1, 3])
        .width(Length::Fill)
        .style(move |_theme: &Theme, status| crate::controls::field(look, status))
        .into()
}

fn rows(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'_, Message> {
    if let Some(error) = &panel.error {
        return container(label(error.clone(), look.dim).wrapping(text::Wrapping::Word))
            .height(Length::Fill)
            .into();
    }
    let visible = panel.listing.visible();
    if visible.is_empty() && panel.loading.is_none() && panel.path.is_some() {
        let hidden = panel.listing.hidden_count();
        let why = if panel.listing.entries.is_empty() {
            "Empty folder".to_owned()
        } else if hidden == panel.listing.entries.len() {
            format!("Only hidden files here ({hidden})")
        } else {
            "Nothing matches the filter".to_owned()
        };
        let mut body = column![label(why, look.dim)].spacing(6);
        if hidden > 0 {
            body = body.push(
                iced::widget::button(label("Show hidden files", look.fg))
                    .padding([2, 8])
                    .on_press(msg(tab, PanelMsg::ShowHidden(true))),
            );
        }
        return container(body).padding(8).height(Length::Fill).into();
    }
    let first = px_rows(panel.scroll_y);
    let count = if panel.view_h > 0.0 {
        px_rows(panel.view_h) + 3
    } else {
        FIRST_ROWS
    };
    let first = first.min(visible.len());
    let end = (first + count).min(visible.len());
    let mut list = column![Space::new().height(rows_px(first))];
    for entry in visible.get(first..end).unwrap_or_default() {
        list = list.push(entry_row(tab, panel, entry, look));
    }
    list = list.push(Space::new().height(rows_px(visible.len() - end)));
    scrollable(list)
        .id(Id::new(LIST_ID))
        .height(Length::Fill)
        .on_scroll(move |v| {
            msg(
                tab,
                PanelMsg::Scrolled(v.absolute_offset().y, v.bounds().height),
            )
        })
        .into()
}

/// The name, clipped; a long one gets a tooltip with the whole (sanitized) name.
fn name_cell<'a>(entry: &Entry, color: Color) -> Element<'a, Message> {
    let name = shown_name(entry);
    let long = name.chars().count() > LONG_NAME;
    let cell = container(label(name.clone(), color))
        .width(Length::Fill)
        .clip(true);
    if !long {
        return cell.into();
    }
    let tip = container(text(name).size(12).wrapping(text::Wrapping::WordOrGlyph))
        .padding(4)
        .max_width(420)
        .style(container::rounded_box);
    tooltip(cell, tip, tooltip::Position::Bottom).into()
}

fn entry_row<'a>(
    tab: TabId,
    panel: &SftpPanelState,
    entry: &Entry,
    look: Look,
) -> Element<'a, Message> {
    let selected = panel.listing.selected.contains(&entry.name);
    let cursor = panel.listing.cursor.as_deref() == Some(entry.name.as_str());
    let name_color = if listing::is_dir(entry) {
        look.accent
    } else {
        look.fg
    };
    let renaming = panel
        .ops
        .edit
        .as_ref()
        .filter(|e| e.old.as_deref() == Some(entry.name.as_str()));
    let name = match renaming {
        Some(edit) => name_field(tab, &edit.value, "New name, Enter to rename", look),
        None => name_cell(entry, name_color),
    };
    let cells = row![
        label(kind_icon(entry), look.dim).width(16),
        name,
        label(listing::human_size(entry), look.fg)
            .width(SIZE_W)
            .align_x(Alignment::End),
        label(listing::modified(entry), look.dim).width(DATE_W),
        label(listing::permissions(entry), look.dim)
            .font(Font::MONOSPACE)
            .width(MODE_W),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    let (fg, accent) = (look.fg, look.accent);
    let body = container(cells)
        .height(ROW_H)
        .padding([0, 4])
        .align_y(Alignment::Center)
        .style(move |_theme: &Theme| container::Style {
            background: selected.then_some(Background::Color(Color { a: 0.16, ..accent })),
            border: Border {
                color: if cursor {
                    Color { a: 0.45, ..fg }
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: look.radius_small.into(),
            },
            ..container::Style::default()
        });
    mouse_area(body)
        .on_press(msg(tab, PanelMsg::Click(entry.name.clone())))
        .on_double_click(msg(tab, PanelMsg::Open(entry.name.clone())))
        .on_right_press(msg(tab, PanelMsg::EntryMenu(entry.name.clone())))
        .into()
}
