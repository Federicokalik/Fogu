//! The tab bar: a custom title bar with pill tabs, quick actions and window controls.
//!
//! Tabs scroll horizontally (vertical wheel included), keep a per-tab colour dot, and can be
//! renamed inline. The bar itself moves the window; the resize edges are layered above.

use fogu_core::Action;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Id, Row, button, column, container, mouse_area, row, scrollable, space, text, text_input,
    tooltip,
};
use iced::{Alignment, Border, Color, Element, Length};

use crate::context_menu::to_iced;
use crate::controls;
use crate::design::UiTheme;
use crate::drag::{DragMsg, Payload};
use crate::icons::{Icon, icon};
use crate::lifecycle::{warning_detail, warning_title};
use crate::message::{Message, WindowCmd};
use crate::palette::{Look, mix};
use crate::tab::{Tab, TabId};

/// Header width in logical px.
pub const TAB_W: f32 = 200.0;
/// A header plus the 1 px between headers (drag targets in the bar).
pub const TAB_STRIDE: f32 = TAB_W + 1.0;
/// Height of the bar (it is the window's title bar).
/// macOS: room at the bar's left end for the native window buttons (the title bar is hidden).
pub const TRAFFIC_LIGHTS_W: f32 = 80.0;
pub const BAR_H: f32 = 40.0;
/// Titles longer than this many characters end in `…`.
const MAX_TITLE_CHARS: usize = 20;
const TITLE_SIZE: f32 = 13.0;
/// Width of the window's resize edges.
const EDGE: f32 = 5.0;
/// Warning badge colour (amber).
const WARN: Color = Color::from_rgb(0.96, 0.65, 0.14);
/// Id of the inline rename field.
pub const RENAME_INPUT: &str = "fogu-rename-input";
/// Id of the scrollable tab strip.
pub const TAB_STRIP: &str = "fogu-tab-strip";

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
    pub look: Look,
    /// The active tab has an SSH pane: show the SFTP button (`true` = panel open).
    pub sftp: Option<bool>,
    /// The active tab has an SSH pane: show the "Ports" button with this many listening
    /// forwards.
    pub forwards: Option<usize>,
    /// The profile sidebar is open (its toggle button is highlighted).
    pub profiles: bool,
    /// Empty room before the headers ([`TRAFFIC_LIGHTS_W`] on macOS).
    pub inset: f32,
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

fn solid<'a>(color: Color, width: Length, height: f32) -> Element<'a, Message> {
    container(space())
        .width(width)
        .height(height)
        .style(move |_theme: &iced::Theme| container::background(color))
        .into()
}

