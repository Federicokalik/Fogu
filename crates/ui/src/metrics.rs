//! Terminal font resolution and cell metrics.
//!
//! The family is resolved explicitly through cosmic-text's fontdb (it ignores fontconfig
//! aliases), falling back to the first monospace face; `cell_w = advance('0')`,
//! `cell_h = ascent + descent + line_gap`, both rounded to whole device pixels.

use iced::Font;
use iced::advanced::graphics::text::cosmic_text::fontdb::{self, Database, Query};
use iced::advanced::graphics::text::cosmic_text::skrifa::instance::{LocationRef, Size};
use iced::advanced::graphics::text::cosmic_text::skrifa::{FontRef, MetadataProvider as _};
use iced::advanced::graphics::text::font_system;

/// Requested terminal font (fixed default).
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    /// Family name; `monospace` resolves to fontdb's monospace family.
    pub family: String,
    /// Size in points (1 pt = 4/3 logical px).
    pub size_pt: f32,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "monospace".to_owned(),
            size_pt: 12.0,
        }
    }
}

impl FontSpec {
    /// Size in logical pixels.
    pub fn size_px(&self) -> f32 {
        self.size_pt * 4.0 / 3.0
    }
}

/// Metrics of the resolved face in em (multiply by the font size in px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontUnits {
    /// Advance of `'0'` in em.
    pub advance: f32,
    /// `ascent + descent + line_gap` in em.
    pub line_height: f32,
}

impl FontUnits {
    /// Typical monospace proportions, used when no face could be resolved.
    pub const FALLBACK: Self = Self {
        advance: 0.6,
        line_height: 1.2,
    };
}

/// A resolved terminal font: what iced draws with plus its unit metrics.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedFont {
    /// Font handed to `fill_text`.
    pub font: Font,
    /// Unit metrics of the regular face.
    pub units: FontUnits,
}

/// Cell size for one font size and scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    /// Text size in logical px.
    pub font_size: f32,
    /// Cell width in logical px (a whole number of device px).
    pub cell_w: f32,
    /// Cell height in logical px (a whole number of device px).
    pub cell_h: f32,
    /// Cell width in device px.
    pub cell_w_px: u16,
    /// Cell height in device px.
    pub cell_h_px: u16,
}

impl CellMetrics {
    /// Scales `units` to `font_size` logical px at `scale` device px per logical px.
    pub fn new(units: FontUnits, font_size: f32, scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let (width, width_px) = round_to_device(units.advance * font_size, scale);
        let (height, height_px) = round_to_device(units.line_height * font_size, scale);
        Self {
            font_size,
            cell_w: width,
            cell_h: height,
            cell_w_px: width_px,
            cell_h_px: height_px,
        }
    }
}

/// Rounds a logical length to whole device pixels (at least 1). Returns the snapped logical
/// length and the device-pixel count.
pub fn round_to_device(logical: f32, scale: f32) -> (f32, u16) {
    let px = (logical * scale).round().clamp(1.0, f32::from(u16::MAX));
    // `px` is in 1..=u16::MAX after the clamp.
    #[expect(clippy::cast_sign_loss, reason = "clamped to 1..=u16::MAX above")]
    let px_u = px as u16;
    (px / scale, px_u)
}

/// Resolves `family` in iced's global font database and reads its metrics.
pub fn resolve(family: &str) -> ResolvedFont {
    let mut system = font_system()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let db = system.raw().db();
    let Some((id, name)) = find_face(db, family) else {
        tracing::warn!(
            family,
            "no monospace font found; using iced's default monospace"
        );
        return ResolvedFont {
            font: Font::MONOSPACE,
            units: FontUnits::FALLBACK,
        };
    };
    let units = db
        .with_face_data(id, read_units)
        .flatten()
        .unwrap_or_else(|| {
            tracing::warn!(font = name, "could not read font metrics; using defaults");
            FontUnits::FALLBACK
        });
    tracing::info!(requested = family, font = name, ?units, "terminal font");
    // iced fonts name their family with `&'static str`; one small leak per resolved family.
    let name: &'static str = Box::leak(name.into_boxed_str());
    ResolvedFont {
        font: Font::with_name(name),
        units,
    }
}

/// Explicit lookup of `family` (or fontdb's monospace family for `monospace`), else the first
/// monospace face in the database.
fn find_face(db: &Database, family: &str) -> Option<(fontdb::ID, String)> {
    let wanted = if family.eq_ignore_ascii_case("monospace") {
        fontdb::Family::Monospace
    } else {
        fontdb::Family::Name(family)
    };
    let id = db
        .query(&Query {
            families: &[wanted],
            ..Query::default()
        })
        .or_else(|| {
            let first = db
                .faces()
                .find(|face| face.monospaced && !face.post_script_name.contains("Emoji"))?;
            tracing::warn!(
                requested = family,
                "font family not found; using first monospace face"
            );
            Some(first.id)
        })?;
    let face = db.face(id)?;
    let name = face.families.first().map(|(name, _)| name.clone())?;
    Some((id, name))
}

fn read_units(data: &[u8], index: u32) -> Option<FontUnits> {
    let font = FontRef::from_index(data, index).ok()?;
    let size = Size::unscaled();
    let metrics = font.metrics(size, LocationRef::default());
    let upem = f32::from(metrics.units_per_em);
    let glyph = font.charmap().map('0')?;
    let advance = font
        .glyph_metrics(size, LocationRef::default())
        .advance_width(glyph)?;
    if upem <= 0.0 || advance <= 0.0 {
        return None;
    }
    // skrifa reports descent as a negative offset below the baseline.
    let line_height = (metrics.ascent - metrics.descent + metrics.leading) / upem;
    Some(FontUnits {
        advance: advance / upem,
        line_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_whole_device_pixels() {
        let cases = [
            // (logical, scale) → (logical, px)
            ((9.6, 1.0), (10.0, 10)),
            ((9.4, 1.0), (9.0, 9)),
            ((9.6, 2.0), (9.5, 19)),
            ((9.6, 1.25), (9.6, 12)),
            ((9.6, 1.5), (9.333_333, 14)),
            ((0.1, 1.0), (1.0, 1)),
        ];
        for ((logical, scale), (want_logical, want_px)) in cases {
            let (got_logical, got_px) = round_to_device(logical, scale);
            assert_eq!(got_px, want_px, "{logical} @ {scale}");
            assert!(
                (got_logical - want_logical).abs() < 1e-4,
                "{logical} @ {scale}"
            );
            // The snapped length is always a whole number of device pixels.
            let device = got_logical * scale;
            assert!((device - device.round()).abs() < 1e-4);
        }
    }

    #[test]
    fn cell_metrics_scale_units() {
        let units = FontUnits {
            advance: 0.6,
            line_height: 1.17,
        };
        let m = CellMetrics::new(units, 16.0, 1.0);
        assert_eq!((m.cell_w_px, m.cell_h_px), (10, 19));
        assert!((m.cell_w - 10.0).abs() < f32::EPSILON);
        let hidpi = CellMetrics::new(units, 16.0, 2.0);
        assert_eq!((hidpi.cell_w_px, hidpi.cell_h_px), (19, 37));
        // A broken scale factor falls back to 1.0.
        assert_eq!(CellMetrics::new(units, 16.0, 0.0), m);
    }

    #[test]
    fn twelve_points_is_sixteen_pixels() {
        assert!((FontSpec::default().size_px() - 16.0).abs() < f32::EPSILON);
    }
}
