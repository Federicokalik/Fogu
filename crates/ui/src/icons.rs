//! Title bar icons: 1.25 px strokes on a 16×16 grid, rasterized once per (icon, size, colour)
//! with tiny-skia at 2× and shown as images, so they share one weight and stay centred whatever
//! fonts the system has (glyphs like ☰ ☐ ✕ come from different fonts).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use iced::widget::{container, image};
use iced::{Color, Element, Length};
use tiny_skia::{LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::message::Message;

/// Pixels per logical pixel the icons are drawn at (sharp on 2× screens, smooth on 1×).
const SCALE: f32 = 2.0;
/// Stroke width in logical pixels, whatever the icon's size.
const STROKE: f32 = 1.25;

/// The icons the title bar uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Minimize,
    Maximize,
    Close,
    Plus,
    ChevronDown,
    ChevronRight,
    /// The profile sidebar (a window with a right-hand panel).
    Sidebar,
    /// The SFTP panel.
    Folder,
    /// Port forwarding (two opposite arrows).
    Ports,
    ArrowLeft,
    ArrowUp,
    Home,
    Refresh,
    Search,
    Settings,
}

/// `icon`, `size` px square in `color`, centred in all the space it is given.
pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> Element<'a, Message> {
    container(image(handle(icon, size, color)).width(size).height(size))
        .center(Length::Fill)
        .into()
}

/// The same icon in a fixed-size box, for rows where `icon`'s fill would push text away.
pub fn fixed<'a>(glyph: Icon, size: f32, color: Color) -> Element<'a, Message> {
    container(icon(glyph, size, color))
        .width(size)
        .height(size)
        .into()
}

type Key = (Icon, u32, [u8; 4]);

/// The cached image: a handle keeps its texture, a new one each frame would re-upload it.
fn handle(icon: Icon, size: f32, color: Color) -> image::Handle {
    static CACHE: OnceLock<Mutex<HashMap<Key, image::Handle>>> = OnceLock::new();
    let key = (icon, size.to_bits(), color.into_rgba8());
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    cache
        .entry(key)
        .or_insert_with(|| rasterize(icon, size, color))
        .clone()
}

/// Straight (not premultiplied) RGBA pixels of `icon`, `px` pixels square.
pub(crate) fn pixels(icon: Icon, px: u32, color: Color) -> Vec<u8> {
    let Some(mut pixmap) = Pixmap::new(px, px) else {
        return Vec::new();
    };
    let [red, green, blue, alpha] = color.into_rgba8();
    let mut paint = Paint::default();
    paint.set_color_rgba8(red, green, blue, alpha);
    paint.anti_alias = true;
    let unit = f32::from(u16::try_from(px).unwrap_or(u16::MAX)) / 16.0;
    let stroke = Stroke {
        width: STROKE * SCALE,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    if let Some(path) = path(icon, unit) {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let straight = p.demultiply();
            [
                straight.red(),
                straight.green(),
                straight.blue(),
                straight.alpha(),
            ]
        })
        .collect()
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "icon sizes are small positive numbers of pixels"
)]
fn rasterize(icon: Icon, size: f32, color: Color) -> image::Handle {
    let px = (size * SCALE).round().clamp(1.0, 256.0) as u32;
    let rgba = pixels(icon, px, color);
    if rgba.is_empty() {
        return image::Handle::from_rgba(1, 1, vec![0; 4]);
    }
    image::Handle::from_rgba(px, px, rgba)
}

