//! Resolves the terminal's `Color::{Named, Indexed, Spec}` against the active [`ColorScheme`].
//!
//! The 269-entry table (16 ANSI, 6×6×6 cube, 24 grays, fg/bg/cursor, dim and bright variants) is
//! built once per scheme; the same table is handed to the `TermHandle` so OSC 4/10/11 queries
//! answer with the colours actually drawn.

use fogu_core::{ColorScheme, RgbColor};
use fogu_term::{Color, NamedColor, Palette, Rgb, default_palette, dim};

use fogu_core::StyleDef;
use fogu_core::style::{
    BarSpec, ChromeFont, DialogSpec, InteractionSpec, LabelCase, LayoutSpec, Mix, Role, ShadowSpec,
    ShadowTint, TabSpec, Weight,
};

use crate::contrast;
use crate::design::{Env, Shadows, Shape, Type};

/// The resolved colour table of one scheme.
#[derive(Clone, Debug)]
pub struct Colors {
    table: Box<Palette>,
    /// Text drawn under a filled block cursor (the palette has no slot for it).
    cursor_text: Rgb,
}

impl Colors {
    /// Builds the full table for `scheme`.
    pub fn new(scheme: &ColorScheme) -> Self {
        // The default table already holds the computed 256-colour cube and gray ramp.
        let mut table = default_palette();
        let rgb = |c: RgbColor| Rgb {
            r: c.r,
            g: c.g,
            b: c.b,
        };
        for (i, color) in scheme.ansi.iter().enumerate() {
            set(&mut table, i, rgb(*color));
            if i < 8 {
                set(
                    &mut table,
                    NamedColor::DimBlack as usize + i,
                    dim(rgb(*color)),
                );
            }
        }
        let fg = rgb(scheme.fg);
        set(&mut table, NamedColor::Foreground as usize, fg);
        set(&mut table, NamedColor::Background as usize, rgb(scheme.bg));
        set(&mut table, NamedColor::Cursor as usize, rgb(scheme.cursor));
        set(&mut table, NamedColor::BrightForeground as usize, fg);
        set(&mut table, NamedColor::DimForeground as usize, dim(fg));
        Self {
            table,
            cursor_text: rgb(scheme.cursor_text),
        }
    }

    /// The table in the form `TermHandle::set_palette` takes.
    pub fn palette(&self) -> Box<Palette> {
        self.table.clone()
    }

    /// Resolves a cell colour; `bold` maps ANSI 0–7 to their bright variants (the usual
    /// "bold is bright" behaviour).
    pub fn resolve(&self, color: Color, bold: bool) -> Rgb {
        let index = match color {
            Color::Spec(rgb) => return rgb,
            Color::Named(named) => named as usize,
            Color::Indexed(i) => usize::from(i),
        };
        let index = match index {
            0..8 if bold => index + 8,
            i if i == NamedColor::Foreground as usize && bold => {
                NamedColor::BrightForeground as usize
            }
            i => i,
        };
        self.get(index)
    }

    /// Default background.
    pub fn background(&self) -> Rgb {
        self.get(NamedColor::Background as usize)
    }

    /// Default foreground.
    pub fn foreground(&self) -> Rgb {
        self.get(NamedColor::Foreground as usize)
    }

    /// Cursor colour.
    pub fn cursor(&self) -> Rgb {
        self.get(NamedColor::Cursor as usize)
    }

    /// Colour of the character under a filled block cursor.
    pub fn cursor_text(&self) -> Rgb {
        self.cursor_text
    }

    fn get(&self, index: usize) -> Rgb {
        self.table.get(index).copied().unwrap_or_default()
    }
}

fn set(table: &mut Palette, index: usize, value: Rgb) {
    if let Some(slot) = table.get_mut(index) {
        *slot = value;
    }
}

