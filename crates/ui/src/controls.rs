//! Shared widget styles: surfaces, fields, buttons, rows, tooltips and labels.
//!
//! Every style comes from [`Look`], so a UI style changes the whole chrome (its interaction
//! language included: hover fills, outlines or inverse video; filled, outlined, text or
//! bracketed buttons) without touching the terminal palette.

use fogu_core::style::{ButtonForm, DialogPlacement, HoverMode, SelectionMode};
use iced::widget::{button, container, text, text_input, tooltip};
use iced::{Background, Border, Color, Element, Shadow};

use crate::palette::{Look, mix};

/// A filled dock/panel surface with the style's radius and panel shadow.
pub(crate) fn panel(look: Look) -> container::Style {
    container::Style {
        background: Some(look.panel.into()),
        text_color: Some(look.fg),
        border: Border {
            color: look.border,
            width: look.shape.border,
            radius: look.shape.card.into(),
        },
        shadow: look.shadow.panel,
        ..container::Style::default()
    }
}

/// A raised floating surface (menus, toasts): lighter than [`panel`], deeper shadow.
pub(crate) fn floating(look: Look) -> container::Style {
    container::Style {
        background: Some(look.bg.into()),
        text_color: Some(look.fg),
        border: Border {
            color: look.border,
            width: look.shape.border,
            radius: look.shape.card.into(),
        },
        shadow: look.shadow.floating,
        ..container::Style::default()
    }
}

/// A small dialog: [`floating`], shaped by where the style places it (a top sheet hangs from
/// the edge, a command bar is square).
pub(crate) fn dialog_frame(look: Look) -> container::Style {
    let mut style = floating(look);
    style.border.radius = match look.dialog.placement {
        DialogPlacement::Center => look.shape.card.into(),
        DialogPlacement::TopSheet => iced::border::Radius::default().bottom(look.shape.card),
        DialogPlacement::CommandBar => 0.0.into(),
    };
    style
}

/// The colour that dims the window behind a dialog.
pub(crate) fn scrim(look: Look) -> Color {
    let tint = if look.dark {
        Color::BLACK
    } else {
        mix(Color::BLACK, look.base, 0.3)
    };
    Color {
        a: look.dialog.backdrop,
        ..tint
    }
}

/// Text fields sit on the terminal background so they read as inputs inside a dock.
pub(crate) fn field(look: Look, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(if focused {
            look.base
        } else {
            mix(look.panel, look.base, 0.55)
        }),
        border: Border {
            color: if focused { look.focus } else { look.border },
            width: if focused {
                look.shape.focus
            } else {
                look.shape.border
            },
            radius: look.shape.control.into(),
        },
        icon: look.dim,
        placeholder: look.dim,
        value: look.fg,
        selection: look.accent_soft,
    }
}

/// Flat chrome button (icons, toggles, row actions): the style's hover language, danger turns
/// red with white text.
pub(crate) fn flat_button(look: Look, status: button::Status, danger: bool) -> button::Style {
    let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let pressed = matches!(status, button::Status::Pressed);
    let mut style = button::Style {
        background: None,
        text_color: look.fg,
        border: Border::default().rounded(look.shape.control),
        shadow: Shadow::default(),
        snap: true,
    };
    if hot && danger {
        style.background = Some(look.danger.into());
        style.text_color = Color::WHITE;
        return style;
    }
    if !hot {
        return style;
    }
    match look.interaction.hover {
        HoverMode::Fill => {
            let fill = if pressed {
                Color { a: 0.18, ..look.fg }
            } else {
                look.hover
            };
            style.background = Some(fill.into());
        }
        // Icons keep their own colour, so flat buttons get a strong accent wash instead of a
        // full inversion.
        HoverMode::Inverse => {
            style.background = Some(
                Color {
                    a: if pressed { 0.45 } else { 0.3 },
                    ..look.accent
                }
                .into(),
            );
            style.border = style.border.color(look.accent).width(1.0);
        }
        HoverMode::Outline => {
            style.border = style
                .border
                .color(if pressed { look.fg } else { look.border })
                .width(1.0);
            if pressed {
                style.background = Some(look.hover.into());
            }
        }
    }
    style
}

/// The role of a dialog button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    /// The default action.
    Primary,
    /// Any other action.
    Secondary,
    /// A destructive action.
    Danger,
}

