//! Drawing of the SFTP panel (stock iced widgets, scheme colours): header, toolbar, breadcrumb,
//! column headers, a virtualized entry list, and the transfers strip; plus the conflict dialog.
//! Every remote string shown here went through `display_name`.

use std::fmt::Write as _;

use iced::widget::{
    Id, Space, button, checkbox, column, container, mouse_area, row, scrollable, text, text_input,
    tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Theme};

use super::breadcrumb::segments;
use super::listing::{self, SortKey, kind_icon, shown_name};
use super::{PanelMsg, SftpMsg, SftpPanelState};
use crate::message::Message;
use crate::palette::{self, Colors};
use crate::tab::TabId;

/// Height of one list row (keyboard scrolling uses it too).
pub const ROW_H: f32 = 22.0;
/// The entry list's scrollable.
pub const LIST_ID: &str = "fogu-sftp-list";
/// The filter field.
pub const FILTER_INPUT: &str = "fogu-sftp-filter";
/// The typed-path field.
pub const PATH_INPUT: &str = "fogu-sftp-path";
const SIZE: f32 = 13.0;
/// Rows drawn before the first scroll event says how tall the list is.
const FIRST_ROWS: usize = 60;
/// Names longer than this (in characters) get a tooltip.
const LONG_NAME: usize = 24;
const SIZE_W: f32 = 62.0;
const DATE_W: f32 = 124.0;
const MODE_W: f32 = 82.0;

/// Height in pixels of `rows` list rows.
#[expect(
    clippy::cast_precision_loss,
    reason = "row counts stay far below 2^24, where f32 is exact"
)]
pub fn rows_px(rows: usize) -> f32 {
    rows as f32 * ROW_H
}

/// Whole rows in `px` pixels (negative = 0).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "floored and clamped to 0..=1e9 before the cast"
)]
fn px_rows(px: f32) -> usize {
    (px / ROW_H).floor().clamp(0.0, 1e9) as usize
}

/// The colours the panel draws with (from the terminal scheme).
#[derive(Clone, Copy, Debug)]
pub(super) struct Look {
    pub(super) bg: Color,
    pub(super) fg: Color,
    pub(super) dim: Color,
    pub(super) accent: Color,
}

impl Look {
    pub(super) fn new(colors: &Colors) -> Self {
        let bg = palette::to_iced(colors.background());
        let fg = palette::to_iced(colors.foreground());
        Self {
            bg: mix(bg, fg, 0.06),
            fg,
            dim: mix(fg, bg, 0.45),
            // The scheme's bright blue: directories, selection, progress.
            accent: palette::to_iced(colors.resolve(
                fogu_term::Color::Named(fogu_term::NamedColor::BrightBlue),
                false,
            )),
        }
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

pub(super) fn msg(tab: TabId, message: PanelMsg) -> Message {
    Message::Sftp(SftpMsg::Panel(tab, message))
}

/// A flat text button in the panel's colours; `None` = disabled.
pub(super) fn flat<'a>(
    label: impl Into<String>,
    look: Look,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let fg = look.fg;
    button(text(label.into()).size(SIZE))
        .padding([2, 6])
        .style(move |_theme: &Theme, status| {
            let hover = Color { a: 0.14, ..fg };
            button::Style {
                background: match status {
                    button::Status::Hovered | button::Status::Pressed => {
                        Some(Background::Color(hover))
                    }
                    _ => None,
                },
                text_color: match status {
                    button::Status::Disabled => Color { a: 0.35, ..fg },
                    _ => fg,
                },
                border: Border::default().rounded(3),
                ..button::Style::default()
            }
        })
        .on_press_maybe(on_press)
        .into()
}

pub(super) fn label<'a>(value: impl text::IntoFragment<'a>, color: Color) -> text::Text<'a> {
    text(value)
        .size(SIZE)
        .color(color)
        .wrapping(text::Wrapping::None)
}