/// The window chrome's colours and tokens (menus, panels, title bar), derived from the UI style
/// over the terminal scheme so dark and light schemes both work.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Look {
    /// Menus, dialogs, the picker.
    pub(crate) bg: iced::Color,
    pub(crate) fg: iced::Color,
    /// Muted text.
    pub(crate) dim: iced::Color,
    /// Links, selection, active toggles. Style default, overridable in config.
    pub(crate) accent: iced::Color,
    /// Tinted accent for selected rows and focus rings.
    pub(crate) accent_soft: iced::Color,
    /// Text on an accent fill.
    pub(crate) on_accent: iced::Color,
    /// The accent as a text colour, readable on panels and menus.
    pub(crate) accent_text: iced::Color,
    /// Focus rings: the secondary accent when the style has one, else the accent.
    pub(crate) focus: iced::Color,
    /// The terminal background (the active tab).
    pub(crate) base: iced::Color,
    /// The title / tab bar.
    pub(crate) raised: iced::Color,
    /// Docked panel surface.
    pub(crate) panel: iced::Color,
    /// Behind islands.
    pub(crate) canvas: iced::Color,
    /// 1 px borders and the sidebar handle.
    pub(crate) border: iced::Color,
    /// Split dividers between panes.
    pub(crate) divider: iced::Color,
    /// Hovered rows and buttons.
    pub(crate) hover: iced::Color,
    /// Close-button hover, errors.
    pub(crate) danger: iced::Color,
    /// The second accent (gradient ends), the accent when the style has none.
    pub(crate) secondary: iced::Color,
    /// The scheme is dark.
    pub(crate) dark: bool,
    /// Radii and strokes.
    pub(crate) shape: Shape,
    /// Shadows.
    pub(crate) shadow: Shadows,
    /// Fonts and letter case.
    pub(crate) text: Type,
    /// The tab bar.
    pub(crate) bar: BarSpec,
    /// Tab headers.
    pub(crate) tabs: TabSpec,
    /// Layout and density.
    pub(crate) layout: LayoutSpec,
    /// Interaction language.
    pub(crate) interaction: InteractionSpec,
    /// Small dialogs.
    pub(crate) dialog: DialogSpec,
}

impl Look {
    #[expect(
        clippy::too_many_lines,
        reason = "one assignment per token; splitting it would scatter the recipe order"
    )]
    pub(crate) fn new(
        colors: &Colors,
        style: &StyleDef,
        accent: Option<RgbColor>,
        env: Env,
    ) -> Self {
        let base = to_iced(colors.background());
        let fg = to_iced(colors.foreground());
        let dark = base.r * 0.299 + base.g * 0.587 + base.b * 0.114 < 0.5;
        let rgb = |c: RgbColor| iced::Color::from_rgb8(c.r, c.g, c.b);
        // Accents fill indicators and selections: at least 3:1 against the terminal colour.
        let accent = contrast::ensure(
            rgb(accent.unwrap_or(style.color.accent)),
            base,
            contrast::UI,
        );
        let secondary = style.color.secondary.map_or(accent, rgb);
        let surface = &style.surface;
        let mut known = Known {
            base,
            fg,
            accent,
            bg: None,
            raised: None,
            panel: None,
        };
        let bg = known.mix(surface.bg, dark);
        known.bg = Some(bg);
        let raised = known.mix(surface.raised, dark);
        known.raised = Some(raised);
        let panel = known.mix(surface.panel, dark);
        known.panel = Some(panel);
        let canvas = known.mix(surface.canvas, dark);
        let border = known.mix(surface.border, dark);
        let divider = known.mix(surface.divider, dark);
        // Muted text still reads as text on every chrome surface.
        let dim = contrast::ensure(
            contrast::ensure(known.mix(surface.dim, dark), panel, contrast::TEXT),
            bg,
            contrast::TEXT,
        );
        // Chrome text: the scheme's foreground, nudged where a low-contrast scheme (Solarized)
        // would fall under 4.5:1 on a chrome surface. The terminal keeps the scheme's own.
        let fg = [bg, raised, panel].into_iter().fold(fg, |fg, surface| {
            contrast::ensure(fg, surface, contrast::TEXT)
        });
        let shadow = |spec: ShadowSpec| {
            let tint = match spec.tint {
                ShadowTint::Black => iced::Color::BLACK,
                ShadowTint::Accent => mix(iced::Color::BLACK, accent, spec.mix),
                ShadowTint::Secondary => mix(iced::Color::BLACK, secondary, spec.mix),
            };
            let blur = if env.software {
                spec.blur.min(SOFTWARE_BLUR)
            } else {
                spec.blur
            };
            iced::Shadow {
                color: iced::Color {
                    a: spec.alpha,
                    ..tint
                },
                offset: iced::Vector::new(spec.x, spec.y),
                blur_radius: blur,
            }
        };
        let chrome = match style.text.chrome {
            ChromeFont::Ui => crate::UI_FONT,
            ChromeFont::Mono => env.mono,
        };
        let strong = iced::Font {
            weight: iced::font::Weight::Medium,
            ..chrome
        };
        let shape = &style.shape;
        let on_accent =
            contrast::best_on(accent, &[base, fg, iced::Color::BLACK, iced::Color::WHITE]);
        let accent_text = contrast::ensure(
            contrast::ensure(accent, panel, contrast::TEXT),
            bg,
            contrast::TEXT,
        );
        Self {
            bg,
            fg,
            dim,
            accent,
            accent_soft: iced::Color {
                a: if dark {
                    surface.soft_dark
                } else {
                    surface.soft_light
                },
                ..accent
            },
            on_accent,
            accent_text,
            focus: secondary,
            base,
            raised,
            panel,
            canvas,
            border,
            divider,
            hover: iced::Color {
                a: surface.hover,
                ..fg
            },
            danger: rgb(style.color.danger),
            secondary,
            dark,
            shape: Shape {
                card: shape.card,
                control: shape.control,
                tab: iced::border::Radius {
                    top_left: shape.tab_top,
                    top_right: shape.tab_top,
                    bottom_right: shape.tab_bottom,
                    bottom_left: shape.tab_bottom,
                },
                dot: shape.dot,
                border: shape.border,
                focus: shape.focus,
            },
            shadow: Shadows {
                panel: shadow(style.shadow.panel),
                floating: shadow(style.shadow.floating),
                tab: shadow(style.shadow.tab),
            },
            text: Type {
                chrome,
                strong,
                active: match style.text.active {
                    Weight::Regular => chrome,
                    Weight::Medium => strong,
                },
                mono: env.mono,
                upper: style.text.labels == LabelCase::Upper,
                mono_chrome: style.text.chrome == ChromeFont::Mono,
            },
            bar: style.bar,
            tabs: style.tabs,
            layout: style.layout,
            interaction: style.interaction,
            dialog: style.dialog,
        }
    }

    /// List row height from the style's density.
    pub(crate) fn row_h(&self) -> f32 {
        self.layout.density.row_height()
    }

    /// The iced theme for stock widgets (text fields, sliders, scrollbars, tooltips), so none
    /// of them is drawn in iced's default light colours.
    pub(crate) fn theme(&self, colors: &Colors) -> iced::Theme {
        let ansi = |c| to_iced(colors.resolve(Color::Named(c), false));
        iced::Theme::custom(
            "fogu",
            iced::theme::Palette {
                background: self.bg,
                text: self.fg,
                primary: self.accent,
                success: ansi(NamedColor::Green),
                warning: ansi(NamedColor::Yellow),
                danger: self.danger,
            },
        )
    }
}

