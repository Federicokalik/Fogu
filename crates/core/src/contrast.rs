//! WCAG 2 relative luminance and contrast ratio, used to keep UI styles and colour schemes
//! readable (text ≥ 4.5:1, UI component boundaries ≥ 3:1).

use crate::scheme::RgbColor;

/// Relative luminance of an sRGB colour given as `0.0..=1.0` channels.
#[must_use]
pub fn relative_luminance(r: f32, g: f32, b: f32) -> f32 {
    let linear = |c: f32| {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// Contrast ratio between two luminances, `1.0..=21.0`, symmetric.
#[must_use]
pub fn ratio_of(a: f32, b: f32) -> f32 {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

impl RgbColor {
    /// Relative luminance (WCAG 2).
    #[must_use]
    pub fn luminance(self) -> f32 {
        let channel = |c: u8| f32::from(c) / 255.0;
        relative_luminance(channel(self.r), channel(self.g), channel(self.b))
    }
}

/// Contrast ratio between two colours, `1.0..=21.0`.
#[must_use]
pub fn ratio(a: RgbColor, b: RgbColor) -> f32 {
    ratio_of(a.luminance(), b.luminance())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn extremes_and_identity() {
        let black = RgbColor::from_hex(0x00_0000);
        let white = RgbColor::from_hex(0xff_ffff);
        assert!(close(ratio(black, white), 21.0));
        assert!(close(ratio(white, black), 21.0));
        assert!(close(ratio(white, white), 1.0));
    }

    #[test]
    fn known_gray_on_white() {
        // #777777 on white is the classic "just below AA" pair.
        let r = ratio(RgbColor::from_hex(0x77_7777), RgbColor::from_hex(0xff_ffff));
        assert!(close(r, 4.48), "{r}");
    }
}
