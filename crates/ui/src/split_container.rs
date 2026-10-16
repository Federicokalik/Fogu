//! `SplitContainer`: lays a tab's panes out from its `SplitTree<PaneId>`, draws the dividers and
//! the focused pane's border, and turns divider drags into ratio changes.
//! An optional side panel (the SFTP browser) sits right of the panes behind one more divider
//! with the same hit area, cursor and double-click behaviour.

use fogu_core::{Divider, Orientation, Rect, SplitTree};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse, overlay};
use iced::{Border, Color, Element, Event, Length, Point, Rectangle, Size, Vector};

use crate::message::Message;
use crate::sftp::{PanelMsg, SftpMsg};
use crate::tab::{PaneId, TabId};

/// Space between sibling panes; the divider line sits in its middle.
pub const GAP: f32 = 4.0;
/// Width of a divider's invisible mouse hit area.
const HIT: f32 = 8.0;
/// Room kept around the panes for the focused pane's border.
const BORDER: f32 = 1.0;

/// The split container of one tab. Children come in any order, each tagged with its pane.
pub struct SplitContainer<'a> {
    tab: TabId,
    tree: &'a SplitTree<PaneId>,
    ids: Vec<PaneId>,
    children: Vec<Element<'a, Message>>,
    background: Color,
    divider: Color,
    /// The pane outlined, and the outline colour.
    border: Option<(PaneId, Color)>,
    /// Width share of the side panel (the last child), if there is one.
    side: Option<f32>,
}

impl std::fmt::Debug for SplitContainer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitContainer")
            .field("tab", &self.tab)
            .field("panes", &self.ids)
            .finish_non_exhaustive()
    }
}

/// Widget state: the divider being dragged and the last press (double-click detection).
#[derive(Debug, Default)]
struct State {
    drag: Option<Drag>,
    last_click: Option<mouse::Click>,
}

/// Which kind of divider is dragged.
#[derive(Clone, Debug)]
enum Drag {
    /// A split of the tree.
    Split(Divider),
    /// The side panel's divider (its `area` is the whole inner area).
    Side(Divider),
}

impl Drag {
    fn divider(&self) -> &Divider {
        match self {
            Self::Split(d) | Self::Side(d) => d,
        }
    }
}

impl<'a> SplitContainer<'a> {
    /// Lays out `panes` (one per leaf of `tree`) of tab `tab` on a `background` with `divider`
    /// lines between them.
    pub fn new(
        tab: TabId,
        tree: &'a SplitTree<PaneId>,
        panes: Vec<(PaneId, Element<'a, Message>)>,
        background: Color,
        divider: Color,
    ) -> Self {
        let (ids, children) = panes.into_iter().unzip();
        Self {
            tab,
            tree,
            ids,
            children,
            background,
            divider,
            border: None,
            side: None,
        }
    }

    /// Adds `panel` right of the panes, `ratio` of the width (the rest minus the gap goes to
    /// the panes); dragging its divider publishes the new ratio.
    #[must_use]
    pub fn side(mut self, panel: Element<'a, Message>, ratio: f32) -> Self {
        self.children.push(panel);
        self.side = Some(ratio);
        self
    }

    /// The panes' area, and the side panel's area and divider.
    fn areas(&self, bounds: Rectangle) -> (Rect, Option<(Rect, Divider)>) {
        let all = inner(bounds);
        let Some(ratio) = self.side else {
            return (all, None);
        };
        let avail = (all.w - GAP).max(0.0);
        let side_w = (avail * ratio).round();
        let panes = Rect {
            w: avail - side_w,
            ..all
        };
        let gap = Rect {
            x: all.x + panes.w,
            w: GAP,
            ..all
        };
        let side = Rect {
            x: gap.x + GAP,
            w: side_w,
            ..all
        };
        let divider = Divider {
            path: fogu_core::NodePath::default(),
            rect: gap,
            dir: Orientation::Horizontal,
            area: all,
        };
        (panes, Some((side, divider)))
    }

    /// Outlines `pane` with a 1-px `color` border.
    #[must_use]
    pub fn border(mut self, pane: PaneId, color: Color) -> Self {
        self.border = Some((pane, color));
        self
    }

    /// Mouse handling for the dividers; `true` when the event was theirs.
    fn on_divider_event(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(at) = cursor.position() else {
                    return false;
                };
                let Some(drag) = self.divider_at(bounds, at) else {
                    return false;
                };
                let click = mouse::Click::new(at, mouse::Button::Left, state.last_click);
                state.last_click = Some(click);
                if click.kind() == mouse::click::Kind::Double {
                    state.drag = None;
                    shell.publish(match drag {
                        Drag::Split(_) => Message::Equalize(self.tab),
                        Drag::Side(_) => self.side_message(None),
                    });
                } else {
                    state.drag = Some(drag);
                }
                true
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let Some(drag) = &state.drag else {
                    return false;
                };
                let ratio = drag.divider().ratio_at(position.x, position.y, GAP);
                shell.publish(match drag {
                    Drag::Split(d) => Message::SplitRatio(self.tab, d.path.clone(), ratio),
                    // `ratio` is the panes' share; the panel gets the rest.
                    Drag::Side(_) => self.side_message(Some(1.0 - ratio)),
                });
                true
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take().is_some()
            }
            _ => false,
        }
    }

    fn side_message(&self, ratio: Option<f32>) -> Message {
        Message::Sftp(SftpMsg::Panel(self.tab, PanelMsg::Width(ratio)))
    }

    /// Every divider: the tree's, then the side panel's.
    fn dividers(&self, bounds: Rectangle) -> Vec<Drag> {
        let (panes, side) = self.areas(bounds);
        let splits = self.tree.dividers(panes, GAP).into_iter().map(Drag::Split);
        splits.chain(side.map(|(_, d)| Drag::Side(d))).collect()
    }

    fn divider_at(&self, bounds: Rectangle, at: Point) -> Option<Drag> {
        self.dividers(bounds)
            .into_iter()
            .find(|d| hit_area(d.divider()).contains(at))
    }
}

