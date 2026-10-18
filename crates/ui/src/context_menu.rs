//! The one generic context-menu overlay: a `Vec` of
//! items at an anchor point, closed on click-away / Esc. Items map to an [`Action`] (same enum as
//! hotkeys) or to a UI-only [`Message`]. Later menus add builders, not a new widget.

use fogu_core::{Action, RgbColor};
use iced::widget::{
    Id, button, column, container, mouse_area, opaque, row, rule, slider, space, stack, text,
    text_input, tooltip,
};
use iced::{Alignment, Border, Color, Element, Length, Padding, Point, Size};

use crate::message::Message;
use crate::palette::Look;

/// Menu width in logical px.
const MENU_W: f32 = 220.0;
/// Estimated row height, used only to keep the menu inside the window.
const ROW_H: f32 = 28.0;
/// Id of the menu's text field.
pub const MENU_INPUT: &str = "fogu-menu-input";

/// Tab colour presets (Radix-like mid tones, readable on dark and light schemes).
pub const PRESET_COLORS: [(&str, u32); 8] = [
    ("Red", 0xE5_484D),
    ("Orange", 0xF7_6B15),
    ("Yellow", 0xFF_C53D),
    ("Green", 0x30_A46C),
    ("Teal", 0x12_A594),
    ("Blue", 0x00_90FF),
    ("Purple", 0x8E_4EC6),
    ("Pink", 0xD6_409F),
];

/// What choosing an item does.
#[derive(Clone, Debug)]
pub enum MenuAction {
    /// Run a keymap action (on the active tab/pane).
    Run(Action),
    /// A UI-only message (open a sub-menu, set a colour, …).
    Ui(Message),
}

impl MenuAction {
    fn message(&self) -> Message {
        match self {
            Self::Run(action) => Message::Action(*action),
            Self::Ui(message) => message.clone(),
        }
    }
}

/// One menu row.
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub action: MenuAction,
    pub enabled: bool,
    /// Draw a separator line above this item.
    pub separator: bool,
    /// Colour square shown before the label.
    pub swatch: Option<RgbColor>,
    /// Hint shown on hover (e.g. why the item is disabled).
    pub tooltip: Option<&'static str>,
    /// A toggle: drawn with a check mark when `Some(true)`.
    pub checked: Option<bool>,
}

impl MenuItem {
    /// An enabled item without separator or swatch.
    pub fn new(label: impl Into<String>, action: MenuAction) -> Self {
        Self {
            label: label.into(),
            action,
            enabled: true,
            separator: false,
            swatch: None,
            tooltip: None,
            checked: None,
        }
    }

    /// A greyed-out item; `why` is shown on hover.
    #[must_use]
    pub fn disabled(mut self, why: &'static str) -> Self {
        self.enabled = false;
        self.tooltip = Some(why);
        self
    }

    /// Enabled only when `on`; `why` explains otherwise.
    #[must_use]
    pub fn enabled_if(self, on: bool, why: &'static str) -> Self {
        if on { self } else { self.disabled(why) }
    }

    /// With a separator line above it.
    #[must_use]
    pub fn after_separator(mut self) -> Self {
        self.separator = true;
        self
    }

    /// A toggle in state `on`.
    #[must_use]
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
}

/// What Enter in the menu's field does.
#[derive(Clone, Copy, Debug)]
pub enum MenuSubmit {
    /// Parse a `#RRGGBB` colour ("Custom hex…"); anything else marks the field.
    Color(fn(RgbColor) -> Message),
    /// Send the trimmed text when not empty ("New group…").
    Text(fn(String) -> Message),
}

impl MenuSubmit {
    /// The message for `value`, or `None` when it is not acceptable.
    pub fn message(self, value: &str) -> Option<Message> {
        match self {
            Self::Color(pick) => parse_color(value).map(pick),
            Self::Text(send) => {
                let text = value.trim();
                (!text.is_empty()).then(|| send(text.to_owned()))
            }
        }
    }
}

/// A single-line field at the bottom of the menu ("Custom hex…", "New group…").
#[derive(Clone, Debug)]
pub struct MenuInput {
    pub placeholder: &'static str,
    pub value: String,
    /// The last submit was rejected.
    pub invalid: bool,
    pub submit: MenuSubmit,
}