/// Largest shadow blur under software rendering.
const SOFTWARE_BLUR: f32 = 6.0;

/// Colours a [`Mix`] can refer to; roles not resolved yet stand for the base.
struct Known {
    base: iced::Color,
    fg: iced::Color,
    accent: iced::Color,
    bg: Option<iced::Color>,
    raised: Option<iced::Color>,
    panel: Option<iced::Color>,
}

impl Known {
    fn role(&self, role: Role) -> iced::Color {
        match role {
            Role::Base => self.base,
            Role::Fg => self.fg,
            Role::Accent => self.accent,
            Role::Bg => self.bg.unwrap_or(self.base),
            Role::Raised => self.raised.unwrap_or(self.base),
            Role::Panel => self.panel.unwrap_or(self.base),
            Role::Black => iced::Color::BLACK,
            Role::White => iced::Color::WHITE,
        }
    }

    fn mix(&self, recipe: Mix, dark: bool) -> iced::Color {
        let t = if dark { recipe.dark } else { recipe.light };
        mix(self.role(recipe.from), self.role(recipe.toward), t)
    }
}

/// Linear mix of two colours (`t` = share of `b`).
pub(crate) fn mix(a: iced::Color, b: iced::Color, t: f32) -> iced::Color {
    iced::Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

/// Converts to an iced colour.
pub fn to_iced(rgb: Rgb) -> iced::Color {
    iced::Color::from_rgb8(rgb.r, rgb.g, rgb.b)
}

/// Mixes `a` and `b` half and half (used for SGR 2 "dim").
pub fn blend_half(a: Rgb, b: Rgb) -> Rgb {
    let mix = u8::midpoint;
    Rgb {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: u32) -> Rgb {
        let c = RgbColor::from_hex(hex);
        Rgb {
            r: c.r,
            g: c.g,
            b: c.b,
        }
    }

    #[test]
    fn resolves_against_the_scheme() {
        let colors = Colors::new(&ColorScheme::default());
        let cases = [
            (Color::Named(NamedColor::Red), false, rgb(0xff_615a)),
            (Color::Named(NamedColor::Red), true, rgb(0xf5_8c80)),
            (Color::Indexed(1), true, rgb(0xf5_8c80)),
            (Color::Indexed(9), false, rgb(0xf5_8c80)),
            (Color::Named(NamedColor::Foreground), false, rgb(0xca_caca)),
            (Color::Named(NamedColor::Background), true, rgb(0x17_1717)),
            (Color::Named(NamedColor::Cursor), false, rgb(0xbb_bbbb)),
            // Cube: 16 + 36*1 + 6*2 + 3 → (95, 135, 175).
            (Color::Indexed(67), false, rgb(0x5f_87af)),
            // Gray ramp start/end.
            (Color::Indexed(232), false, rgb(0x08_0808)),
            (Color::Indexed(255), false, rgb(0xee_eeee)),
            (Color::Indexed(200), true, rgb(0xff_00d7)),
            (Color::Spec(rgb(0x12_3456)), true, rgb(0x12_3456)),
        ];
        for (color, bold, want) in cases {
            assert_eq!(colors.resolve(color, bold), want, "{color:?} bold={bold}");
        }
        assert_eq!(colors.cursor_text(), rgb(0x17_1717));
    }

    #[test]
    fn dim_variants_follow_the_scheme() {
        let colors = Colors::new(&ColorScheme::default());
        // 0xff_615a * 2/3 per channel.
        assert_eq!(
            colors.resolve(Color::Named(NamedColor::DimRed), false),
            rgb(0xaa_403c)
        );
        assert_eq!(colors.palette().len(), 269);
    }

    fn color(c: iced::Color) -> String {
        format!("{:.4} {:.4} {:.4} {:.4}", c.r, c.g, c.b, c.a)
    }

    fn shadow(s: iced::Shadow) -> String {
        format!(
            "{} @ {:.1},{:.1} blur {:.1}",
            color(s.color),
            s.offset.x,
            s.offset.y,
            s.blur_radius
        )
    }

    /// Every chrome colour and shape token of every preset over every built-in scheme, so a
    /// refactor of how they are derived shows up as a snapshot diff.
    #[test]
    fn look_tokens_snapshot() {
        use std::fmt::Write as _;
        let mut out = String::new();
        for scheme in ColorScheme::builtins() {
            let colors = Colors::new(scheme);
            for style in fogu_core::StyleCatalog::builtins().styles() {
                let look = Look::new(&colors, style, None, Env::default());
                let _ = writeln!(out, "## {} / {}", scheme.name, style.key);
                for (name, value) in [
                    ("bg", look.bg),
                    ("fg", look.fg),
                    ("dim", look.dim),
                    ("accent", look.accent),
                    ("accent_soft", look.accent_soft),
                    ("base", look.base),
                    ("raised", look.raised),
                    ("panel", look.panel),
                    ("border", look.border),
                    ("hover", look.hover),
                    ("danger", look.danger),
                ] {
                    let _ = writeln!(out, "{name}: {}", color(value));
                }
                let _ = writeln!(
                    out,
                    "radius: {} small: {} gutter: {}",
                    look.shape.card, look.shape.control, look.layout.gutter
                );
                let _ = writeln!(
                    out,
                    "panel shadow: {}",
                    shadow(crate::controls::panel(look).shadow)
                );
                let _ = writeln!(
                    out,
                    "floating shadow: {}",
                    shadow(crate::controls::floating(look).shadow)
                );
            }
        }
        insta::assert_snapshot!(out);
    }

    /// Every built-in style stays readable over its paired palette and the classic ones.
    #[test]
    fn styles_keep_contrast_over_every_palette() {
        use crate::contrast::{TEXT, UI, ratio};
        let styles = fogu_core::StyleCatalog::builtins();
        for style in styles.styles() {
            let mut names = vec!["Fogu Dark", "Fogu Light", "Solarized Dark", "Dracula"];
            names.extend(style.paired_scheme.as_deref());
            for name in names {
                let scheme = ColorScheme::by_name(name).unwrap_or_default();
                let look = Look::new(&Colors::new(&scheme), style, None, Env::default());
                let at = format!("{} over {name}", style.key);
                for (surface, color) in [
                    ("bg", look.bg),
                    ("raised", look.raised),
                    ("panel", look.panel),
                ] {
                    assert!(ratio(look.fg, color) >= TEXT, "{at}: fg on {surface}");
                }
                assert!(ratio(look.dim, look.panel) >= TEXT, "{at}: dim on panel");
                assert!(ratio(look.dim, look.bg) >= TEXT, "{at}: dim on bg");
                assert!(ratio(look.accent, look.base) >= UI, "{at}: accent on base");
                assert!(
                    ratio(look.accent_text, look.panel) >= TEXT,
                    "{at}: accent text"
                );
                assert!(
                    ratio(look.on_accent, look.accent) >= TEXT,
                    "{at}: text on accent"
                );
            }
        }
    }

    #[test]
    fn look_stays_small_enough_to_copy() {
        assert!(
            std::mem::size_of::<Look>() <= 768,
            "{}",
            std::mem::size_of::<Look>()
        );
    }

    #[test]
    fn blend_half_is_the_midpoint() {
        assert_eq!(blend_half(rgb(0xff_ffff), rgb(0x00_0000)), rgb(0x7f_7f7f));
        assert_eq!(blend_half(rgb(0x10_2030), rgb(0x10_2030)), rgb(0x10_2030));
    }
}
