//! The one generic context-menu overlay: a `Vec` of
//! items at an anchor point, closed on click-away / Esc. Items map to an [`Action`] (same enum as
//! hotkeys) or to a UI-only [`Message`]. Later menus add builders, not a new widget.

use fogu_core::{Action, RgbColor};
use iced::widget::{
    Id, button, column, container, mouse_area, opaque, row, rule, space, stack, text, text_input,
};
use iced::{Alignment, Border, Color, Element, Length, Padding, Point, Size};

use crate::message::Message;

/// Menu width in logical px.
const MENU_W: f32 = 200.0;
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
        }
    }
}

/// A single-line text field at the bottom of the menu (e.g. "Custom hex…").
#[derive(Clone, Debug, Default)]
pub struct MenuInput {
    pub placeholder: &'static str,
    pub value: String,
    /// The last submit was rejected.
    pub invalid: bool,
}

/// An open context menu.
#[derive(Clone, Debug)]
pub struct ContextMenu {
    /// Top-left corner, window coordinates.
    pub anchor: Point,
    pub items: Vec<MenuItem>,
    pub input: Option<MenuInput>,
}

impl ContextMenu {
    /// The tab menu (applies to the active tab: right-click activates first); `ssh` adds the
    /// SFTP panel toggle.
    pub fn tab(anchor: Point, ssh: bool) -> Self {
        let mut close = MenuItem::new("Close", MenuAction::Run(Action::CloseTab));
        close.separator = true;
        let mut items = vec![
            MenuItem::new("Rename…", MenuAction::Run(Action::RenameTab)),
            MenuItem::new("Color ▸", MenuAction::Ui(Message::MenuColors)),
        ];
        if ssh {
            items.push(MenuItem::new(
                "SFTP panel",
                MenuAction::Run(Action::ToggleSftp),
            ));
        }
        items.push(close);
        Self {
            anchor,
            items,
            input: None,
        }
    }

    /// The tab colour palette: presets, "None" and a custom hex field.
    pub fn tab_colors(anchor: Point) -> Self {
        let mut items: Vec<MenuItem> = PRESET_COLORS
            .iter()
            .map(|&(name, hex)| {
                let color = RgbColor::from_hex(hex);
                let mut item =
                    MenuItem::new(name, MenuAction::Ui(Message::SetTabColor(Some(color))));
                item.swatch = Some(color);
                item
            })
            .collect();
        let mut none = MenuItem::new("None", MenuAction::Ui(Message::SetTabColor(None)));
        none.separator = true;
        items.push(none);
        Self {
            anchor,
            items,
            input: Some(MenuInput {
                placeholder: "Custom hex… #RRGGBB",
                ..MenuInput::default()
            }),
        }
    }

    /// Top-left corner clamped so the menu stays inside `window`.
    fn position(&self, window: Size) -> Point {
        let rows = self.items.len() + usize::from(self.input.is_some()) * 2;
        let height = crate::metrics::cells_f32(rows) * ROW_H + 8.0;
        Point::new(
            self.anchor.x.min(window.width - MENU_W).max(0.0),
            self.anchor.y.min(window.height - height).max(0.0),
        )
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

fn swatch<'a>(color: Option<RgbColor>) -> Element<'a, Message> {
    let fill = color.map(to_iced);
    container(space())
        .width(12)
        .height(12)
        .style(move |_theme: &iced::Theme| container::Style {
            background: fill.map(Into::into),
            border: Border {
                color: Color::from_rgba(0.5, 0.5, 0.5, 0.6),
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn item_view(item: &MenuItem) -> Element<'_, Message> {
    let mut label = row![].spacing(8).align_y(Alignment::Center);
    if item.swatch.is_some() {
        label = label.push(swatch(item.swatch));
    }
    label = label.push(text(item.label.as_str()).size(13));
    button(label)
        .width(Length::Fill)
        .padding([4, 10])
        .style(button::text)
        .on_press_maybe(item.enabled.then(|| item.action.message()))
        .into()
}

fn menu_box(menu: &ContextMenu) -> Element<'_, Message> {
    let mut col = column![].spacing(2);
    for item in &menu.items {
        if item.separator {
            col = col.push(rule::horizontal(1));
        }
        col = col.push(item_view(item));
    }
    if let Some(input) = &menu.input {
        col = col.push(rule::horizontal(1)).push(
            text_input(input.placeholder, &input.value)
                .id(Id::new(MENU_INPUT))
                .on_input(Message::MenuInput)
                .on_submit(Message::MenuSubmit)
                .size(13)
                .padding(6),
        );
        if input.invalid {
            col = col.push(
                text("Not a #RRGGBB colour")
                    .size(11)
                    .color(Color::from_rgb8(0xE5, 0x48, 0x4D)),
            );
        }
    }
    container(col)
        .padding(4)
        .width(MENU_W)
        .style(container::bordered_box)
        .into()
}

/// Stacks `menu` over `base` at its anchor; clicks outside it (any button) close it and never
/// reach `base`. Just `base` when there is no menu.
pub fn overlay<'a>(
    base: Element<'a, Message>,
    menu: Option<&'a ContextMenu>,
    window: Size,
) -> Element<'a, Message> {
    let Some(menu) = menu else { return base };
    let at = menu.position(window);
    let click_away = mouse_area(space().width(Length::Fill).height(Length::Fill))
        .on_press(Message::CloseMenu)
        .on_right_press(Message::CloseMenu)
        .on_middle_press(Message::CloseMenu);
    let placed = container(menu_box(menu))
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
    fn menu_stays_inside_the_window() {
        let menu = ContextMenu::tab(Point::new(990.0, 5.0), false);
        let at = menu.position(Size::new(1000.0, 650.0));
        assert!((at.x - (1000.0 - MENU_W)).abs() < f32::EPSILON);
        assert!((at.y - 5.0).abs() < f32::EPSILON);
    }
}
