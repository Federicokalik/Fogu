//! `SplitContainer` geometry tests.

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
