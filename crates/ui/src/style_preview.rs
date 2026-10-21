//! The settings page's style thumbnails: a miniature of the window drawn with the style's own
//! structure (bar placement and fill, tab anatomy, islands, sidebar) over its paired palette, so
//! the six styles read as different layouts before they are applied.

use fogu_core::style::{
    BarFill, BarPlacement, Geometry, IndicatorColor, SidebarMode, TabSizing, TabStyle,
};
use fogu_core::{ColorScheme, RgbColor, StyleDef};
use iced::gradient::Linear;
use iced::widget::{Space, column, container, row, stack};
use iced::{Alignment, Background, Border, Color, Element, Length, Radians, Shadow};

use crate::design::Env;
use crate::message::Message;
use crate::palette::{Colors, Look, mix};

/// Height of a thumbnail.
pub(crate) const PREVIEW_H: f32 = 96.0;
/// Radii scale down with the drawing.
const SCALE: f32 = 0.5;

fn rgb(c: RgbColor) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

/// A filled rounded block `w`×`h`.
fn block<'a>(
    w: impl Into<Length>,
    h: impl Into<Length>,
    fill: Color,
    radius: f32,
) -> Element<'a, Message> {
    container(Space::new().width(w).height(h))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(fill.into()),
            border: Border::default().rounded(radius),
            ..container::Style::default()
        })
        .into()
}

/// A short line standing for a tab title or a line of text.
fn line<'a>(w: f32, color: Color) -> Element<'a, Message> {
    block(w, 2.0, color, 1.0)
}

/// The bar's background, as the real bar fills it.
fn bar_fill(tone: Look) -> Background {
    let raised = tone.raised;
    match tone.bar.fill {
        BarFill::Solid => raised.into(),
        BarFill::Base => tone.base.into(),
        BarFill::Vertical => Linear::new(Radians(std::f32::consts::PI))
            .add_stop(0.0, mix(raised, tone.accent, tone.bar.gradient))
            .add_stop(1.0, raised)
            .into(),
        BarFill::Horizontal => Linear::new(Radians(std::f32::consts::FRAC_PI_2))
            .add_stop(0.0, mix(raised, tone.accent, tone.bar.gradient))
            .add_stop(0.5, raised)
            .add_stop(1.0, mix(raised, tone.secondary, tone.bar.gradient))
            .into(),
    }
}

