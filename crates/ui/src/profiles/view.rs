//! Drawing of the profile sidebar (stock iced widgets, scheme colours like the SFTP panel):
//! title, search, quick connect, then the sections and rows from [`lines`].

use fogu_core::{Action, Config};
use iced::widget::{
    Id, button, column, container, mouse_area, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Color, Element, Length, mouse};

use super::handlers::{Launch, ProfilesMsg};
use super::{Line, ProfilesView, QUICK_INPUT, SEARCH_INPUT, entries, lines, now_secs, rows};
use crate::fuzzy_list::{self, RowEvents, RowLook};
use crate::message::Message;
use crate::palette::Colors;
use crate::sftp::panel::Look;

/// Width of the drag handle on the sidebar's right edge.
pub(crate) const HANDLE_W: f32 = 4.0;

fn msg(m: ProfilesMsg) -> Message {
    Message::Profiles(m)
}

/// `body` with the sidebar on its left when open; while its edge is dragged, pointer moves
/// anywhere in the window resize it.
pub(crate) fn with_sidebar<'a>(
    body: Element<'a, Message>,
    view: &'a ProfilesView,
    config: &Config,
    colors: &Colors,
) -> Element<'a, Message> {
    if !view.open {
        return body;
    }
    let look = Look::new(colors);
    let edge = look.dim;
    let handle = mouse_area(
        container(space())
            .width(HANDLE_W)
            .height(Length::Fill)
            .style(move |_theme: &iced::Theme| container::background(Color { a: 0.4, ..edge })),
    )
    .interaction(mouse::Interaction::ResizingHorizontally)
    .on_press(msg(ProfilesMsg::ResizeStart));
    let all = row![sidebar(view, config, look), handle, body];
    if !view.resizing {
        return all.into();
    }
    mouse_area(all)
        .on_move(|p| msg(ProfilesMsg::ResizeTo(p.x)))
        .on_release(msg(ProfilesMsg::ResizeEnd))
        .into()
}

fn sidebar<'a>(view: &'a ProfilesView, config: &Config, look: Look) -> Element<'a, Message> {
    let all = entries(
        config,
        &view.builtins,
        &view.ssh_hosts.profiles,
        Some(&view.recent),
    );
    let rows = rows(&all, config, &view.recent, now_secs());
    let lines = lines(view, config, &all, &rows);
    let row_look = RowLook {
        fg: look.fg,
        dim: look.dim,
        selected: Color {
            a: 0.35,
            ..look.accent
        },
    };
    let mut list = column![].spacing(1);
    let mut pos = 0;
    for line in lines {
        match line {
            Line::Header {
                section,
                label,
                count,
                collapsed,
            } => {
                let arrow = if collapsed { "▸" } else { "▾" };
                let header = button(
                    text(format!("{arrow} {label} ({count})"))
                        .size(12)
                        .color(look.dim),
                )
                .width(Length::Fill)
                .padding([4, 4])
                .style(button::text)
                .on_press(msg(ProfilesMsg::Toggle(section)));
                list = list.push(header);
            }
            Line::Item(index) => {
                let (Some(entry), Some(item)) = (all.get(index), rows.get(index)) else {
                    continue;
                };
                let id = entry.profile.id.clone();
                let events = RowEvents {
                    press: Some(msg(ProfilesMsg::Select(pos))),
                    double: Some(msg(ProfilesMsg::RowOpen(id.clone(), Launch::Tab))),
                    middle: Some(msg(ProfilesMsg::RowOpen(id.clone(), Launch::Split))),
                    right: Some(msg(ProfilesMsg::Menu(id, pos))),
                };
                let selected = view.focused && pos == view.selected;
                list = list.push(fuzzy_list::row_view(item, selected, row_look, events));
                pos += 1;
            }
        }
    }
    if pos == 0 && !view.query.trim().is_empty() {
        list = list.push(text("No matching profile").size(12).color(look.dim));
    }
    let close = button(text("×").size(14).color(look.fg))
        .padding([0, 6])
        .style(button::text)
        .on_press(Message::Action(Action::ToggleProfiles));
    let title = row![
        text("Profiles").size(14).color(look.fg).width(Length::Fill),
        close
    ]
    .align_y(Alignment::Center);
    let search = text_input("Search profiles", &view.query)
        .id(Id::new(SEARCH_INPUT))
        .on_input(|q| msg(ProfilesMsg::Query(q)))
        .on_submit(msg(ProfilesMsg::Submit))
        .size(13)
        .padding(5);
    let quick = text_input("Quick connect: user@host:port", &view.quick)
        .id(Id::new(QUICK_INPUT))
        .on_input(|q| msg(ProfilesMsg::Quick(q)))
        .on_submit(msg(ProfilesMsg::QuickSubmit))
        .size(13)
        .padding(5);
    let mut top = column![title, search, quick].spacing(6);
    if let Some(err) = &view.quick_error {
        top = top.push(
            text(err.clone())
                .size(11)
                .color(Color::from_rgb8(0xE5, 0x48, 0x4D)),
        );
    }
    let body = column![top, scrollable(list).height(Length::Fill)]
        .spacing(8)
        .padding(8);
    let bg = look.bg;
    let panel = container(body)
        .width(view.width)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::background(bg));
    mouse_area(panel)
        .on_enter(msg(ProfilesMsg::Hover(true)))
        .on_exit(msg(ProfilesMsg::Hover(false)))
        .on_move(|p| msg(ProfilesMsg::Cursor(p)))
        .into()
}
