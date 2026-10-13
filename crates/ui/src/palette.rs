//! Resolves the terminal's `Color::{Named, Indexed, Spec}` against the active [`ColorScheme`].
//!
//! The 269-entry table (16 ANSI, 6×6×6 cube, 24 grays, fg/bg/cursor, dim and bright variants) is
//! built once per scheme; the same table is handed to the `TermHandle` so OSC 4/10/11 queries
//! answer with the colours actually drawn.

use fogu_core::{ColorScheme, RgbColor};
use fogu_term::{Color, NamedColor, Palette, Rgb, default_palette};

/// The resolved colour table of one scheme.
#[derive(Clone, Debug)]
pub struct Colors {
    table: Box<Palette>,
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
        let dim = |c: Rgb| Rgb {
            r: c.r / 3 * 2,
            g: c.g / 3 * 2,
            b: c.b / 3 * 2,
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
        let fg = rgb(scheme.foreground);
        set(&mut table, NamedColor::Foreground as usize, fg);
        set(
            &mut table,
            NamedColor::Background as usize,
            rgb(scheme.background),
        );
        set(&mut table, NamedColor::Cursor as usize, rgb(scheme.cursor));
        set(&mut table, NamedColor::BrightForeground as usize, fg);
        set(&mut table, NamedColor::DimForeground as usize, dim(fg));
        Self { table }
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

    fn get(&self, index: usize) -> Rgb {
        self.table.get(index).copied().unwrap_or_default()
    }
}

fn set(table: &mut Palette, index: usize, value: Rgb) {
    if let Some(slot) = table.get_mut(index) {
        *slot = value;
    }
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

    #[test]
    fn blend_half_is_the_midpoint() {
        assert_eq!(blend_half(rgb(0xff_ffff), rgb(0x00_0000)), rgb(0x7f_7f7f));
        assert_eq!(blend_half(rgb(0x10_2030), rgb(0x10_2030)), rgb(0x10_2030));
    }
}
