//! Terminal colour schemes. Only the built-in dark default exists.

/// An sRGB colour, 8 bits per channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RgbColor {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl RgbColor {
    /// Builds a colour from a `0xRRGGBB` literal.
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
        }
    }
}

/// Foreground/background/cursor plus the 16 ANSI colours (0–7 normal, 8–15 bright).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorScheme {
    /// Display name.
    pub name: String,
    /// Default text colour.
    pub foreground: RgbColor,
    /// Default background colour.
    pub background: RgbColor,
    /// Cursor colour.
    pub cursor: RgbColor,
    /// ANSI colours 0–15.
    pub ansi: [RgbColor; 16],
}

impl Default for ColorScheme {
    /// "Fogu Dark" (see THIRD_PARTY_LICENSES for the origin of the colour values).
    fn default() -> Self {
        const ANSI: [u32; 16] = [
            0x00_0000, 0xff_615a, 0xb1_e969, 0xeb_d99c, 0x5d_a9f6, 0xe8_6aff, 0x82_fff7, 0xde_dacf,
            0x31_3131, 0xf5_8c80, 0xdd_f88f, 0xee_e5b2, 0xa5_c7ff, 0xdd_aaff, 0xb7_fff9, 0xff_ffff,
        ];
        Self {
            name: "Fogu Dark".to_owned(),
            foreground: RgbColor::from_hex(0xca_caca),
            background: RgbColor::from_hex(0x17_1717),
            cursor: RgbColor::from_hex(0xbb_bbbb),
            ansi: ANSI.map(RgbColor::from_hex),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_hex_splits_channels() {
        assert_eq!(
            RgbColor::from_hex(0x12_3456),
            RgbColor {
                r: 0x12,
                g: 0x34,
                b: 0x56
            }
        );
    }
}