/// One miniature tab header.
#[expect(
    clippy::too_many_lines,
    reason = "one miniature per tab style, mirroring the bar's own header"
)]
fn tab<'a>(tone: Look, index: usize, width: f32, height: f32) -> Element<'a, Message> {
    let active = index == 0;
    let spec = tone.tabs;
    let bar = match tone.bar.fill {
        BarFill::Base => tone.base,
        _ => tone.raised,
    };
    let idle = mix(tone.fg, bar, 0.45);
    let indicator = match spec.indicator_color {
        IndicatorColor::Accent => tone.accent,
        IndicatorColor::Fg => tone.fg,
    };
    let (fill, title, border) = match spec.style {
        TabStyle::Pill => (
            active.then(|| mix(bar, tone.fg, 0.12)),
            if active { tone.fg } else { idle },
            active.then(|| mix(bar, tone.accent, 0.6)),
        ),
        TabStyle::Folder => (
            active.then_some(tone.base),
            if active { tone.fg } else { idle },
            None,
        ),
        TabStyle::Underline => (None, if active { tone.fg } else { idle }, None),
        TabStyle::Inverse => (
            active.then_some(tone.accent),
            if active { tone.on_accent } else { idle },
            None,
        ),
        TabStyle::Glow => (
            active.then_some(Color {
                a: 0.16,
                ..tone.accent
            }),
            if active { tone.fg } else { idle },
            None,
        ),
    };
    let marker: Element<'a, Message> = match spec.marker {
        fogu_core::style::TabMarker::Dot => block(
            3.0,
            3.0,
            if active { tone.accent } else { idle },
            tone.shape.dot.min(1.5),
        ),
        fogu_core::style::TabMarker::Index => block(2.0, 4.0, title, 0.0),
        fogu_core::style::TabMarker::None => Space::new().width(0).into(),
    };
    let label = row![marker, line((width - 12.0).max(6.0) * 0.6, title)]
        .spacing(3)
        .align_y(Alignment::Center);
    let radius = iced::border::Radius {
        top_left: tone.shape.tab.top_left * SCALE,
        top_right: tone.shape.tab.top_right * SCALE,
        bottom_right: tone.shape.tab.bottom_right * SCALE,
        bottom_left: tone.shape.tab.bottom_left * SCALE,
    };
    let glow = spec.style == TabStyle::Glow;
    let shadow = tone.shadow.tab;
    let mut body = column![
        container(label)
            .height(Length::Fill)
            .align_y(Alignment::Center)
            .padding([0, 4])
    ];
    if active && spec.indicator > 0.0 {
        let length = if spec.indicator_length > 0.0 {
            (spec.indicator_length * SCALE).min(width)
        } else {
            width - 6.0
        };
        let bar_line = container(Space::new().width(length).height(1.5)).style(move |_theme| {
            container::Style {
                background: Some(indicator.into()),
                shadow: if glow {
                    Shadow {
                        blur_radius: 5.0,
                        ..shadow
                    }
                } else {
                    Shadow::default()
                },
                ..container::Style::default()
            }
        });
        body = body.push(container(bar_line).center_x(Length::Fill).padding([1, 0]));
    }
    let pad = match spec.style {
        TabStyle::Pill | TabStyle::Glow => [2.0, 1.0, 2.0],
        TabStyle::Folder => [3.0, 0.0, 0.0],
        TabStyle::Inverse => [1.5, 0.0, 1.5],
        TabStyle::Underline => [0.0, 0.0, 0.0],
    };
    let chip = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::Style {
            background: fill.map(Into::into),
            border: Border {
                color: border.unwrap_or(Color::TRANSPARENT),
                width: if border.is_some() { 1.0 } else { 0.0 },
                radius,
            },
            ..container::Style::default()
        });
    container(chip)
        .padding(iced::Padding {
            top: pad[0],
            right: pad[1],
            bottom: pad[2],
            left: pad[1],
        })
        .width(width)
        .height(height)
        .into()
}