/// A labelled slider at the bottom of the menu (background opacity, in percent).
#[derive(Clone, Debug)]
pub struct MenuSlider {
    pub label: String,
    pub range: std::ops::RangeInclusive<f32>,
    pub value: f32,
    pub on_change: fn(f32) -> Message,
}

/// An open context menu.
#[derive(Clone, Debug)]
pub struct ContextMenu {
    /// Top-left corner, window coordinates.
    pub anchor: Point,
    pub items: Vec<MenuItem>,
    pub input: Option<MenuInput>,
    pub slider: Option<MenuSlider>,
    /// The item the arrow keys moved to.
    pub selected: Option<usize>,
}

impl ContextMenu {
    /// The tab menu (applies to the active tab: right-click activates first); `ssh` adds the
    /// SFTP panel and port forwarding items, `stored` enables "Save to profile".
    pub fn tab(anchor: Point, ssh: bool, stored: bool) -> Self {
        let run = |label: &str, action| MenuItem::new(label, MenuAction::Run(action));
        let mut items = vec![
            MenuItem::new("Rename…", MenuAction::Run(Action::RenameTab)),
            MenuItem::new("Color ▸", MenuAction::Ui(Message::MenuColors)),
            MenuItem::new(
                "Background ▸",
                MenuAction::Ui(Message::Background(crate::background_menu::BgMsg::Menu)),
            ),
        ];
        items.extend([
            run("Duplicate", Action::DuplicateTab).after_separator(),
            MenuItem::new("Split ▸", MenuAction::Ui(Message::MenuSplit)),
            MenuItem::new("Save to profile", MenuAction::Ui(Message::TabSaveToProfile))
                .enabled_if(stored, "Only for tabs of a stored profile")
                .after_separator(),
        ]);
        if ssh {
            items.push(run("SFTP panel", Action::ToggleSftp).after_separator());
            items.push(MenuItem::new(
                "Port forwarding…",
                MenuAction::Ui(Message::Forwards(crate::forwards_dialog::ForwardsMsg::Open)),
            ));
        }
        items.extend([
            run("Close others", Action::CloseOtherTabs).after_separator(),
            run("Close tabs to the right", Action::CloseTabsRight),
            run("Close", Action::CloseTab),
            run("Clear saved sessions…", Action::ClearSavedSessions).after_separator(),
        ]);
        Self::new(anchor, items)
    }

    /// A menu of plain items.
    pub fn new(anchor: Point, items: Vec<MenuItem>) -> Self {
        Self {
            anchor,
            items,
            input: None,
            slider: None,
            selected: None,
        }
    }

    /// Moves the keyboard selection to the next (`down`) or previous enabled item, wrapping.
    pub fn move_selection(&mut self, down: bool) {
        let enabled: Vec<usize> = (0..self.items.len())
            .filter(|i| self.items.get(*i).is_some_and(|it| it.enabled))
            .collect();
        let len = enabled.len();
        if len == 0 {
            return;
        }
        let at = self
            .selected
            .and_then(|s| enabled.iter().position(|i| *i == s));
        let next = match at {
            Some(at) if down => (at + 1) % len,
            Some(at) => (at + len - 1) % len,
            None if down => 0,
            None => len - 1,
        };
        self.selected = enabled.get(next).copied();
    }

    /// What Enter does: the selected item's message.
    pub fn chosen(&self) -> Option<Message> {
        let item = self.items.get(self.selected?)?;
        item.enabled.then(|| item.action.message())
    }

    /// The tab colour palette: presets, "None" and a custom hex field.
    pub fn tab_colors(anchor: Point) -> Self {
        let presets = PRESET_COLORS.map(|(name, hex)| (name, RgbColor::from_hex(hex)));
        Self::palette(anchor, &presets, Some(Message::SetTabColor(None)), |c| {
            Message::SetTabColor(Some(c))
        })
    }

