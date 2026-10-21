//! The Appearance page: live UI styles, accent and terminal palette selection.

use fogu_core::{ColorScheme, Config, RgbColor, SchemeCatalog, StyleCatalog, StyleDef};
use iced::widget::{column, container, mouse_area, row, space, text};
use iced::{Alignment, Border, Color, Element, Length};

use super::{Anchor, SettingsMsg, anchored};
use crate::controls::{self, ButtonKind};
use crate::design::{Env, resolve_style};
use crate::icons::{Icon, icon};
use crate::message::Message;
use crate::palette::Look;
use crate::style_code::CodeKind;
use crate::style_preview;

/// Style cards per row.
const PER_ROW: usize = 3;
/// Height of a card's description: two lines of 11 px text.
const DESCRIPTION_H: f32 = 30.0;

/// The page body (inside the settings page's scroll).
#[expect(
    clippy::too_many_lines,
    reason = "one settings page; splitting the visual tree would spread related controls apart"
)]
pub(super) fn body<'a>(
    look: Look,
    config: &Config,
    (schemes, styles): (&SchemeCatalog, &StyleCatalog),
    env: Env,
    highlight: Option<Anchor>,
) -> Element<'a, Message> {
    let (current, _) = resolve_style(&config.appearance.ui_theme, styles);
    let accent = config.appearance.accent;
    let selected_scheme = config.appearance.color_scheme.as_str();
    let body = |label: &str| {
        text(label.to_owned())
            .size(12)
            .font(look.text.chrome)
            .color(look.dim)
    };

    let card_for = |style: &StyleDef| {
        let scheme = preview_scheme(style, selected_scheme, schemes);
        style_card(style, current, scheme, accent, look, env)
    };
    let (builtin, custom): (Vec<&StyleDef>, Vec<&StyleDef>) = styles
        .styles()
        .iter()
        .partition(|s| StyleCatalog::is_builtin(s));
    let mut themes = column![grid(builtin.iter().map(|s| card_for(s)).collect())].spacing(12);
    if !custom.is_empty() {
        themes = themes
            .push(controls::section_label(look, "Custom styles"))
            .push(grid(custom.iter().map(|s| card_for(s)).collect()));
    }
    themes = themes.push(code_button(
        look,
        &format!("New style from code, based on {}", current.name),
        SettingsMsg::NewCode(CodeKind::Style),
    ));

    let swatch = |value: Option<RgbColor>, label: &'a str| {
        let c = value.unwrap_or(current.color.accent);
        let color = Color::from_rgb8(c.r, c.g, c.b);
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
                    radius: (look.shape.dot * 3.0).min(11.0).into(),
                },
                ..container::Style::default()
            }
        });
        mouse_area(
            column![
                dot,
                text(label).size(10).font(look.text.chrome).color(look.dim)
            ]
            .spacing(5)
            .align_x(Alignment::Center),
        )
        .on_press(Message::Settings(SettingsMsg::Accent(value)))
    };

    let accents = row![
        swatch(None, "Style"),
        swatch(Some(RgbColor::from_hex(0xFF_6B_35)), "Ember"),
        swatch(Some(RgbColor::from_hex(0x4C_C2_FF)), "Sky"),
        swatch(Some(RgbColor::from_hex(0x2F_6F_ED)), "Blue"),
        swatch(Some(RgbColor::from_hex(0x63_FF_8A)), "Green"),
        swatch(Some(RgbColor::from_hex(0xFF_4F_D8)), "Magenta"),
        swatch(Some(RgbColor::from_hex(0x5E_EA_D4)), "Teal"),
        swatch(Some(RgbColor::from_hex(0xE5_48_4D)), "Red"),
    ]
    .spacing(14)
    .align_y(Alignment::Start);

    // Part of the page's scroll: no second scrollbar inside it.
    let scheme_rows = schemes.schemes().iter().map(|scheme| {
        let custom = schemes.source(&scheme.name).is_some();
        scheme_row(scheme, scheme.name == selected_scheme, custom, look)
    });
    let scheme_list = column(scheme_rows).spacing(2);
    let reload = iced::widget::button(
        row![
            icon(Icon::Refresh, 12.0, look.dim),
            text("Reload styles and palettes")
                .size(11)
                .font(look.text.chrome)
                .color(look.dim),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([3, 7])
    .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
    .on_press(Message::Settings(SettingsMsg::Reload));
    let mut palette_header = row![
        controls::section_label(look, "Terminal palette"),
        space().width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    match current.paired_scheme.as_deref() {
        Some(paired) if paired != selected_scheme && schemes.by_name(paired).is_some() => {
            palette_header = palette_header.push(controls::action(
                &format!("Use matching palette ({paired})"),
                ButtonKind::Secondary,
                look,
                Some(Message::Settings(SettingsMsg::UseMatchingPalette)),
            ));
        }
        Some(paired) if schemes.by_name(paired).is_none() => {
            palette_header =
                palette_header.push(body(&format!("Paired palette {paired:?} not found")));
        }
        _ => {}
    }
    palette_header = palette_header.push(code_button(
        look,
        "New palette from code",
        SettingsMsg::NewCode(CodeKind::Palette),
    ));
    palette_header = palette_header.push(reload);

    let density = match current.layout.density {
        fogu_core::style::Density::Compact => "Compact rows",
        fogu_core::style::Density::Regular => "Regular rows",
        fogu_core::style::Density::Comfy => "Comfortable rows",
    };
    let block = |anchor: Anchor, content: Element<'a, Message>| {
        anchored(anchor.id(), highlight == Some(anchor), content, look)
    };
    column![
        block(
            Anchor::UiStyle,
            column![
                controls::section_label(look, "UI style"),
                body(
                    "Each style changes the window's structure: where the tabs sit and how they \
                     look, islands or docked panels, density, buttons and dialogs."
                ),
                themes,
            ]
            .spacing(12)
            .into(),
        ),
        block(
            Anchor::Accent,
            column![
                controls::section_label(look, "Accent"),
                body(
                    "The accent tints selections, focus rings and active controls; the terminal \
                     palette stays independent."
                ),
                accents,
            ]
            .spacing(12)
            .into(),
        ),
        block(
            Anchor::Palette,
            column![
                palette_header,
                body(
                    "Switching style also switches to its paired palette while the current one \
                     is the default or another style's pair; a palette you pick stays."
                ),
                scheme_list,
            ]
            .spacing(12)
            .into(),
        ),
        block(
            Anchor::Density,
            column![
                controls::section_label(look, "Density"),
                body(&format!("{density}, set by the {} style.", current.name)),
            ]
            .spacing(12)
            .into(),
        ),
    ]
    .spacing(20)
    .into()
}

/// The palette a style's thumbnail is drawn with: its paired one, else the current one.
fn preview_scheme<'s>(
    style: &StyleDef,
    selected: &str,
    schemes: &'s SchemeCatalog,
) -> &'s ColorScheme {
    style
        .paired_scheme
        .as_deref()
        .and_then(|name| schemes.by_name(name))
        .or_else(|| schemes.by_name(selected))
        .or_else(|| schemes.schemes().first())
        .unwrap_or_else(|| default_scheme())
}