/// The miniature tab strip: three tabs sized by the style's policy.
fn strip<'a>(tone: Look, height: f32, island: bool) -> Element<'a, Message> {
    let widths: [f32; 3] = match tone.tabs.sizing {
        TabSizing::Fixed => [34.0; 3],
        TabSizing::Fill => [46.0; 3],
        TabSizing::Fit => [24.0, 36.0, 29.0],
    };
    let mut tabs = row![].spacing(1).align_y(Alignment::End);
    for (i, w) in widths.into_iter().enumerate() {
        if i > 0 && tone.tabs.separators && i > 1 {
            tabs = tabs.push(
                container(block(1.0, height * 0.4, tone.border, 0.0))
                    .height(height)
                    .align_y(Alignment::Center),
            );
        }
        tabs = tabs.push(tab(tone, i, w, height));
    }
    let fill = bar_fill(tone);
    let merge = tone.tabs.style == TabStyle::Folder;
    let content: Element<'a, Message> = if merge {
        stack![
            column![
                Space::new().height(Length::Fill),
                block(Length::Fill, 1.0, tone.border, 0.0)
            ],
            tabs
        ]
        .into()
    } else {
        tabs.into()
    };
    let radius = if island { tone.shape.card * SCALE } else { 0.0 };
    let border = tone.border;
    container(content)
        .padding([0.0, if island { 4.0 } else { 2.0 }])
        .width(Length::Fill)
        .height(height)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(fill),
            border: Border {
                color: if island { border } else { Color::TRANSPARENT },
                width: if island { 1.0 } else { 0.0 },
                radius: radius.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A few coloured lines on the terminal background.
fn terminal<'a>(tone: Look, scheme: &ColorScheme, island: bool) -> Element<'a, Message> {
    let ansi = |i: usize| scheme.ansi.get(i).copied().map_or(tone.fg, rgb);
    let lines = column![
        row![line(10.0, ansi(2)), line(26.0, tone.fg)].spacing(3),
        row![line(18.0, ansi(4)), line(14.0, ansi(5))].spacing(3),
        line(30.0, mix(tone.fg, tone.base, 0.3)),
        row![
            line(10.0, ansi(2)),
            block(4.0, 6.0, rgb(scheme.cursor), 0.0)
        ]
        .spacing(3)
        .align_y(Alignment::Center),
    ]
    .spacing(4);
    let radius = if island { tone.shape.card * SCALE } else { 0.0 };
    let (base, border) = (tone.base, tone.border);
    container(lines)
        .padding(6)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(base.into()),
            border: Border {
                color: if island { border } else { Color::TRANSPARENT },
                width: if island { 1.0 } else { 0.0 },
                radius: radius.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// The sidebar: flush with a line, or a detached card.
fn sidebar<'a>(tone: Look) -> Element<'a, Message> {
    let floating = tone.layout.sidebar == SidebarMode::Floating;
    let rows = column![
        line(14.0, tone.dim),
        block(Length::Fill, 5.0, tone.accent_soft, 1.0),
        line(18.0, tone.dim),
        line(12.0, tone.dim),
    ]
    .spacing(4);
    let (panel, border, radius) = (tone.panel, tone.border, tone.shape.card * SCALE);
    container(rows)
        .padding(4)
        .width(34)
        .height(Length::Fill)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(panel.into()),
            border: Border {
                color: border,
                width: if floating { 1.0 } else { 0.0 },
                radius: if floating { radius.into() } else { 0.0.into() },
            },
            ..container::Style::default()
        })
        .into()
}

/// The thumbnail of `style` over `scheme`.
pub(crate) fn preview<'a>(
    style: &StyleDef,
    scheme: &ColorScheme,
    accent: Option<RgbColor>,
    env: Env,
) -> Element<'a, Message> {
    let tone = Look::new(&Colors::new(scheme), style, accent, env);
    let island = tone.layout.geometry == Geometry::Islands;
    let gap = if island { 3.0 } else { 0.0 };
    let rule = |show: bool| -> Element<'a, Message> {
        if show && !island {
            block(Length::Fill, 1.0, tone.border, 0.0)
        } else {
            Space::new().height(0).into()
        }
    };
    let side_gap = if tone.layout.sidebar == SidebarMode::Floating {
        3.0
    } else {
        0.0
    };
    let body = row![terminal(tone, scheme, island), sidebar(tone)]
        .spacing(side_gap)
        .height(Length::Fill);
    let merge = tone.tabs.style == TabStyle::Folder;
    let window = match tone.bar.placement {
        BarPlacement::Top => column![
            strip(tone, 18.0, island),
            rule(tone.bar.rule && !merge),
            body
        ],
        BarPlacement::Bottom => {
            let top = container(Space::new())
                .width(Length::Fill)
                .height(9)
                .style(move |_theme: &iced::Theme| container::background(bar_fill(tone)));
            column![
                top,
                rule(tone.bar.rule),
                body,
                rule(tone.bar.rule),
                strip(tone, 13.0, island)
            ]
        }
    }
    .spacing(gap);
    let canvas = if island { tone.canvas } else { tone.base };
    container(window)
        .padding(gap)
        .width(Length::Fill)
        .height(PREVIEW_H)
        .clip(true)
        .style(move |_theme: &iced::Theme| container::background(canvas))
        .into()
}
