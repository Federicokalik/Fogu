//! Drawing of the profile sidebar (stock iced widgets, scheme colours like the SFTP panel):
//! title, search, quick connect, then the sections and rows from [`lines`].

use fogu_core::config::ProfileClick;
use fogu_core::{Action, Config};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Id, button, column, container, mouse_area, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Color, Element, Length, mouse};

use super::handlers::{Launch, ProfilesMsg};
use super::{Line, ProfilesView, QUICK_INPUT, SEARCH_INPUT, entries, lines, now_secs, rows};
use crate::drag::{DragMsg, Payload};
use crate::fuzzy_list::{self, RowEvents, RowLook};
use crate::message::Message;
use crate::palette::Look;

/// Width of the drag handle on the sidebar's left edge.
pub(crate) const HANDLE_W: f32 = 4.0;

fn msg(m: ProfilesMsg) -> Message {
    Message::Profiles(m)
}

/// `body` with the sidebar on its right when open; while its edge is dragged, pointer moves
/// anywhere in the window resize it.
pub(crate) fn with_sidebar<'a>(
    body: Element<'a, Message>,
    view: &'a ProfilesView,
    config: &Config,
    look: Look,
) -> Element<'a, Message> {
    if !view.open {
        return body;
    }
    let edge = look.border;
    let handle = mouse_area(
        container(space())
            .width(HANDLE_W)
            .height(Length::Fill)
            .style(move |_theme: &iced::Theme| container::background(edge)),
    )
    .interaction(mouse::Interaction::ResizingHorizontally)
    .on_press(msg(ProfilesMsg::ResizeStart));
    let all = row![body, handle, sidebar(view, config, look)];
    if !view.resizing {
        return all.into();
    }
    mouse_area(all)
        .on_move(|p| msg(ProfilesMsg::ResizeTo(p.x)))
        .on_release(msg(ProfilesMsg::ResizeEnd))
        .into()
}

/// "Profiles", then "+" (New ▸) and "×" (close the sidebar).
fn title_row<'a>(look: Look) -> Element<'a, Message> {
    let icon = |label: &str, on_press: Message| {
        button(text(label.to_owned()).size(14).color(look.fg))
            .padding([0, 6])
            .style(button::text)
            .on_press(on_press)
    };
    row![
        text("Profiles").size(14).color(look.fg).width(Length::Fill),
        icon("+", msg(ProfilesMsg::NewMenu)),
        icon("×", Message::Action(Action::ToggleProfiles)),
    ]
    .align_y(Alignment::Center)
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
                let open = msg(ProfilesMsg::RowOpen(id.clone(), Launch::Tab));
                let (press, double) = match config.appearance.profiles_click {
                    ProfileClick::Single => (open, None),
                    ProfileClick::Double => (msg(ProfilesMsg::Select(pos)), Some(open)),
                };
                // The press may start a drag onto a pane; released in place it is a click.
                let press = Payload::Profile(id.clone(), Box::new(press));
                let events = RowEvents {
                    press: Some(Message::Drag(DragMsg::Start(press))),
                    double,
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
    let title = title_row(look);
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
        top = top.push(text(err.clone()).size(11).color(look.danger));
    }
    let scroller = Scrollbar::new().width(6).scroller_width(6);
    let list = scrollable(list)
        .direction(Direction::Vertical(scroller))
        .height(Length::Fill);
    let body = column![top, list].spacing(8).padding(8);
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
