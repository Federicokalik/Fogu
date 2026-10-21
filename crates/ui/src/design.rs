//! UI shape presets, independent from the terminal colour scheme.
//!
//! A [`UiTheme`] changes form: radii, borders, elevation, accent defaults and small spacing
//! decisions. The terminal palette still comes from `ColorScheme`, so every preset works with
//! Fogu Dark, Fogu Light, Solarized and Dracula.

use crate::notifications::{Level, Notification};

/// The four selectable shape presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiTheme {
    /// Dark, warm and compact: Fogu's own look.
    #[default]
    Fogu,
    /// Layered surfaces, softer radii, Windows 11 flavour.
    Fluent,
    /// Monochrome, almost square, minimum chrome.
    Minimal,
    /// Terminal nostalgia: square corners, phosphor accent.
    Retro,
}

impl UiTheme {
    /// Resolves the config value; an unknown name falls back to [`Self::Fogu`] and says so.
    #[must_use]
    pub fn resolve(name: &str) -> (Self, Option<Notification>) {
        match name.trim().to_ascii_lowercase().as_str() {
            "fogu" | "ember" | "" => (Self::Fogu, None),
            "fluent" | "windows" => (Self::Fluent, None),
            "minimal" | "paper" => (Self::Minimal, None),
            "retro" | "terminal" => (Self::Retro, None),
            other => (
                Self::Fogu,
                Some(
                    Notification::new(Level::Warn, "Unknown UI theme").detail(format!(
                        "\"{other}\" is not a known UI theme; using \"fogu\""
                    )),
                ),
            ),
        }
    }

    /// Stable config value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fogu => "fogu",
            Self::Fluent => "fluent",
            Self::Minimal => "minimal",
            Self::Retro => "retro",
        }
    }

    /// Default accent, used unless `appearance.accent` overrides it.
    #[must_use]
    pub(crate) const fn accent_rgb(self) -> (u8, u8, u8) {
        match self {
            Self::Fogu => (0xFF, 0x6B, 0x35),
            Self::Fluent => (0x4C, 0xC2, 0xFF),
            Self::Minimal => (0x2F, 0x6F, 0xED),
            Self::Retro => (0x63, 0xFF, 0x8A),
        }
    }

    /// Corner radius of cards, dialogs and floating panels.
    #[must_use]
    pub(crate) const fn radius(self) -> f32 {
        match self {
            Self::Fogu => 8.0,
            Self::Fluent => 10.0,
            Self::Minimal => 3.0,
            Self::Retro => 0.0,
        }
    }

    /// Corner radius of rows, inputs and small controls.
    #[must_use]
    pub(crate) const fn radius_small(self) -> f32 {
        match self {
            Self::Fogu => 5.0,
            Self::Fluent => 6.0,
            Self::Minimal => 2.0,
            Self::Retro => 0.0,
        }
    }

    /// Whether panels get a subtle drop shadow (the renderer draws it as a border/offset).
    #[must_use]
    pub(crate) const fn elevated(self) -> bool {
        matches!(self, Self::Fluent)
    }

    /// Horizontal padding for panel headers and rows.
    #[must_use]
    pub(crate) const fn gutter(self) -> f32 {
        match self {
            Self::Fogu | Self::Fluent => 12.0,
            Self::Minimal => 10.0,
            Self::Retro => 8.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknown_falls_back() {
        for theme in [
            UiTheme::Fogu,
            UiTheme::Fluent,
            UiTheme::Minimal,
            UiTheme::Retro,
        ] {
            assert_eq!(UiTheme::resolve(theme.as_str()).0, theme);
        }
        let (theme, note) = UiTheme::resolve("nope");
        assert_eq!(theme, UiTheme::Fogu);
        assert!(note.is_some());
        assert_eq!(UiTheme::resolve("").0, UiTheme::Fogu);
    }
}