/// The icon's outline on the 16×16 grid, scaled by `u` pixels per grid unit.
#[expect(
    clippy::too_many_lines,
    reason = "one match holding the complete icon set; splitting it would hide the glyph shapes"
)]
fn path(icon: Icon, u: f32) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    let mut line = |points: &[(f32, f32)]| {
        if let Some(&(x, y)) = points.first() {
            b.move_to(x * u, y * u);
        }
        for &(x, y) in points.iter().skip(1) {
            b.line_to(x * u, y * u);
        }
    };
    match icon {
        Icon::Minimize => line(&[(3.0, 8.5), (13.0, 8.5)]),
        Icon::Maximize => line(&[
            (3.5, 3.5),
            (12.5, 3.5),
            (12.5, 12.5),
            (3.5, 12.5),
            (3.5, 3.5),
        ]),
        Icon::Close => {
            line(&[(4.0, 4.0), (12.0, 12.0)]);
            line(&[(12.0, 4.0), (4.0, 12.0)]);
        }
        Icon::Plus => {
            line(&[(8.0, 3.0), (8.0, 13.0)]);
            line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
        Icon::ChevronDown => line(&[(5.0, 6.5), (8.0, 9.5), (11.0, 6.5)]),
        Icon::ChevronRight => line(&[(6.5, 5.0), (9.5, 8.0), (6.5, 11.0)]),
        Icon::ArrowLeft => {
            line(&[(13.0, 8.0), (3.0, 8.0)]);
            line(&[(6.5, 4.5), (3.0, 8.0), (6.5, 11.5)]);
        }
        Icon::ArrowUp => {
            line(&[(8.0, 13.0), (8.0, 3.0)]);
            line(&[(4.5, 6.5), (8.0, 3.0), (11.5, 6.5)]);
        }
        Icon::Home => {
            line(&[(2.5, 7.5), (8.0, 2.5), (13.5, 7.5)]);
            line(&[(4.5, 6.5), (4.5, 13.0), (11.5, 13.0), (11.5, 6.5)]);
        }
        Icon::Refresh => {
            line(&[(3.5, 6.0), (4.6, 3.6), (7.2, 2.6), (10.2, 3.2), (12.2, 5.2)]);
            line(&[(12.5, 2.7), (12.2, 5.2), (9.7, 5.6)]);
            line(&[
                (12.5, 10.0),
                (11.4, 12.4),
                (8.8, 13.4),
                (5.8, 12.8),
                (3.8, 10.8),
            ]);
            line(&[(3.5, 13.3), (3.8, 10.8), (6.3, 10.4)]);
        }
        Icon::Settings => {
            line(&[(2.5, 4.5), (13.5, 4.5)]);
            line(&[(2.5, 8.0), (13.5, 8.0)]);
            line(&[(2.5, 11.5), (13.5, 11.5)]);
            line(&[
                (11.2, 4.5),
                (10.5, 5.5),
                (9.3, 5.0),
                (9.3, 4.0),
                (10.5, 3.5),
                (11.2, 4.5),
            ]);
            line(&[
                (4.2, 8.0),
                (4.9, 9.0),
                (6.1, 8.5),
                (6.1, 7.5),
                (4.9, 7.0),
                (4.2, 8.0),
            ]);
            line(&[
                (9.2, 11.5),
                (9.9, 12.5),
                (11.1, 12.0),
                (11.1, 11.0),
                (9.9, 10.5),
                (9.2, 11.5),
            ]);
        }
        Icon::Search => {
            line(&[(10.8, 10.8), (14.0, 14.0)]);
            line(&[
                (7.0, 2.5),
                (9.4, 3.5),
                (10.5, 5.8),
                (10.0, 8.3),
                (8.2, 10.2),
                (5.7, 10.7),
                (3.4, 9.6),
                (2.2, 7.3),
                (2.5, 4.8),
                (4.3, 3.0),
                (7.0, 2.5),
            ]);
        }
        Icon::Sidebar => {
            line(&[
                (2.0, 3.0),
                (14.0, 3.0),
                (14.0, 13.0),
                (2.0, 13.0),
                (2.0, 3.0),
            ]);
            line(&[(10.0, 3.0), (10.0, 13.0)]);
        }
        Icon::Folder => line(&[
            (2.0, 4.0),
            (6.0, 4.0),
            (7.5, 5.5),
            (14.0, 5.5),
            (14.0, 12.5),
            (2.0, 12.5),
            (2.0, 4.0),
        ]),
        Icon::Ports => {
            line(&[(2.5, 6.0), (13.0, 6.0)]);
            line(&[(10.5, 3.5), (13.0, 6.0), (10.5, 8.5)]);
            line(&[(13.5, 10.5), (3.0, 10.5)]);
            line(&[(5.5, 8.0), (3.0, 10.5), (5.5, 13.0)]);
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_draws_opaque_pixels_in_its_colour() {
        let red = Color::from_rgb8(0xff, 0, 0);
        for icon in [
            Icon::Minimize,
            Icon::Maximize,
            Icon::Close,
            Icon::Plus,
            Icon::ChevronDown,
            Icon::ChevronRight,
            Icon::Sidebar,
            Icon::Folder,
            Icon::Ports,
            Icon::ArrowLeft,
            Icon::ArrowUp,
            Icon::Home,
            Icon::Refresh,
            Icon::Search,
        ] {
            let rgba = pixels(icon, 32, red);
            assert_eq!(rgba.len(), 32 * 32 * 4, "{icon:?}");
            let solid = rgba.chunks(4).filter(|p| p == &[0xff, 0, 0, 0xff]).count();
            assert!(solid > 20, "{icon:?}: {solid} solid pixels");
        }
    }
}
