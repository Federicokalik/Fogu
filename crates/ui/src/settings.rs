//! The settings page: live UI presets, accent and terminal palette selection, persisted to
//! `config.toml`.

use fogu_core::{ColorScheme, Config, RgbColor, SchemeCatalog};
use iced::widget::{column, container, mouse_area, row, scrollable, space, text};
use iced::{Alignment, Border, Color, Element, Length};

use crate::controls;
use crate::design::UiTheme;
use crate::icons::{Icon, icon};
use crate::message::Message;
use crate::palette::{Colors, Look, mix};

/// A choice in the settings page.
#[derive(Clone, Debug)]
pub enum SettingsMsg {
    /// Switch the shape preset (applies immediately and is saved).
    Theme(UiTheme),
    /// Set or clear the accent override.
    Accent(Option<RgbColor>),
    /// Switch the terminal colour scheme by exact name (applies and is saved).
    Scheme(String),
    /// Re-read `themes/` and apply the configured scheme against the new catalog.
    ReloadSchemes,
}

/// The open settings page (only the appearance section exists today).
#[derive(Clone, Copy, Debug, Default)]
pub struct SettingsPage;

/// The page body, placed inside a floating modal card by [`crate::view`].
#[expect(
    clippy::too_many_lines,
    reason = "one settings page; splitting the visual tree would spread related controls apart"
)]
pub fn view<'a>(
    _page: SettingsPage,
    colors: &Colors,
    look: Look,
    config: &Config,
    schemes: &SchemeCatalog,
) -> Element<'a, Message> {
    let (current, _) = UiTheme::resolve(&config.appearance.ui_theme);
    let accent = config.appearance.accent;

    let close = iced::widget::button(icon(Icon::Close, 12.0, look.dim))
        .width(28)
        .height(28)
        .padding(0)
        .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
        .on_press(Message::CloseSettings);
    let header = row![
        column![
            text("Settings").size(22).color(look.fg),
            text("Changes apply live and are saved automatically.")
                .size(12)
                .color(look.dim),
        ]
        .spacing(3)
        .width(Length::Fill),
        close,
    ]
    .align_y(Alignment::Start);

    let nav_item = |label: &'a str, active: bool| {
        let color = if active { look.accent } else { look.dim };
        container(text(label).size(13).color(color))
            .width(Length::Fill)
            .padding([8, 10])
            .style(move |_theme: &iced::Theme| {
                if active {
                    container::Style {
                        background: Some(look.accent_soft.into()),
                        text_color: Some(look.accent),
                        border: Border {
                            color: look.accent,
                            width: 1.0,
                            radius: look.radius_small.into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
    };
    let nav = column![
        nav_item("Appearance", true),
        nav_item("Terminal", false),
        nav_item("Profiles", false),
        nav_item("Keys", false),
        nav_item("Advanced", false),
    ]
    .spacing(2)
    .width(170);

    let themes = row![
        preset_card(UiTheme::Fogu, current, accent, colors, look),
        preset_card(UiTheme::Fluent, current, accent, colors, look),
    ]
    .spacing(12);
    let themes2 = row![
        preset_card(UiTheme::Minimal, current, accent, colors, look),
        preset_card(UiTheme::Retro, current, accent, colors, look),
    ]
    .spacing(12);

    let swatch = |value: Option<RgbColor>, label: &'a str| {
        let color = value.map_or_else(
            || {
                let (r, g, b) = current.accent_rgb();
                Color::from_rgb8(r, g, b)
            },
            |c| Color::from_rgb8(c.r, c.g, c.b),
        );
        let selected = accent == value;
        let dot = container(space().width(22).height(22)).style(move |_theme: &iced::Theme| {
            container::Style {
                background: Some(color.into()),
                border: Border {
                    color: if selected {
                        look.fg
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 2.0,
                    radius: 11.0.into(),
                },
                ..container::Style::default()
            }
        });
        mouse_area(
            column![dot, text(label).size(10).color(look.dim)]
                .spacing(5)
                .align_x(Alignment::Center),
        )
        .on_press(Message::Settings(SettingsMsg::Accent(value)))
    };

    let accents = row![
        swatch(None, "Auto"),
        swatch(Some(RgbColor::from_hex(0xFF_6B_35)), "Ember"),
        swatch(Some(RgbColor::from_hex(0x4C_C2_FF)), "Sky"),
        swatch(Some(RgbColor::from_hex(0x2F_6F_ED)), "Blue"),
        swatch(Some(RgbColor::from_hex(0x63_FF_8A)), "Green"),
        swatch(Some(RgbColor::from_hex(0xA7_8B_FA)), "Violet"),
        swatch(Some(RgbColor::from_hex(0xE5_48_4D)), "Red"),
    ]
    .spacing(14)
    .align_y(Alignment::Start);

    let selected_scheme = config.appearance.color_scheme.as_str();
    let scheme_rows = schemes
        .schemes()
        .iter()
        .map(|scheme| scheme_row(scheme, scheme.name == selected_scheme, look));
    let scheme_list = scrollable(column(scheme_rows).spacing(2)).height(138);
    let reload = iced::widget::button(
        row![
            icon(Icon::Refresh, 12.0, look.dim),
            text("Reload themes").size(11).color(look.dim),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([3, 7])
    .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
    .on_press(Message::Settings(SettingsMsg::ReloadSchemes));
    let palette_header = row![
        section("TERMINAL PALETTE", look),
        space().width(Length::Fill),
        reload,
    ]
    .align_y(Alignment::Center);

    let content = column![
        section("UI THEME", look),
        themes,
        themes2,
        space().height(8),
        section("ACCENT", look),
        text("The accent tints selections, focus rings and active controls; the terminal palette stays independent.")
            .size(12)
            .color(look.dim),
        accents,
        space().height(8),
        palette_header,
        text("Built-in plus custom TOML files from the themes/ directory. Selecting one applies it immediately.")
            .size(12)
            .color(look.dim),
        scheme_list,
        space().height(8),
        section("DENSITY", look),
        text("Compact: tuned for terminal work on 1080p and above.")
            .size(12)
            .color(look.dim),
    ]
    .spacing(12);

    let content = scrollable(content).height(Length::Fill).width(Length::Fill);
    let body = row![nav, rule(look), content]
        .spacing(18)
        .height(Length::Fill);
    container(
        column![header, horizontal_rule(look), body]
            .spacing(16)
            .padding(20),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn scheme_row(scheme: &ColorScheme, selected: bool, look: Look) -> Element<'static, Message> {
    let ansi = |index: usize| scheme.ansi.get(index).copied().unwrap_or(scheme.fg);
    let swatches = row![
        swatch_color(scheme.bg),
        swatch_color(scheme.fg),
        swatch_color(ansi(1)),
        swatch_color(ansi(2)),
        swatch_color(ansi(4)),
        swatch_color(ansi(6)),
    ]
    .spacing(3);
    let name = scheme.name.clone();
    let label = name.clone();
    let row = container(
        row![
            swatches,
            text(label)
                .size(12)
                .color(if selected { look.accent } else { look.fg })
                .width(Length::Fill),
            if selected {
                text("active").size(10).color(look.accent)
            } else {
                text("").size(10)
            },
        ]
        .spacing(9)
        .align_y(Alignment::Center),
    )
    .padding([5, 8])
    .style(move |_theme: &iced::Theme| container::Style {
        background: selected.then_some(look.accent_soft.into()),
        border: Border {
            color: if selected { look.accent } else { look.border },
            width: 1.0,
            radius: look.radius_small.into(),
        },
        ..container::Style::default()
    });
    mouse_area(row)
        .on_press(Message::Settings(SettingsMsg::Scheme(name)))
        .into()
}

fn swatch_color(color: RgbColor) -> Element<'static, Message> {
    container(space().width(12).height(12))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(Color::from_rgb8(color.r, color.g, color.b).into()),
            border: Border::default().rounded(2),
            ..container::Style::default()
        })
        .into()
}

fn section(label: &str, look: Look) -> Element<'_, Message> {
    text(label).size(10).color(look.dim).into()
}

fn rule(look: Look) -> Element<'static, Message> {
    container(space())
        .width(1)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::background(look.border))
        .into()
}