/// A dialog button in the style's button form.
pub(crate) fn button_style(look: Look, kind: ButtonKind, status: button::Status) -> button::Style {
    let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let disabled = matches!(status, button::Status::Disabled);
    let (tone, on_tone) = match kind {
        ButtonKind::Primary => (look.accent, look.on_accent),
        ButtonKind::Secondary => (look.fg, look.base),
        ButtonKind::Danger => (look.danger, Color::WHITE),
    };
    let radius = look.shape.control;
    let width = look.shape.border.max(1.0);
    let mut style = match look.interaction.button {
        ButtonForm::Filled => {
            let (fill, text_color) = match kind {
                ButtonKind::Secondary => (mix(look.panel, look.fg, 0.1), look.fg),
                _ => (tone, on_tone),
            };
            button::Style {
                background: Some(
                    if hot {
                        mix(fill, text_color, 0.12)
                    } else {
                        fill
                    }
                    .into(),
                ),
                text_color,
                border: Border {
                    color: if kind == ButtonKind::Secondary {
                        look.border
                    } else {
                        fill
                    },
                    width,
                    radius: radius.into(),
                },
                ..button::Style::default()
            }
        }
        ButtonForm::Outline => button::Style {
            background: hot.then(|| Color { a: 0.14, ..tone }.into()),
            text_color: text_tone(look, kind),
            border: Border {
                color: if kind == ButtonKind::Secondary {
                    look.border
                } else {
                    tone
                },
                width,
                radius: radius.into(),
            },
            ..button::Style::default()
        },
        ButtonForm::Text => button::Style {
            background: hot.then(|| look.hover.into()),
            text_color: text_tone(look, kind),
            border: Border::default().rounded(radius),
            ..button::Style::default()
        },
        ButtonForm::Inverse => {
            let primary = kind != ButtonKind::Secondary;
            let filled = primary != hot;
            button::Style {
                background: filled.then(|| tone.into()),
                text_color: if filled {
                    on_tone
                } else {
                    text_tone(look, kind)
                },
                border: Border {
                    color: tone,
                    width,
                    radius: radius.into(),
                },
                ..button::Style::default()
            }
        }
    };
    if disabled {
        style.text_color.a *= 0.45;
        if let Some(Background::Color(c)) = style.background {
            style.background = Some(Color { a: c.a * 0.45, ..c }.into());
        }
        style.border.color.a *= 0.45;
    }
    style
}

/// The colour a button of `kind` uses for its text when it is not filled.
fn text_tone(look: Look, kind: ButtonKind) -> Color {
    match kind {
        ButtonKind::Primary => look.accent_text,
        ButtonKind::Secondary => look.fg,
        ButtonKind::Danger => crate::contrast::ensure(look.danger, look.bg, crate::contrast::TEXT),
    }
}

/// [`button_style`] as a widget style closure.
pub(crate) fn kind(
    look: Look,
    kind: ButtonKind,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_theme, status| button_style(look, kind, status)
}

/// A button label in the style's form (bracketed upper case for inverse buttons).
pub(crate) fn button_label(look: Look, label: &str) -> String {
    match look.interaction.button {
        ButtonForm::Inverse => format!("[ {} ]", label.to_uppercase()),
        _ => label.to_owned(),
    }
}

/// A dialog button: `label` in the chrome font, styled as `role`; disabled without a message.
pub(crate) fn action<'a, M: Clone + 'a>(
    label: &str,
    role: ButtonKind,
    look: Look,
    on_press: Option<M>,
) -> button::Button<'a, M> {
    button(
        text(button_label(look, label))
            .size(13)
            .font(look.text.chrome),
    )
    .padding([5, 12])
    .style(kind(look, role))
    .on_press_maybe(on_press)
}

/// A hovered or selected list row in the style's selection language: `(background, text)`.
pub(crate) fn row_colors(look: Look, selected: bool, hovered: bool) -> (Option<Color>, Color) {
    match (selected, look.interaction.selection) {
        (true, SelectionMode::Inverse) => (Some(look.accent), look.on_accent),
        (true, SelectionMode::Fill) => (
            Some(Color {
                a: 0.22,
                ..look.accent
            }),
            look.fg,
        ),
        (false, _) if hovered => (Some(look.hover), look.fg),
        (false, _) => (None, look.fg),
    }
}

/// Tooltips: a small floating card.
pub(crate) fn tooltip_box(look: Look) -> container::Style {
    container::Style {
        background: Some(look.bg.into()),
        text_color: Some(look.fg),
        border: Border {
            color: look.border,
            width: look.shape.border.max(1.0),
            radius: look.shape.control.into(),
        },
        shadow: Shadow {
            blur_radius: look.shadow.floating.blur_radius.min(12.0),
            offset: look.shadow.floating.offset * 0.5,
            ..look.shadow.floating
        },
        ..container::Style::default()
    }
}

/// `content` with a tooltip saying `label`.
pub(crate) fn tip<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    label: impl text::IntoFragment<'a>,
    position: tooltip::Position,
    look: Look,
) -> Element<'a, M> {
    let bubble = container(text(label).size(12).font(look.text.chrome))
        .padding([4, 7])
        .max_width(360)
        .style(move |_theme: &iced::Theme| tooltip_box(look));
    tooltip(content, bubble, position).into()
}

/// A section label ("Terminal palette") in the style's case and chrome font.
pub(crate) fn section_label<'a, M: 'a>(look: Look, label: &str) -> Element<'a, M> {
    let upper = look.text.upper;
    text(look.text.label(label))
        .size(if upper { 10 } else { 11 })
        .font(if upper {
            look.text.chrome
        } else {
            look.text.strong
        })
        .color(look.dim)
        .into()
}

/// A `size` px colour dot with the style's dot radius.
pub(crate) fn dot<'a, M: 'a>(look: Look, color: Color, size: f32) -> Element<'a, M> {
    let radius = look.shape.dot.min(size / 2.0);
    container(iced::widget::space().width(size).height(size))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(color.into()),
            border: Border::default().rounded(radius),
            ..container::Style::default()
        })
        .into()
}

/// A 1 px horizontal hairline.
pub(crate) fn hairline<'a, M: 'a>(color: Color) -> Element<'a, M> {
    container(iced::widget::space())
        .width(iced::Length::Fill)
        .height(1)
        .style(move |_theme: &iced::Theme| container::background(color))
        .into()
}
