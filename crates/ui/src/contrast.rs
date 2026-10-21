//! Contrast on iced colours: measure (WCAG 2) and nudge a colour until it reads against a
//! background, so every style keeps text ≥ 4.5:1 and accents ≥ 3:1 over any scheme.

use fogu_core::contrast::{ratio_of, relative_luminance};
use iced::Color;

/// Body text over its background.
pub(crate) const TEXT: f32 = 4.5;
/// UI component boundaries and state indicators (WCAG 1.4.11).
pub(crate) const UI: f32 = 3.0;

/// Relative luminance of an opaque colour.
pub(crate) fn luminance(c: Color) -> f32 {
    relative_luminance(c.r, c.g, c.b)
}

/// `top` drawn over the opaque `under`.
pub(crate) fn composite(top: Color, under: Color) -> Color {
    let a = top.a.clamp(0.0, 1.0);
    Color::from_rgb(
        top.r * a + under.r * (1.0 - a),
        top.g * a + under.g * (1.0 - a),
        top.b * a + under.b * (1.0 - a),
    )
}

/// Contrast ratio of `a` over `b` (`a` may be translucent; `b` is taken as opaque).
pub(crate) fn ratio(a: Color, b: Color) -> f32 {
    ratio_of(luminance(composite(a, b)), luminance(b))
}

/// `color`, moved toward black or white (whichever gains contrast) just enough to reach `min`
/// against `against`. Unchanged when it already does; alpha is kept.
pub(crate) fn ensure(color: Color, against: Color, min: f32) -> Color {
    if ratio(color, against) >= min {
        return color;
    }
    let toward = if luminance(against) > 0.18 {
        Color::BLACK
    } else {
        Color::WHITE
    };
    let shift = |t: f32| {
        let mixed = crate::palette::mix(color, toward, t);
        Color {
            a: color.a,
            ..mixed
        }
    };
    if ratio(shift(1.0), against) < min {
        return shift(1.0);
    }
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for _ in 0..16 {
        let mid = f32::midpoint(lo, hi);
        if ratio(shift(mid), against) >= min {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    shift(hi)
}

/// The candidate with the highest contrast on `fill`.
pub(crate) fn best_on(fill: Color, candidates: &[Color]) -> Color {
    candidates
        .iter()
        .copied()
        .max_by(|a, b| ratio(*a, fill).total_cmp(&ratio(*b, fill)))
        .unwrap_or(Color::BLACK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_reaches_the_target_and_keeps_good_colours() {
        let white = Color::WHITE;
        let gray = Color::from_rgb8(0xAA, 0xAA, 0xAA);
        assert!(ratio(gray, white) < TEXT);
        let fixed = ensure(gray, white, TEXT);
        assert!(ratio(fixed, white) >= TEXT, "{}", ratio(fixed, white));
        // Darker, not lighter, on white.
        assert!(fixed.r < gray.r);
        let black = Color::from_rgb8(0x10, 0x10, 0x10);
        assert_eq!(ensure(white, black, TEXT), white);
        // On a dark background the nudge goes toward white.
        let dark_blue = Color::from_rgb8(0x20, 0x30, 0x80);
        let lifted = ensure(dark_blue, black, UI);
        assert!(ratio(lifted, black) >= UI);
        assert!(lifted.r > dark_blue.r);
    }

    #[test]
    fn best_on_picks_the_readable_candidate() {
        let yellow = Color::from_rgb8(0xFF, 0xE0, 0x40);
        assert_eq!(best_on(yellow, &[Color::WHITE, Color::BLACK]), Color::BLACK);
    }
}