/// The area the panes share: `bounds` minus room for the border.
fn inner(bounds: Rectangle) -> Rect {
    Rect {
        x: bounds.x + BORDER,
        y: bounds.y + BORDER,
        w: (bounds.width - 2.0 * BORDER).max(0.0),
        h: (bounds.height - 2.0 * BORDER).max(0.0),
    }
}

fn rectangle(r: Rect) -> Rectangle {
    Rectangle {
        x: r.x,
        y: r.y,
        width: r.w,
        height: r.h,
    }
}

/// A divider's gap widened (or narrowed) across its thin axis to `width`, centred.
fn across(d: &Divider, width: f32) -> Rectangle {
    let r = rectangle(d.rect);
    match d.dir {
        Orientation::Horizontal => Rectangle {
            x: r.x + (r.width - width) / 2.0,
            width,
            ..r
        },
        Orientation::Vertical => Rectangle {
            y: r.y + (r.height - width) / 2.0,
            height: width,
            ..r
        },
    }
}

fn hit_area(d: &Divider) -> Rectangle {
    across(d, HIT)
}

fn resize_cursor(dir: Orientation) -> mouse::Interaction {
    match dir {
        Orientation::Horizontal => mouse::Interaction::ResizingHorizontally,
        Orientation::Vertical => mouse::Interaction::ResizingVertically,
    }
}

fn fill(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    let quad = Quad {
        bounds,
        ..Quad::default()
    };
    renderer::Renderer::fill_quad(renderer, quad, color);
}

impl Widget<Message, iced::Theme, iced::Renderer> for SplitContainer<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.max();
        let (pane_area, side_area) = self.areas(Rectangle::with_size(size));
        let rects = self.tree.layout(pane_area, GAP);
        // Children are the panes (tagged by `ids`), then the side panel if any.
        let side_rect = side_area.map(|(r, _)| r);
        let nodes = self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .enumerate()
            .map(|(i, (child, state))| {
                let rect = match self.ids.get(i) {
                    Some(id) => rects
                        .iter()
                        .find(|(pane, _)| pane == id)
                        .map_or(Rect::default(), |&(_, r)| r),
                    None => side_rect.unwrap_or_default(),
                };
                let limits = layout::Limits::new(Size::ZERO, Size::new(rect.w, rect.h));
                child
                    .as_widget_mut()
                    .layout(state, renderer, &limits)
                    .move_to(Point::new(rect.x, rect.y))
            })
            .collect();
        layout::Node::with_children(size, nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, state), layout) in self
                .children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .operate(state, layout, renderer, operation);
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if self.on_divider_event(state, event, layout.bounds(), cursor, shell) {
            shell.capture_event();
            return;
        }
        for ((child, state), layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
                state, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if let Some(drag) = &tree.state.downcast_ref::<State>().drag {
            return resize_cursor(drag.divider().dir);
        }
        if let Some(drag) = cursor
            .position()
            .and_then(|at| self.divider_at(layout.bounds(), at))
        {
            return resize_cursor(drag.divider().dir);
        }
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, state), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .find(|&interaction| interaction != mouse::Interaction::None)
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        fill(renderer, bounds, self.background);
        for ((child, state), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            child
                .as_widget()
                .draw(state, renderer, theme, style, layout, cursor, viewport);
        }
        for drag in self.dividers(bounds) {
            fill(renderer, across(drag.divider(), 1.0), self.divider);
        }
        let Some((pane, color)) = self.border else {
            return;
        };
        let focused = self
            .ids
            .iter()
            .zip(layout.children())
            .find(|(id, _)| **id == pane);
        if let Some((_, layout)) = focused {
            let quad = Quad {
                bounds: layout.bounds().expand(BORDER),
                border: Border {
                    color,
                    width: BORDER,
                    radius: 0.0.into(),
                },
                ..Quad::default()
            };
            renderer::Renderer::fill_quad(renderer, quad, Color::TRANSPARENT);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a> From<SplitContainer<'a>> for Element<'a, Message> {
    fn from(container: SplitContainer<'a>) -> Self {
        Element::new(container)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_area_is_wider_than_the_gap_and_centred() {
        let mut tree = SplitTree::new(PaneId(1));
        assert!(tree.split(&PaneId(1), fogu_core::Side::Right, PaneId(2)));
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(202.0, 102.0));
        let divider = tree.dividers(inner(bounds), GAP).remove(0);
        let hit = hit_area(&divider);
        assert!((hit.width - HIT).abs() < f32::EPSILON);
        let gap_centre = divider.rect.x + divider.rect.w / 2.0;
        assert!((hit.center_x() - gap_centre).abs() < 1e-4);
        assert!(hit.contains(Point::new(gap_centre + 3.5, 50.0)));
        assert!(!hit.contains(Point::new(gap_centre + 4.5, 50.0)));
        let line = across(&divider, 1.0);
        assert!((line.width - 1.0).abs() < f32::EPSILON);
    }
}
