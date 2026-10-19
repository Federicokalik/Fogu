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
use super::{PanelMsg, SftpMsg, SftpPanelState};
use crate::message::Message;
use crate::palette::Colors;
use crate::tab::TabId;

/// Height of one list row (keyboard scrolling uses it too).
pub const ROW_H: f32 = 22.0;
/// The entry list's scrollable.
pub const LIST_ID: &str = "fogu-sftp-list";
/// The filter field.
pub const FILTER_INPUT: &str = "fogu-sftp-filter";
/// The typed-path field.
pub const PATH_INPUT: &str = "fogu-sftp-path";
pub(super) const SIZE: f32 = 13.0;

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
pub(super) fn px_rows(px: f32) -> usize {
    (px / ROW_H).floor().clamp(0.0, 1e9) as usize
}

pub(crate) use crate::palette::Look;

pub(super) fn msg(tab: TabId, message: PanelMsg) -> Message {
    Message::Sftp(SftpMsg::Panel(tab, message))
}

/// A flat text button in the panel's colours; `None` = disabled.
pub(crate) fn flat<'a>(
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
    let look = Look::new(colors, crate::design::UiTheme::Fogu, None);
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
        follow_controls(tab, panel, look),
        nav(
            "cd here",
            "Type cd to this folder into the terminal",
            PanelMsg::CdHere,
            has_path
        ),
        flat("×", look, Some(msg(tab, PanelMsg::Close))),
    ]
    .spacing(4)
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
        flat(
            "New folder",
            look,
            has_path.then(|| msg(tab, PanelMsg::NewFolder))
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
        super::list_view::column_headers(tab, panel, look),
        super::list_view::entries(tab, panel, look),
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

/// "Follow terminal" checkbox; when off and the terminal is elsewhere, a "resume following"
/// link.
fn follow_controls(tab: TabId, panel: &SftpPanelState, look: Look) -> Element<'_, Message> {
    let toggle = checkbox(panel.follow)
        .label("follow terminal")
        .text_size(SIZE)
        .size(14)
        .style(move |theme: &Theme, status| {
            let mut style = checkbox::primary(theme, status);
            style.text_color = Some(look.fg);
            style
        })
        .on_toggle(move |on| msg(tab, PanelMsg::Follow(on)));
    let elsewhere = panel
        .shell_cwd
        .as_ref()
        .is_some_and(|cwd| panel.path.as_ref() != Some(cwd));
    if panel.follow || !elsewhere {
        return toggle.into();
    }
    row![
        toggle,
        flat(
            "resume following",
            look,
            Some(msg(tab, PanelMsg::Follow(true)))
        )
    ]
    .spacing(2)
    .align_y(Alignment::Center)
    .into()
}

fn status_text(panel: &SftpPanelState) -> String {
    if panel.loading.is_some() {
        return "Loading…".to_owned();
    }
    let shown = panel.listing.visible().len();
    let selected = panel.listing.selected.len();
    let mut out = format!("{shown} items");
    let hidden = panel.listing.hidden_count();
    if hidden > 0 {
        let _infallible = write!(out, ", {hidden} hidden");
    }
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