/// The built-in default scheme, for previews when no scheme resolves.
pub(super) fn default_scheme() -> &'static ColorScheme {
    static DEFAULT: std::sync::LazyLock<ColorScheme> =
        std::sync::LazyLock::new(ColorScheme::default);
    &DEFAULT
}

/// `cards` in rows of [`PER_ROW`], short rows padded so cards keep one width.
fn grid(cards: Vec<Element<'_, Message>>) -> Element<'_, Message> {
    let mut rows = column![].spacing(12);
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        let mut line = row![].spacing(12);
        let mut n = 0;
        for card in cards.by_ref().take(PER_ROW) {
            line = line.push(container(card).width(Length::FillPortion(1)));
            n += 1;
        }
        for _ in n..PER_ROW {
            line = line.push(space().width(Length::FillPortion(1)));
        }
        rows = rows.push(line);
    }
    rows.into()
}

/// A flat text button that opens the code editor.
fn code_button<'a>(look: Look, label: &str, message: SettingsMsg) -> Element<'a, Message> {
    iced::widget::button(
        row![
            text("{ }")
                .size(11)
                .font(look.text.mono)
                .color(look.accent_text),
            text(label.to_owned())
                .size(11)
                .font(look.text.chrome)
                .color(look.fg),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([4, 8])
    .style(move |_theme: &iced::Theme, status| controls::flat_button(look, status, false))
    .on_press(Message::Settings(message))
    .into()
}

fn scheme_row(
    scheme: &ColorScheme,
    selected: bool,
    custom: bool,
    look: Look,
) -> Element<'static, Message> {
    let ansi = |index: usize| scheme.ansi.get(index).copied().unwrap_or(scheme.fg);
    let swatches = row![
        swatch_color(scheme.bg, look),
        swatch_color(scheme.fg, look),
        swatch_color(ansi(1), look),
        swatch_color(ansi(2), look),
        swatch_color(ansi(4), look),
        swatch_color(ansi(6), look),
    ]
    .spacing(3);
    let name = scheme.name.clone();
    let label = name.clone();
    let (bg, fg) = controls::row_colors(look, selected, false);
    let row = container(
        row![
            swatches,
            text(label)
                .size(12)
                .font(look.text.chrome)
                .color(fg)
                .width(Length::Fill),
            if selected {
                text("active").size(10).font(look.text.chrome).color(fg)
            } else {
                text("").size(10)
            },
            if custom {
                code_button(
                    look,
                    "Edit",
                    SettingsMsg::EditCode(CodeKind::Palette, scheme.name.clone()),
                )
            } else {
                space().width(0).into()
            },
        ]
        .spacing(9)
        .align_y(Alignment::Center),
    )
    .padding([3, 8])
    .style(move |_theme: &iced::Theme| container::Style {
        background: bg.map(Into::into),
        border: Border {
            color: if selected { look.accent } else { look.border },
            width: 1.0,
            radius: look.shape.control.into(),
        },
        ..container::Style::default()
    });
    mouse_area(row)
        .on_press(Message::Settings(SettingsMsg::Scheme(name)))
        .into()
}

fn swatch_color(color: RgbColor, look: Look) -> Element<'static, Message> {
    let radius = look.shape.dot.min(3.0);
    container(space().width(12).height(12))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(Color::from_rgb8(color.r, color.g, color.b).into()),
            border: Border::default().rounded(radius),
            ..container::Style::default()
        })
        .into()
}

