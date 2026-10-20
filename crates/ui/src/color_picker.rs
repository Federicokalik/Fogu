//! A colour picker you point at: a saturation × brightness square for the current hue over a
//! rainbow hue bar. Click or drag in either; every move sends `on_change(colour)`.

use std::f32::consts::{FRAC_PI_2, PI};

use fogu_core::RgbColor;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse};
use iced::gradient::Linear;
use iced::{Background, Border, Color, Element, Event, Length, Point, Rectangle, Size};

use crate::context_menu::{from_hsv, to_hsv, to_iced};

/// Height of the saturation × brightness square.
const SQUARE_H: f32 = 120.0;
/// Space between the square and the hue bar.
const GAP: f32 = 8.0;
/// Height of the hue bar.
const BAR_H: f32 = 14.0;
const RADIUS: f32 = 4.0;
/// Where the picker starts when there is no colour yet.
const START: RgbColor = RgbColor::from_hex(0x00_90FF);

/// Which part a drag started in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Square,
    Hue,
}

#[derive(Debug, Default)]
struct State {
    drag: Option<Part>,
    /// The hue last picked: greys and black have none of their own, so it is kept here.
    hue: Option<f32>,
}

pub struct ColorPicker<'a, M> {
    color: Option<RgbColor>,
    width: f32,
    on_change: Box<dyn Fn(RgbColor) -> M + 'a>,
}

impl<M> std::fmt::Debug for ColorPicker<'_, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorPicker")
            .field("color", &self.color)
            .field("width", &self.width)
            .finish_non_exhaustive()
    }
}

/// A `size` px square filled with the picked `color` (an outline only while there is none).
pub fn preview<'a, M: 'a>(color: Option<RgbColor>, size: f32, outline: Color) -> Element<'a, M> {
    iced::widget::container(iced::widget::space())
        .width(size)
        .height(size)
        .style(move |_theme: &iced::Theme| iced::widget::container::Style {
            background: color.map(|c| to_iced(c).into()),
            border: Border {
                color: outline,
                width: 1.0,
                radius: RADIUS.into(),
            },
            ..iced::widget::container::Style::default()
        })
        .into()
}

/// A picker `width` px wide showing `color` (`None` starts from a default blue).
pub fn color_picker<'a, M>(
    color: Option<RgbColor>,
    width: f32,
    on_change: impl Fn(RgbColor) -> M + 'a,
) -> ColorPicker<'a, M> {
    ColorPicker {
        color,
        width,
        on_change: Box::new(on_change),
    }
}

impl<M> ColorPicker<'_, M> {
    /// Hue, saturation and value shown: the remembered hue when the colour has none.
    fn hsv(&self, state: &State) -> (f32, f32, f32) {
        let (h, s, v) = to_hsv(self.color.unwrap_or(START));
        let hue = if s == 0.0 || v == 0.0 {
            state.hue.unwrap_or(h)
        } else {
            h
        };
        (hue, s, v)
    }
}

/// The square and the hue bar inside the widget's `bounds`.
fn parts(bounds: Rectangle) -> (Rectangle, Rectangle) {
    let square = Rectangle {
        height: SQUARE_H,
        ..bounds
    };
    let bar = Rectangle {
        y: bounds.y + SQUARE_H + GAP,
        height: BAR_H,
        ..bounds
    };
    (square, bar)
}

/// `at` as 0..=1 fractions across `r` (clamped, so a drag may leave the area).
fn fraction(r: Rectangle, at: Point) -> (f32, f32) {
    (
        ((at.x - r.x) / r.width).clamp(0.0, 1.0),
        ((at.y - r.y) / r.height).clamp(0.0, 1.0),
    )
}

fn quad(bounds: Rectangle, border: Border) -> Quad {
    Quad {
        bounds,
        border,
        ..Quad::default()
    }
}

