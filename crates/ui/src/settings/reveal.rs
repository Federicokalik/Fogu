//! Scrolls the settings page so a row (a container with a known id) is in view: one widget
//! operation measures the row inside the page's scrollable, a second scrolls there.

use iced::advanced::widget::operation::{Operation, Outcome, Scrollable};
use iced::widget::Id;
use iced::widget::operation::AbsoluteOffset;
use iced::{Rectangle, Task, Vector};

use crate::message::Message;

/// Room left above the revealed row.
const MARGIN: f32 = 12.0;

/// Finds the scrollable `page` and the container `target`; their top edges give the offset.
struct Measure {
    page: Id,
    target: Id,
    page_top: Option<f32>,
    target_top: Option<f32>,
}

impl Operation<f32> for Measure {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<f32>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.target) {
            self.target_top = Some(bounds.y);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.page) {
            self.page_top = Some(content_bounds.y);
        }
    }

    fn finish(&self) -> Outcome<f32> {
        match (self.page_top, self.target_top) {
            (Some(page), Some(target)) => Outcome::Some((target - page - MARGIN).max(0.0)),
            _ => Outcome::None,
        }
    }
}

/// Scrolls scrollable `page` to the container `target`.
pub fn reveal(page: Id, target: Id) -> Task<Message> {
    let scroll_id = page.clone();
    iced::advanced::widget::operate(Measure {
        page,
        target,
        page_top: None,
        target_top: None,
    })
    .then(move |y| {
        iced::widget::operation::scroll_to(
            scroll_id.clone(),
            AbsoluteOffset {
                x: None,
                y: Some(y),
            },
        )
    })
}