/// The whole panel of tab `tab`.
pub fn view<'a>(tab: TabId, panel: &'a SftpPanelState, colors: &Colors) -> Element<'a, Message> {
    let look = Look::new(colors);
    let has_path = panel.path.is_some();
    let nav = |label: &str, tip: &'a str, m: PanelMsg, enabled: bool| {
        tooltip(
            flat(label.to_owned(), look, enabled.then(|| msg(tab, m))),
            container(text(tip).size(12))
                .padding(4)
                .style(container::rounded_box),
            tooltip::Position::Bottom,
        )
    };
    let header = row![
        label("SFTP", look.fg).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        Space::new().width(Length::Fill),
        flat("×", look, Some(msg(tab, PanelMsg::Close))),
    ]
    .align_y(Alignment::Center);
    let toolbar = row![
        nav("←", "Back", PanelMsg::Back, !panel.back.is_empty()),
        nav("↑", "Up", PanelMsg::Up, has_path),
        nav("⌂", "Home", PanelMsg::Home, true),
        nav("⟳", "Refresh", PanelMsg::Refresh, true),
        text_input("Filter", &panel.listing.filter)
            .id(Id::new(FILTER_INPUT))
            .on_input(move |f| msg(tab, PanelMsg::Filter(f)))
            .size(SIZE)
            .padding(3)
            .width(Length::Fill),
        checkbox(panel.listing.show_hidden)
            .label("hidden")
            .text_size(SIZE)
            .size(14)
            .style(move |theme: &Theme, status| {
                let mut style = checkbox::primary(theme, status);
                style.text_color = Some(look.fg);
                style
            })
            .on_toggle(move |on| msg(tab, PanelMsg::ShowHidden(on))),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    let actions = row![
        flat(
            "Upload…",
            look,
            has_path.then(|| msg(tab, PanelMsg::Upload))
        ),
        flat(
            "Download…",
            look,
            (!panel.listing.selected.is_empty()).then(|| msg(tab, PanelMsg::Download))
        ),
        Space::new().width(Length::Fill),
        label(status_text(panel), look.dim),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    let mut body = column![
        header,
        toolbar,
        breadcrumb(tab, panel, look),
        actions,
        column_headers(tab, panel, look),
        entries(tab, panel, look),
    ]
    .spacing(4);
    if !panel.transfers.items.is_empty() {
        body = body.push(super::transfer_view::transfers(tab, panel, look));
    }
    let bg = look.bg;
    let content = container(body)
        .padding(6)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::background(bg));
    mouse_area(content)
        .on_enter(msg(tab, PanelMsg::Hover(true)))
        .on_exit(msg(tab, PanelMsg::Hover(false)))
        .into()
}

fn status_text(panel: &SftpPanelState) -> String {
    if panel.loading.is_some() {
        return "Loading…".to_owned();
    }
    let shown = panel.listing.visible().len();
    let selected = panel.listing.selected.len();
    let mut out = format!("{shown} items");
    if panel.listing.truncated {
        out.push_str(" (truncated)");
    }
    if selected > 0 {
        let _infallible = write!(out, ", {selected} selected");
    }
    out
}

/// Clickable segments; a click on the empty bar switches to a path field.
fn breadcrumb(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'_, Message> {
    if let Some(value) = &panel.editing {
        return text_input("/path/to/dir", value)
            .id(Id::new(PATH_INPUT))
            .on_input(move |v| msg(tab, PanelMsg::PathInput(v)))
            .on_submit(msg(tab, PanelMsg::PathSubmit))
            .size(SIZE)
            .padding(3)
            .into();
    }
    let mut parts = row![].align_y(Alignment::Center);
    let segs = panel.path.as_deref().map(segments).unwrap_or_default();
    for (i, seg) in segs.into_iter().enumerate() {
        if i > 1 {
            parts = parts.push(label("/", look.dim));
        }
        parts = parts.push(flat(
            seg.label,
            look,
            Some(msg(tab, PanelMsg::Go(seg.path))),
        ));
    }
    let dim = look.dim;
    let bar = container(
        // Anchored right: a deep path shows its last segments.
        scrollable(parts)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(2).scroller_width(2),
            ))
            .anchor_right(),
    )
    .width(Length::Fill)
    .padding([1, 2])
    .style(move |_theme: &Theme| container::Style {
        border: Border {
            color: Color { a: 0.4, ..dim },
            width: 1.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    });
    mouse_area(bar)
        .on_press(msg(tab, PanelMsg::EditPath))
        .into()
}

fn column_headers<'a>(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'a, Message> {
    let l = &panel.listing;
    let head = |name: &str, key: SortKey| {
        let arrow = match (l.sort == key, l.descending) {
            (true, false) => " ▲",
            (true, true) => " ▼",
            (false, _) => "",
        };
        flat(
            format!("{name}{arrow}"),
            look,
            Some(msg(tab, PanelMsg::Sort(key))),
        )
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
fn entries(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'_, Message> {
    if let Some(error) = &panel.error {
        return container(label(error.clone(), look.dim).wrapping(text::Wrapping::Word))
            .height(Length::Fill)
            .into();
    }
    let visible = panel.listing.visible();
    if visible.is_empty() && panel.loading.is_none() && panel.path.is_some() {
        let why = if panel.listing.entries.is_empty() {
            "Empty folder"
        } else {
            "Nothing matches (filter / hidden files)"
        };
        return container(label(why, look.dim))
            .padding(8)
            .height(Length::Fill)
            .into();
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
fn name_cell<'a>(entry: &fogu_session::sftp::Entry, color: Color) -> Element<'a, Message> {
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
    entry: &fogu_session::sftp::Entry,
    look: Look,
) -> Element<'a, Message> {
    let selected = panel.listing.selected.contains(&entry.name);
    let cursor = panel.listing.cursor.as_deref() == Some(entry.name.as_str());
    let name_color = if listing::is_dir(entry) {
        look.accent
    } else {
        look.fg
    };
    let cells = row![
        label(kind_icon(entry), look.dim).width(16),
        name_cell(entry, name_color),
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
            background: selected.then_some(Background::Color(Color { a: 0.28, ..accent })),
            border: Border {
                color: if cursor {
                    Color { a: 0.5, ..fg }
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        });
    mouse_area(body)
        .on_press(msg(tab, PanelMsg::Click(entry.name.clone())))
        .on_double_click(msg(tab, PanelMsg::Open(entry.name.clone())))
        .into()
}
