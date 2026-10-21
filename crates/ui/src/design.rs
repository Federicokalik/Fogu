//! UI style resolution and the concrete tokens the chrome draws with.
//!
//! A [`StyleDef`] (built-in or from `styles/*.toml`) is plain data; [`Look`](crate::palette::Look)
//! turns it into colours over the active terminal scheme plus the iced-side tokens below. The
//! terminal palette itself still comes from `ColorScheme`, so every style works with every
//! scheme.

use fogu_core::{ColorScheme, StyleCatalog, StyleDef};
use iced::border::Radius;
use iced::{Font, Shadow};

use crate::notifications::{Level, Notification};

/// The style called `name` (key, display name or alias, any case); an unknown name falls back
/// to the default style and says so. An empty name is the default, silently.
pub(crate) fn resolve_style<'a>(
    name: &str,
    catalog: &'a StyleCatalog,
) -> (&'a StyleDef, Option<Notification>) {
    if name.trim().is_empty() {
        return (catalog.default_style(), None);
    }
    if let Some(style) = catalog.resolve(name) {
        return (style, None);
    }
    let fallback = catalog.default_style();
    let note = Notification::new(Level::Warn, "Unknown UI style").detail(format!(
        "\"{}\" is not a known UI style; using \"{}\"",
        name.trim(),
        fallback.key
    ));
    (fallback, Some(note))
}

/// Whether the scheme called `current` follows the UI style: it is the default scheme or the
/// palette paired with some style. A scheme the user picked on purpose does not.
pub(crate) fn follows_style(current: &str, styles: &StyleCatalog) -> bool {
    current == ColorScheme::default().name
        || styles
            .styles()
            .iter()
            .any(|s| s.paired_scheme.as_deref() == Some(current))
}

/// The scheme to switch to together with a switch to `to`: its paired palette, when the current
/// scheme follows the style and the paired one is available.
pub(crate) fn suggested_scheme(
    current: &str,
    to: &StyleDef,
    styles: &StyleCatalog,
    schemes: &fogu_core::SchemeCatalog,
) -> Option<String> {
    let paired = to.paired_scheme.as_deref()?;
    (paired != current && follows_style(current, styles) && schemes.by_name(paired).is_some())
        .then(|| paired.to_owned())
}

/// What a style needs from the environment besides the scheme.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Env {
    /// The terminal's monospace font (the chrome font of mono styles).
    pub(crate) mono: Font,
    /// Software rendering: shadows are capped, as tiny-skia draws them per pixel.
    pub(crate) software: bool,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            mono: Font::MONOSPACE,
            software: false,
        }
    }
}

/// Radii and strokes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    /// Cards, dialogs and floating panels.
    pub(crate) card: f32,
    /// Rows, inputs and buttons.
    pub(crate) control: f32,
    /// Tab headers (per corner).
    pub(crate) tab: Radius,
    /// Colour dots.
    pub(crate) dot: f32,
    /// Border width of panels, cards and fields.
    pub(crate) border: f32,
    /// Border width of a focused field.
    pub(crate) focus: f32,
}

/// Tinted, ready-to-use shadows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shadows {
    /// Docked panels.
    pub(crate) panel: Shadow,
    /// Menus, dialogs, toasts and floating cards.
    pub(crate) floating: Shadow,
    /// The active tab header.
    pub(crate) tab: Shadow,
}

/// Fonts and letter case of the chrome.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Type {
    /// Body text of the chrome.
    pub(crate) chrome: Font,
    /// Titles and emphasised labels.
    pub(crate) strong: Font,
    /// Active tab titles.
    pub(crate) active: Font,
    /// The terminal font, for previews.
    pub(crate) mono: Font,
    /// Section labels in upper case.
    pub(crate) upper: bool,
    /// The chrome is set in the terminal font.
    pub(crate) mono_chrome: bool,
}

impl Type {
    /// `label` in the style's case.
    pub(crate) fn label(self, label: &str) -> String {
        if self.upper {
            label.to_uppercase()
        } else {
            label.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "tests")]

    use fogu_core::SchemeCatalog;

    use super::*;

    #[test]
    fn names_resolve_and_unknown_falls_back() {
        let catalog = StyleCatalog::builtins();
        for style in catalog.styles() {
            let (found, note) = resolve_style(&style.key, &catalog);
            assert_eq!(found.key, style.key);
            assert!(note.is_none());
        }
        let (style, note) = resolve_style("nope", &catalog);
        assert_eq!(style.key, "fogu");
        assert!(note.is_some());
        let (style, note) = resolve_style("  ", &catalog);
        assert_eq!(style.key, "fogu");
        assert!(note.is_none());
        assert_eq!(resolve_style("Windows", &catalog).0.key, "fluent");
    }

    #[test]
    fn suggestion_only_replaces_a_scheme_that_follows_the_style() {
        let styles = StyleCatalog::builtins();
        let schemes = SchemeCatalog::builtins();
        let Some(paired) = styles
            .styles()
            .iter()
            .find(|s| s.paired_scheme.as_deref().is_some_and(|p| p != "Fogu Dark"))
        else {
            // No built-in pairs with a non-default palette yet.
            return;
        };
        let target = paired.paired_scheme.clone().unwrap();
        // From the default scheme: follow.
        assert_eq!(
            suggested_scheme("Fogu Dark", paired, &styles, &schemes),
            Some(target.clone())
        );
        // A scheme picked on purpose stays.
        assert_eq!(suggested_scheme("Dracula", paired, &styles, &schemes), None);
        // Already there: nothing to do.
        assert_eq!(suggested_scheme(&target, paired, &styles, &schemes), None);
    }
}
