//! Shared widget styles: surfaces, fields and flat chrome buttons.
//!
//! Every style comes from [`Look`], so the four UI presets change the whole chrome without
//! touching the terminal palette.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow, Vector};

use crate::palette::Look;

/// A filled dock/panel surface with the preset's radius and, for Fluent, a soft shadow.
pub(crate) fn panel(look: Look) -> container::Style {
    container::Style {
        background: Some(look.panel.into()),
        text_color: Some(look.fg),
        border: Border {
            color: look.border,
            width: 1.0,
            radius: look.radius.into(),
        },
        shadow: if look.elevated {
            Shadow {
                color: Color {
                    a: 0.22,
                    ..Color::BLACK
                },
                offset: Vector::new(0.0, 6.0),
                blur_radius: 18.0,
            }
        } else {
            Shadow::default()
        },
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
            width: 1.0,
            radius: look.radius.into(),
        },
        shadow: Shadow {
            color: Color {
                a: 0.42,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

/// Text fields sit on the terminal background so they read as inputs inside a dock.
pub(crate) fn field(look: Look, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(if focused {
            look.base
        } else {
            crate::palette::mix(look.panel, look.base, 0.55)
        }),
        border: Border {
            color: if focused { look.accent } else { look.border },
            width: 1.0,
            radius: look.radius_small.into(),
        },
        icon: look.dim,
        placeholder: look.dim,
        value: look.fg,
        selection: look.accent_soft,
    }
}

/// Flat chrome button: hover fills, danger turns red with white text.
pub(crate) fn flat_button(look: Look, status: button::Status, danger: bool) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered | button::Status::Pressed if danger => {
            (Some(look.danger), Color::WHITE)
        }
        button::Status::Hovered => (Some(look.hover), look.fg),
        button::Status::Pressed => (Some(Color { a: 0.18, ..look.fg }), look.fg),
        _ => (None, look.fg),
    };
    button::Style {
        background: background.map(Into::into),
        text_color,
        border: Border::default().rounded(look.radius_small),
        shadow: Shadow::default(),
        snap: true,
    }
}