/// A round marker centred on `at`: white ring inside a dark one, visible on any colour.
fn marker(renderer: &mut iced::Renderer, at: Point, w: f32, h: f32) {
    let rect = |pad: f32| Rectangle {
        x: at.x - w / 2.0 - pad,
        y: at.y - h / 2.0 - pad,
        width: w + 2.0 * pad,
        height: h + 2.0 * pad,
    };
    let ring = |color: Color, radius: f32| Border {
        color,
        width: 2.0,
        radius: radius.into(),
    };
    let dark = Color::from_rgba(0.0, 0.0, 0.0, 0.6);
    renderer::Renderer::fill_quad(
        renderer,
        quad(rect(1.0), ring(dark, w / 2.0 + 1.0)),
        Color::TRANSPARENT,
    );
    renderer::Renderer::fill_quad(
        renderer,
        quad(rect(-1.0), ring(Color::WHITE, w / 2.0)),
        Color::TRANSPARENT,
    );
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for ColorPicker<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed(self.width),
            Length::Fixed(SQUARE_H + GAP + BAR_H),
        )
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(
            Length::Fixed(self.width),
            Length::Fixed(SQUARE_H + GAP + BAR_H),
            Size::ZERO,
        ))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let (square, bar) = parts(layout.bounds());
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.drag = match cursor.position() {
                    Some(at) if square.contains(at) => Some(Part::Square),
                    Some(at) if bar.contains(at) => Some(Part::Hue),
                    _ => return,
                };
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.drag.is_some() => {}
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.drag.is_some() =>
            {
                state.drag = None;
                shell.capture_event();
                return;
            }
            _ => return,
        }
        let (Some(part), Some(at)) = (state.drag, cursor.position()) else {
            return;
        };
        let (hue, s, v) = self.hsv(state);
        let picked = match part {
            Part::Square => {
                let (x, y) = fraction(square, at);
                from_hsv(hue, x * 100.0, (1.0 - y) * 100.0)
            }
            Part::Hue => {
                let hue = fraction(bar, at).0 * 359.0;
                state.hue = Some(hue);
                from_hsv(hue, s, v)
            }
        };
        state.hue.get_or_insert(hue);
        shell.publish((self.on_change)(picked));
        shell.capture_event();
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let (square, bar) = parts(layout.bounds());
        let dragging = tree.state.downcast_ref::<State>().drag.is_some();
        match cursor.position() {
            _ if dragging => mouse::Interaction::Crosshair,
            Some(at) if square.contains(at) => mouse::Interaction::Crosshair,
            Some(at) if bar.contains(at) => mouse::Interaction::Pointer,
            _ => mouse::Interaction::None,
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let (hue, s, v) = self.hsv(tree.state.downcast_ref::<State>());
        let (square, bar) = parts(layout.bounds());
        let rounded = Border::default().rounded(RADIUS);
        // The pure hue, faded to white leftwards, then darkened to black downwards.
        let pure = to_iced(from_hsv(hue, 100.0, 100.0));
        renderer::Renderer::fill_quad(renderer, quad(square, rounded), pure);
        let white = Linear::new(FRAC_PI_2)
            .add_stop(0.0, Color::WHITE)
            .add_stop(1.0, Color::TRANSPARENT);
        renderer::Renderer::fill_quad(renderer, quad(square, rounded), Background::from(white));
        let black = Linear::new(PI)
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, Color::BLACK);
        renderer::Renderer::fill_quad(renderer, quad(square, rounded), Background::from(black));
        let mut rainbow = Linear::new(FRAC_PI_2);
        for i in 0..=6_u8 {
            let at = f32::from(i) / 6.0;
            rainbow = rainbow.add_stop(at, to_iced(from_hsv(at * 360.0, 100.0, 100.0)));
        }
        renderer::Renderer::fill_quad(renderer, quad(bar, rounded), Background::from(rainbow));
        let spot = Point::new(
            square.x + s / 100.0 * square.width,
            square.y + (1.0 - v / 100.0) * square.height,
        );
        marker(renderer, spot, 12.0, 12.0);
        let hue_at = Point::new(bar.x + hue / 359.0 * bar.width, bar.center_y());
        marker(renderer, hue_at, 6.0, BAR_H);
    }
}

impl<'a, M: 'a> From<ColorPicker<'a, M>> for Element<'a, M> {
    fn from(picker: ColorPicker<'a, M>) -> Self {
        Element::new(picker)
    }
}