/// One style: its thumbnail, name, description and paired palette.
fn style_card<'a>(
    style: &StyleDef,
    current: &StyleDef,
    scheme: &ColorScheme,
    accent: Option<RgbColor>,
    look: Look,
    env: Env,
) -> Element<'a, Message> {
    let selected = style.key == current.key;
    let preview = style_preview::preview(style, scheme, accent, env);
    let frame =
        container(preview)
            .width(Length::Fill)
            .clip(true)
            .style(move |_theme: &iced::Theme| container::Style {
                border: Border {
                    color: look.border,
                    width: 1.0,
                    radius: look.shape.control.into(),
                },
                ..container::Style::default()
            });
    let palette = style.paired_scheme.clone().unwrap_or_default();
    let ansi = |i: usize| scheme.ansi.get(i).copied().unwrap_or(scheme.fg);
    let swatches = [scheme.bg, scheme.fg, ansi(1), ansi(2), ansi(4)]
        .into_iter()
        .fold(row![].spacing(2), |r, c| r.push(swatch_color(c, look)));
    let card = container(
        column![
            frame,
            row![
                text(style.name.clone())
                    .size(13)
                    .font(look.text.strong)
                    .color(if selected { look.accent_text } else { look.fg })
                    .width(Length::Fill),
                if selected {
                    text("active")
                        .size(10)
                        .font(look.text.chrome)
                        .color(look.accent_text)
                } else {
                    text("").size(10)
                },
            ]
            .align_y(Alignment::Center),
            // Two lines whatever the description's length, so every card has one height.
            container(
                text(style.description.clone())
                    .size(11)
                    .font(look.text.chrome)
                    .color(look.dim)
            )
            .height(DESCRIPTION_H)
            .clip(true),
            row![
                swatches,
                text(palette)
                    .size(10)
                    .font(look.text.chrome)
                    .color(look.dim)
                    .width(Length::Fill),
                if StyleCatalog::is_builtin(style) {
                    space().width(0).into()
                } else {
                    code_button(
                        look,
                        "Edit",
                        SettingsMsg::EditCode(CodeKind::Style, style.key.clone()),
                    )
                },
            ]
            .spacing(6)
            .height(22)
            .align_y(Alignment::Center),
        ]
        .spacing(6),
    )
    .width(Length::Fill)
    .padding(10)
    .style(move |_theme: &iced::Theme| container::Style {
        background: Some(look.panel.into()),
        text_color: Some(look.fg),
        border: if selected {
            Border {
                color: look.accent,
                width: 2.0,
                radius: look.shape.card.into(),
            }
        } else {
            Border {
                color: look.border,
                width: 1.0,
                radius: look.shape.card.into(),
            }
        },
        ..container::Style::default()
    });
    mouse_area(card)
        .on_press(Message::Settings(SettingsMsg::Style(style.key.clone())))
        .into()
}