fn horizontal_rule(look: Look) -> Element<'static, Message> {
    container(space())
        .width(Length::Fill)
        .height(1)
        .style(move |_theme: &iced::Theme| container::background(look.border))
        .into()
}

fn preset_card<'a>(
    theme: UiTheme,
    current: UiTheme,
    accent: Option<RgbColor>,
    colors: &Colors,
    look: Look,
) -> Element<'a, Message> {
    let selected = theme == current;
    let tone = Look::new(colors, theme, accent);
    let bar = container(
        row![
            dot(tone.accent, 6.0),
            chip(tone.raised, tone.accent, 34.0),
            chip(tone.raised, tone.border, 22.0),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(20)
    .padding([3, 5])
    .style(move |_theme: &iced::Theme| container::background(tone.raised));
    let terminal = container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::background(tone.base));
    let side = container(space())
        .width(46)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::background(tone.panel));
    let preview = column![bar, row![terminal, side].height(42),];
    let border = if selected {
        Border {
            color: tone.accent,
            width: 2.0,
            radius: look.radius.into(),
        }
    } else {
        Border {
            color: look.border,
            width: 1.0,
            radius: look.radius.into(),
        }
    };
    let name = match theme {
        UiTheme::Fogu => "Fogu Ember",
        UiTheme::Fluent => "Fluent",
        UiTheme::Minimal => "Minimal",
        UiTheme::Retro => "Retro",
    };
    let hint = match theme {
        UiTheme::Fogu => "Warm, compact default",
        UiTheme::Fluent => "Layered, softer surfaces",
        UiTheme::Minimal => "Quiet, monochrome",
        UiTheme::Retro => "Square, phosphor accent",
    };
    let card = container(
        column![
            container(preview)
                .height(66)
                .style(move |_theme: &iced::Theme| container::Style {
                    border: Border {
                        color: tone.border,
                        width: 1.0,
                        radius: look.radius_small.into(),
                    },
                    ..container::Style::default()
                }),
            text(name)
                .size(13)
                .color(if selected { look.accent } else { look.fg }),
            text(hint).size(10).color(look.dim),
        ]
        .spacing(5),
    )
    .width(208)
    .padding(10)
    .style(move |_theme: &iced::Theme| container::Style {
        background: Some(look.panel.into()),
        text_color: Some(look.fg),
        border,
        ..container::Style::default()
    });
    mouse_area(card)
        .on_press(Message::Settings(SettingsMsg::Theme(theme)))
        .into()
}

fn dot<'a>(color: Color, size: f32) -> Element<'a, Message> {
    container(space().width(size).height(size))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(color.into()),
            border: Border::default().rounded(size / 2.0),
            ..container::Style::default()
        })
        .into()
}

fn chip<'a>(surface: Color, accent: Color, width: f32) -> Element<'a, Message> {
    container(space())
        .width(width)
        .height(9)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(mix(surface, accent, 0.22).into()),
            border: Border {
                color: accent,
                width: 1.0,
                radius: 2.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