impl<'a> TabBar<'a> {
    fn header(&self, tab: &'a Tab) -> Element<'a, Message> {
        let look = self.look;
        let (bg, fg) = (look.base, look.fg);
        let active = tab.id == self.active;
        let hovered = self.hovered == Some(tab.id);
        let title: Element<'a, Message> = match self.renaming {
            Some(rename) if rename.tab == tab.id => text_input("Tab title", &rename.value)
                .id(Id::new(RENAME_INPUT))
                .on_input(Message::RenameInput)
                .on_submit(Message::CommitRename)
                .size(TITLE_SIZE)
                .padding([2, 4])
                .style(move |_theme, status| controls::field(look, status))
                .into(),
            _ => text(truncate(tab.title()))
                .size(TITLE_SIZE)
                .color(if active { fg } else { mix(fg, bg, 0.38) })
                .wrapping(text::Wrapping::None)
                .width(Length::Fill)
                .into(),
        };
        let close: Element<'a, Message> = if active || hovered {
            button(icon(Icon::Close, 9.0, if hovered { fg } else { look.dim }))
                .width(20)
                .height(20)
                .padding(0)
                .style(move |_theme: &iced::Theme, status| {
                    let mut style = controls::flat_button(look, status, false);
                    style.border = Border::default().rounded(10);
                    style
                })
                .on_press(Message::CloseTab(tab.id))
                .into()
        } else {
            space().width(20).into()
        };
        let dot_color = tab.color.map_or_else(
            || {
                if active {
                    look.accent
                } else {
                    mix(fg, bg, 0.7)
                }
            },
            to_iced,
        );
        let dot_radius = match look.preset {
            UiTheme::Retro => 0.0,
            UiTheme::Minimal => 1.0,
            UiTheme::Fogu | UiTheme::Fluent => 3.0,
        };
        let dot = container(space().width(6).height(6)).style(move |_theme: &iced::Theme| {
            container::Style {
                background: Some(dot_color.into()),
                border: Border::default().rounded(dot_radius),
                ..container::Style::default()
            }
        });
        let warning = warning_badge(tab);
        let mut line = row![dot].spacing(8).align_y(Alignment::Center);
        if let Some(badge) = warning {
            line = line.push(badge);
        }
        line = line.push(title).push(close);
        let surface = if active && look.preset == UiTheme::Retro {
            mix(look.raised, look.accent, 0.16)
        } else if active {
            mix(look.raised, fg, 0.09)
        } else if hovered {
            mix(look.raised, fg, 0.05)
        } else {
            Color::TRANSPARENT
        };
        let border = if active {
            mix(look.raised, look.accent, 0.55)
        } else {
            Color::TRANSPARENT
        };
        let content = container(line)
            .padding([0.0, look.gutter])
            .height(BAR_H - 8.0)
            .width(Length::Fill)
            .align_y(Alignment::Center)
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(surface.into()),
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: look.radius_small.into(),
                },
                ..container::Style::default()
            });
        let body = container(content).padding([4, 4]).width(TAB_W);
        mouse_area(body)
            .on_press(Message::Drag(DragMsg::Start(Payload::Tab(tab.id))))
            .on_middle_press(Message::CloseTab(tab.id))
            .on_double_click(Message::StartRename(tab.id))
            .on_right_press(Message::OpenTabMenu(tab.id))
            .on_enter(Message::HoverTab(Some(tab.id)))
            .on_exit(Message::LeaveTab(tab.id))
            .into()
    }

    /// The bar: scrollable headers with `+` / `⌄` after them (their empty rest moves the
    /// window), the panel toggles, then the window buttons.
    #[expect(
        clippy::too_many_lines,
        reason = "one title bar layout; tabs, actions and window buttons share its row"
    )]
    pub fn view(self) -> Element<'a, Message> {
        let look = self.look;
        let plus = icon_button(Icon::Plus, 28.0, look, Message::Action(Action::NewTab));
        // Right-click on `+` or the chevron next to it: the profile picker.
        let plus = mouse_area(plus).on_right_press(Message::Action(Action::OpenProfilePicker));
        let picker = icon_button(
            Icon::ChevronDown,
            24.0,
            look,
            Message::Action(Action::OpenProfilePicker),
        );
        let headers = Row::with_children(self.tabs.iter().map(|tab| self.header(tab))).spacing(1);
        let scroller = Scrollbar::new().width(2).scroller_width(2);
        let strip = scrollable(headers)
            .id(Id::new(TAB_STRIP))
            .direction(Direction::Horizontal(scroller))
            .on_scroll(Message::TabStripViewport)
            .width(Length::Fill);
        // As wide as the headers when they fit (so `+` follows the last one), else whatever the
        // bar leaves after `+` and the buttons; the tiny spacer portion takes the rest.
        let strip = container(strip)
            .width(Length::FillPortion(1000))
            .max_width(crate::metrics::cells_f32(self.tabs.len()) * TAB_STRIDE);
        let add =
            container(row![plus, picker].spacing(0).align_y(Alignment::Center)).padding([6, 2]);
        let tabs = row![
            space().width(self.inset),
            strip,
            add,
            space().width(Length::FillPortion(1))
        ]
        .align_y(Alignment::Center)
        .width(Length::Fill);
        // The bar's empty part (tab headers and buttons take their own presses first).
        let tabs = mouse_area(tabs)
            .on_press(Message::Window(WindowCmd::Press))
            .on_release(Message::Window(WindowCmd::Release))
            .on_double_click(Message::Window(WindowCmd::ToggleMaximize))
            .on_right_press(Message::TabBarMenu)
            .on_scroll(Message::TabBarWheel);
        let toggle = |glyph: Icon, on: bool, tip: String, message| {
            let color = if on { look.accent } else { look.fg };
            let button = button(icon(glyph, 16.0, color))
                .width(32)
                .height(28)
                .padding(0)
                .style(move |_theme: &iced::Theme, status| {
                    let mut style = controls::flat_button(look, status, false);
                    if on {
                        style.background = Some(look.accent_soft.into());
                    }
                    style
                })
                .on_press(message);
            let tip = container(text(tip).size(12))
                .padding(4)
                .style(container::rounded_box);
            tooltip(button, tip, tooltip::Position::Bottom)
        };
        let mut bar_row = row![tabs].align_y(Alignment::Center);
        if let Some(count) = self.forwards {
            bar_row = bar_row.push(toggle(
                Icon::Ports,
                count > 0,
                format!("Port forwarding ({count} listening)"),
                Message::Forwards(crate::forwards_dialog::ForwardsMsg::Open),
            ));
        }
        if let Some(open) = self.sftp {
            bar_row = bar_row.push(toggle(
                Icon::Folder,
                open,
                "SFTP panel (toggle-sftp)".to_owned(),
                Message::Action(Action::ToggleSftp),
            ));
        }
        bar_row = bar_row
            .push(toggle(
                Icon::Sidebar,
                self.profiles,
                "Profiles (toggle-profiles)".to_owned(),
                Message::Action(Action::ToggleProfiles),
            ))
            .push(toggle(
                Icon::Settings,
                false,
                "Settings".to_owned(),
                Message::OpenSettings,
            ));
        // macOS: the native title bar has the window buttons.
        if !cfg!(target_os = "macos") {
            bar_row = bar_row
                .push(space().width(6))
                .push(solid(look.border, Length::Fixed(1.0), 20.0))
                .push(space().width(2))
                .push(window_button(
                    Icon::Minimize,
                    look,
                    false,
                    Message::Window(WindowCmd::Minimize),
                ))
                .push(window_button(
                    Icon::Maximize,
                    look,
                    false,
                    Message::Window(WindowCmd::ToggleMaximize),
                ))
                .push(window_button(
                    Icon::Close,
                    look,
                    true,
                    Message::CloseRequested,
                ));
        }
        let bar = container(bar_row)
            .width(Length::Fill)
            .height(BAR_H)
            .style(move |_theme: &iced::Theme| container::background(look.raised));
        let bar = mouse_area(bar).on_move(Message::TabBarCursor);
        column![bar, solid(look.border, Length::Fill, 1.0)].into()
    }
}