    /// A colour palette: `presets` as swatches, an optional "None" item and a custom hex
    /// field; picking a colour sends `pick(colour)`.
    pub fn palette(
        anchor: Point,
        presets: &[(&str, RgbColor)],
        none: Option<Message>,
        pick: fn(RgbColor) -> Message,
    ) -> Self {
        let mut items: Vec<MenuItem> = presets
            .iter()
            .map(|&(name, color)| {
                let mut item = MenuItem::new(name, MenuAction::Ui(pick(color)));
                item.swatch = Some(color);
                item
            })
            .collect();
        if let Some(none) = none {
            let mut none = MenuItem::new("None", MenuAction::Ui(none));
            none.separator = true;
            items.push(none);
        }
        let mut menu = Self::new(anchor, items);
        menu.input = Some(MenuInput {
            placeholder: "Custom hex… #RRGGBB",
            value: String::new(),
            invalid: false,
            submit: MenuSubmit::Color(pick),
        });
        menu
    }

    /// Top-left corner: at the anchor, flipped to the anchor's left / above it when the menu
    /// would cross the window's right / bottom edge, and kept inside the window.
    fn position(&self, window: Size) -> Point {
        let extra = usize::from(self.input.is_some()) + usize::from(self.slider.is_some());
        let rows = self.items.len() + extra * 2;
        let height = crate::metrics::cells_f32(rows) * ROW_H + 8.0;
        Point::new(
            flip(self.anchor.x, MENU_W, window.width),
            flip(self.anchor.y, height, window.height),
        )
    }
}

/// Start of a `size`-long span opened at `at` inside `0..limit`: `at` when it fits, else ending
/// at `at` (flipped), else clamped.
fn flip(at: f32, size: f32, limit: f32) -> f32 {
    if at + size <= limit {
        at
    } else if at - size >= 0.0 {
        at - size
    } else {
        (limit - size).max(0.0)
    }
}

/// `"#rrggbb"` or `"rrggbb"` (surrounding spaces ignored) as a colour.
pub fn parse_color(input: &str) -> Option<RgbColor> {
    let s = input.trim();
    let hex = if s.starts_with('#') {
        s.to_owned()
    } else {
        format!("#{s}")
    };
    RgbColor::parse_hex(&hex).ok()
}

/// `RgbColor` as an iced colour.
pub fn to_iced(c: RgbColor) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

