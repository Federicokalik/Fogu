//! The tab bar (stock iced widgets): one header per tab with title, close `×` (hover/active),
//! colour underline; `+` at the end; horizontal scroll on overflow.

use fogu_core::Action;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Id, Row, button, column, container, mouse_area, row, scrollable, space, text, text_input,
    tooltip,
};
use iced::{Alignment, Color, Element, Length};

use crate::context_menu::to_iced;
use crate::lifecycle::{warning_detail, warning_title};
use crate::message::Message;
use crate::palette::{self, Colors};
use crate::tab::{Tab, TabId};

/// Header width in logical px.
const TAB_W: f32 = 200.0;
/// Titles longer than this many characters end in `…`.
const MAX_TITLE_CHARS: usize = 20;
/// Height of the colour underline.
const UNDERLINE_H: f32 = 3.0;
const TITLE_SIZE: f32 = 13.0;
/// Warning badge colour (amber).
const WARN: Color = Color::from_rgb(0.96, 0.65, 0.14);
/// Id of the inline rename field.
pub const RENAME_INPUT: &str = "fogu-rename-input";

/// An inline rename in progress.
#[derive(Clone, Debug)]
pub struct Rename {
    pub tab: TabId,
    pub value: String,
}

/// Everything the bar needs for one `view`.
#[derive(Clone, Copy, Debug)]
pub struct TabBar<'a> {
    pub tabs: &'a [Tab],
    pub active: TabId,
    pub hovered: Option<TabId>,
    pub renaming: Option<&'a Rename>,
    pub colors: &'a Colors,
    /// The active tab has an SSH pane: show the SFTP button (`true` = panel open).
    pub sftp: Option<bool>,
}

/// `title` cut to [`MAX_TITLE_CHARS`] characters, ending in `…` when cut.
pub fn truncate(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title.to_owned();
    }
    let mut out: String = title.chars().take(MAX_TITLE_CHARS - 1).collect();
    out.push('…');
    out
}

/// Linear mix of two colours (`t` = share of `b`).
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

fn solid<'a>(color: Color, width: Length, height: f32) -> Element<'a, Message> {
    container(space())
        .width(width)
        .height(height)
        .style(move |_theme: &iced::Theme| container::background(color))
        .into()
}

impl<'a> TabBar<'a> {
    fn bar_bg(&self) -> Color {
        mix(
            palette::to_iced(self.colors.background()),
            Color::BLACK,
            0.3,
        )
    }

    fn header(&self, tab: &'a Tab) -> Element<'a, Message> {
        let bg = palette::to_iced(self.colors.background());
        let fg = palette::to_iced(self.colors.foreground());
        let active = tab.id == self.active;
        let hovered = self.hovered == Some(tab.id);
        let title: Element<'a, Message> = match self.renaming {
            Some(rename) if rename.tab == tab.id => text_input("Tab title", &rename.value)
                .id(Id::new(RENAME_INPUT))
                .on_input(Message::RenameInput)
                .on_submit(Message::CommitRename)
                .size(TITLE_SIZE)
                .padding(2)
                .into(),
            _ => text(truncate(tab.title()))
                .size(TITLE_SIZE)
                .color(if active { fg } else { mix(fg, bg, 0.4) })
                .wrapping(text::Wrapping::None)
                .width(Length::Fill)
                .into(),
        };
        let close: Element<'a, Message> = if active || hovered {
            button(text("×").size(TITLE_SIZE).color(fg))
                .padding([0, 4])
                .style(button::text)
                .on_press(Message::CloseTab(tab.id))
                .into()
        } else {
            space().width(16).into()
        };
        let title = match warning_badge(tab) {
            Some(badge) => row![badge, title]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
            None => title,
        };
        let underline = tab.color.map_or(Color::TRANSPARENT, to_iced);
        let header_bg = match (active, hovered) {
            (true, _) => bg,
            (false, true) => mix(self.bar_bg(), fg, 0.08),
            (false, false) => self.bar_bg(),
        };
        let content = column![
            container(row![title, close].spacing(6).align_y(Alignment::Center))
                .padding([5, 10])
                .width(Length::Fill),
            solid(underline, Length::Fill, UNDERLINE_H),
        ];
        let body = container(content)
            .width(TAB_W)
            .style(move |_theme: &iced::Theme| container::background(header_bg));
        mouse_area(body)
            .on_press(Message::ActivateTab(tab.id))
            .on_middle_press(Message::CloseTab(tab.id))
            .on_double_click(Message::StartRename(tab.id))
            .on_right_press(Message::OpenTabMenu(tab.id))
            .on_enter(Message::HoverTab(Some(tab.id)))
            .on_exit(Message::HoverTab(None))
            .into()
    }

    /// The bar: scrollable headers, then `+`.
    pub fn view(self) -> Element<'a, Message> {
        let headers = Row::with_children(self.tabs.iter().map(|tab| self.header(tab))).spacing(1);
        let scroller = Scrollbar::new().width(2).scroller_width(2);
        let tabs = scrollable(headers)
            .direction(Direction::Horizontal(scroller))
            .width(Length::Fill);
        let fg = palette::to_iced(self.colors.foreground());
        let plus = button(text("+").size(16).color(fg))
            .padding([2, 12])
            .style(button::text)
            .on_press(Message::Action(Action::NewTab));
        // Right-click on `+` or the `▾` next to it: the profile picker.
        let plus = mouse_area(plus).on_right_press(Message::Action(Action::OpenProfilePicker));
        let profiles = button(text("▾").size(13).color(fg))
            .padding([2, 8])
            .style(button::text)
            .on_press(Message::Action(Action::OpenProfilePicker));
        let bar_bg = self.bar_bg();
        let mut bar_row = row![tabs, plus, profiles].align_y(Alignment::Center);
        if let Some(open) = self.sftp {
            let color = if open {
                to_iced(fogu_core::RgbColor::from_hex(0x00_90FF))
            } else {
                fg
            };
            let sftp = button(text("SFTP").size(12).color(color))
                .padding([2, 8])
                .style(button::text)
                .on_press(Message::Action(Action::ToggleSftp));
            let tip = container(text("SFTP panel (toggle-sftp)").size(12))
                .padding(4)
                .style(container::rounded_box);
            bar_row = bar_row.push(tooltip(sftp, tip, tooltip::Position::Bottom));
        }
        let bar = container(bar_row)
            .width(Length::Fill)
            .style(move |_theme: &iced::Theme| container::background(bar_bg));
        mouse_area(bar).on_move(Message::TabBarCursor).into()
    }
}

/// A persistent `!` badge when a pane of `tab` reported a warning (e.g. host key not
/// verified); the tooltip says what.
fn warning_badge(tab: &Tab) -> Option<Element<'_, Message>> {
    let lines: Vec<String> = tab
        .warnings()
        .map(|w| format!("{}: {}", warning_title(w), warning_detail(w)))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let badge = container(text("!").size(11).color(Color::BLACK))
        .padding([0, 5])
        .style(|theme: &iced::Theme| container::rounded_box(theme).background(WARN));
    let tip = container(text(lines.join("\n")).size(12))
        .padding(6)
        .max_width(320)
        .style(container::rounded_box);
    Some(tooltip(badge, tip, tooltip::Position::Bottom).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_titles_with_ellipsis() {
        assert_eq!(truncate("zsh"), "zsh");
        let long = "a".repeat(40);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MAX_TITLE_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(
            truncate(&"é".repeat(MAX_TITLE_CHARS)).chars().count(),
            MAX_TITLE_CHARS
        );
    }
}