/// A flat bar button `width` px wide with a 16 px icon.
fn icon_button<'a>(glyph: Icon, width: f32, look: Look, message: Message) -> Element<'a, Message> {
    button(icon(glyph, 16.0, look.fg))
        .width(width)
        .height(28)
        .padding(0)
        .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
        .on_press(message)
        .into()
}

/// Minimize / maximize / close at the bar's right end (close turns white on red).
fn window_button<'a>(
    glyph: Icon,
    look: Look,
    danger: bool,
    message: Message,
) -> Element<'a, Message> {
    button(icon(glyph, 15.0, look.fg))
        .width(44)
        .height(BAR_H)
        .padding(0)
        .style(move |_theme: &iced::Theme, status| {
            let mut style = controls::flat_button(look, status, danger);
            style.border = Border::default().rounded(0);
            style
        })
        .on_press(message)
        .into()
}

/// Invisible strips along the window's edges and corners that start a resize (there are no OS
/// decorations to do it); layered over everything else.
pub fn resize_edges<'a>() -> Element<'a, Message> {
    use iced::mouse::Interaction;
    use iced::window::Direction as D;
    let edge = |w: Length, h: Length, dir: D, cursor: Interaction| -> Element<'a, Message> {
        mouse_area(space().width(w).height(h))
            .interaction(cursor)
            .on_press(Message::Window(WindowCmd::Resize(dir)))
            .into()
    };
    let (e, fill) = (Length::Fixed(EDGE), Length::Fill);
    let (h, v) = (
        Interaction::ResizingHorizontally,
        Interaction::ResizingVertically,
    );
    let (up, down) = (
        Interaction::ResizingDiagonallyUp,
        Interaction::ResizingDiagonallyDown,
    );
    column![
        row![
            edge(e, e, D::NorthWest, down),
            edge(fill, e, D::North, v),
            edge(e, e, D::NorthEast, up)
        ],
        row![
            edge(e, fill, D::West, h),
            space().width(fill),
            edge(e, fill, D::East, h)
        ]
        .height(fill),
        row![
            edge(e, e, D::SouthWest, up),
            edge(fill, e, D::South, v),
            edge(e, e, D::SouthEast, down)
        ],
    ]
    .into()
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
            truncate(&"Ǹ".repeat(MAX_TITLE_CHARS)).chars().count(),
            MAX_TITLE_CHARS
        );
    }
}