fn swatch<'a>(color: Option<RgbColor>, look: Look) -> Element<'a, Message> {
    let fill = color.map(to_iced);
    container(space())
        .width(12)
        .height(12)
        .style(move |_theme: &iced::Theme| container::Style {
            background: fill.map(Into::into),
            border: Border {
                color: look.border,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn item_view(item: &MenuItem, selected: bool, look: Look) -> Element<'_, Message> {
    let mut label = row![].spacing(8).align_y(Alignment::Center);
    if let Some(on) = item.checked {
        label = label.push(
            text(if on { "✓" } else { " " })
                .size(13)
                .width(14)
                .color(look.accent),
        );
    }
    if item.swatch.is_some() {
        label = label.push(swatch(item.swatch, look));
    }
    // "Split ▸": the arrow goes to the right edge as a chevron.
    let (name, submenu) = match item.label.strip_suffix(" ▸") {
        Some(name) => (name, true),
        None => (item.label.as_str(), false),
    };
    label = label.push(text(name).size(13).width(Length::Fill));
    if submenu {
        label = label.push(text("›").size(15).color(look.dim));
    }
    let enabled = item.enabled;
    let button = button(label)
        .width(Length::Fill)
        .padding([5, 12])
        .style(move |_theme: &iced::Theme, status| {
            let hot = enabled && (selected || matches!(status, button::Status::Hovered));
            button::Style {
                background: hot.then_some(look.hover.into()),
                text_color: if enabled {
                    look.fg
                } else {
                    Color { a: 0.4, ..look.fg }
                },
                border: Border::default().rounded(4),
                ..button::Style::default()
            }
        })
        .on_press_maybe(enabled.then(|| item.action.message()));
    match item.tooltip {
        Some(hint) => tooltip(
            button,
            container(text(hint).size(12))
                .padding(6)
                .style(container::rounded_box),
            tooltip::Position::Bottom,
        )
        .into(),
        None => button.into(),
    }
}

fn separator<'a>(look: Look) -> Element<'a, Message> {
    let line = Color { a: 0.12, ..look.fg };
    container(
        rule::horizontal(1).style(move |theme: &iced::Theme| rule::Style {
            color: line,
            ..rule::default(theme)
        }),
    )
    .padding([3, 0])
    .into()
}

fn menu_box(menu: &ContextMenu, look: Look) -> Element<'_, Message> {
    let mut col = column![].spacing(2);
    for (i, item) in menu.items.iter().enumerate() {
        if item.separator {
            col = col.push(separator(look));
        }
        col = col.push(item_view(item, menu.selected == Some(i), look));
    }
    if let Some(s) = &menu.slider {
        col = col.push(separator(look)).push(
            column![
                text(s.label.as_str()).size(13),
                slider(s.range.clone(), s.value, s.on_change).step(5.0_f32),
            ]
            .spacing(4)
            .padding([4, 12]),
        );
    }
    if let Some(input) = &menu.input {
        col = col.push(separator(look)).push(
            text_input(input.placeholder, &input.value)
                .id(Id::new(MENU_INPUT))
                .on_input(Message::MenuInput)
                .on_submit(Message::MenuSubmit)
                .size(13)
                .padding(6),
        );
        if input.invalid && matches!(input.submit, MenuSubmit::Color(_)) {
            col = col.push(text("Not a #RRGGBB colour").size(11).color(look.danger));
        }
    }
    container(col)
        .padding(4)
        .width(MENU_W)
        .style(move |_theme: &iced::Theme| panel_style(look))
        .into()
}

/// Floating panels (menus, the profile picker): raised surface, thin border, soft shadow.
pub(crate) fn panel_style(look: Look) -> container::Style {
    container::Style {
        background: Some(look.bg.into()),
        text_color: Some(look.fg),
        border: Border {
            color: look.border,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: iced::Shadow {
            color: Color {
                a: 0.35,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
        ..container::Style::default()
    }
}

/// Stacks `menu` over `base` at its anchor; clicks outside it (any button) close it and never
/// reach `base`. Just `base` when there is no menu.
pub fn overlay<'a>(
    base: Element<'a, Message>,
    menu: Option<&'a ContextMenu>,
    window: Size,
    look: Look,
) -> Element<'a, Message> {
    let Some(menu) = menu else { return base };
    let at = menu.position(window);
    let click_away = mouse_area(space().width(Length::Fill).height(Length::Fill))
        .on_press(Message::CloseMenu)
        .on_right_press(Message::CloseMenu)
        .on_middle_press(Message::CloseMenu);
    let placed = container(menu_box(menu, look))
        .padding(Padding {
            top: at.y,
            left: at.x,
            right: 0.0,
            bottom: 0.0,
        })
        .width(Length::Fill)
        .height(Length::Fill);
    stack![base, opaque(click_away), placed].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_hex() {
        let red = Some(RgbColor { r: 255, g: 0, b: 0 });
        assert_eq!(parse_color("#ff0000"), red);
        assert_eq!(parse_color(" FF0000 "), red);
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#ff00"), None);
        assert_eq!(parse_color(""), None);
    }

    #[test]
    fn menus_flip_inward_near_the_edges() {
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(flip(100.0, 200.0, 1000.0), 100.0), "fits");
        assert!(
            close(flip(990.0, 200.0, 1000.0), 790.0),
            "opens to the left"
        );
        assert!(close(flip(50.0, 200.0, 120.0), 0.0), "too big: clamped");
        let menu = ContextMenu::tab(Point::new(990.0, 640.0), false, true);
        let at = menu.position(Size::new(1000.0, 650.0));
        assert!(close(at.x, 990.0 - MENU_W) && at.y < 640.0, "{at:?}");
    }

    #[test]
    fn arrows_skip_disabled_items_and_enter_runs_the_selection() {
        let mut menu = ContextMenu::new(
            Point::ORIGIN,
            vec![
                MenuItem::new("a", MenuAction::Run(Action::Copy)),
                MenuItem::new("b", MenuAction::Run(Action::Paste)).disabled("no"),
                MenuItem::new("c", MenuAction::Run(Action::Clear)),
            ],
        );
        assert!(menu.chosen().is_none());
        menu.move_selection(true);
        assert_eq!(menu.selected, Some(0));
        menu.move_selection(true);
        assert_eq!(menu.selected, Some(2), "b is skipped");
        menu.move_selection(true);
        assert_eq!(menu.selected, Some(0), "wraps");
        menu.move_selection(false);
        assert!(matches!(
            menu.chosen(),
            Some(Message::Action(Action::Clear))
        ));
    }
}
