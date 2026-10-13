//! `SplitTree<T>`: pure binary split-tree model and operations. No I/O, no GUI: the split
//! container widget turns [`SplitTree::layout`]/[`SplitTree::dividers`] into iced primitives
//! and handles divider drag by calling [`SplitTree::set_ratio`]; tab recovery serializes a
//! `SplitTree<PaneSpec>` built from the live `SplitTree<PaneId>` via [`SplitTree::map`].

use serde::{Deserialize, Serialize};

use crate::action::Dir;

/// Minimum/maximum share of a split's available space either side may hold.
const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 0.9;

/// Orientation of a [`Node::Split`]. This is **not** the pane-navigation [`Dir`]: a split lays
/// its two children side by side (`Horizontal`, a vertical divider between them) or stacked
/// (`Vertical`, a horizontal divider).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    /// Children `a` (left) and `b` (right) side by side.
    Horizontal,
    /// Children `a` (top) and `b` (bottom) stacked.
    Vertical,
}

/// Which side of an existing leaf a new leaf goes on when splitting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// New leaf to the left of the target; target moves right.
    Left,
    /// New leaf to the right of the target; target stays left.
    Right,
    /// New leaf above the target; target moves down.
    Top,
    /// New leaf below the target; target stays on top.
    Bottom,
}

impl Side {
    /// The orientation of the split this side implies.
    const fn orientation(self) -> Orientation {
        match self {
            Self::Left | Self::Right => Orientation::Horizontal,
            Self::Top | Self::Bottom => Orientation::Vertical,
        }
    }

    /// Whether the new leaf becomes the `a` (first/top-left) child.
    const fn new_leaf_is_a(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
}

/// One node of a binary split tree: either a pane (`Leaf`) or a split into two sub-trees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Node<T> {
    /// A single pane.
    Leaf(T),
    /// A split into two sub-trees. `ratio` is `a`'s share of the available space, clamped to
    /// [`MIN_RATIO`]..=[`MAX_RATIO`].
    Split {
        /// Split orientation.
        dir: Orientation,
        /// `a`'s share of the available space.
        ratio: f32,
        /// First (left/top) child.
        a: Box<Node<T>>,
        /// Second (right/bottom) child.
        b: Box<Node<T>>,
    },
}

/// A binary split tree of leaves of type `T`. Empty only as the result of
/// [`SplitTree::remove`] removing the last leaf; [`SplitTree::new`] always starts with one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitTree<T> {
    root: Option<Node<T>>,
}

/// An axis-aligned rectangle in whatever unit the caller's `area` used (pixels, typically).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// Centre point of the rectangle.
    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Path from the root to a node: `false` steps into `a`, `true` into `b`.
pub type NodePath = Vec<bool>;

/// A divider between the two children of one [`Node::Split`], found at `path`.
///
/// `rect` is exactly the gap between the two children; widening it into a wider mouse hit-area
/// (6-8 px) is the caller's job, not this model's.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    /// Path to the [`Node::Split`] this divider belongs to.
    pub path: NodePath,
    /// The divider's rectangle (the gap itself).
    pub rect: Rect,
    /// The split's orientation.
    pub dir: Orientation,
    /// The whole rectangle of the split (both children plus the gap).
    pub area: Rect,
}

impl Divider {
    /// The ratio that puts this divider's centre at pointer position `(x, y)` (same units and
    /// `gap` as [`SplitTree::dividers`]); unclamped, [`SplitTree::set_ratio`] clamps it.
    #[must_use]
    pub fn ratio_at(&self, x: f32, y: f32, gap: f32) -> f32 {
        let (pos, start, len) = match self.dir {
            Orientation::Horizontal => (x, self.area.x, self.area.w),
            Orientation::Vertical => (y, self.area.y, self.area.h),
        };
        let avail = len - gap;
        if avail <= 0.0 {
            return 0.5;
        }
        (pos - start - gap / 2.0) / avail
    }
}

impl<T> SplitTree<T> {
    /// A tree with a single leaf.
    pub fn new(leaf: T) -> Self {
        Self {
            root: Some(Node::Leaf(leaf)),
        }
    }

    /// Whether the tree has no leaves left (after [`SplitTree::remove`] took the last one).
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// All leaves, in left-to-right / top-to-bottom tree order.
    pub fn leaves(&self) -> Vec<&T> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            collect_leaves(root, &mut out);
        }
        out
    }

    /// Computed rectangle for every leaf, given the overall `area` and the gap left between
    /// siblings for the divider.
    pub fn layout(&self, area: Rect, gap: f32) -> Vec<(T, Rect)>
    where
        T: Clone,
    {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            layout_rec(root, area, gap, &mut out);
        }
        out
    }

    /// One [`Divider`] per [`Node::Split`] in the tree, given the same `area`/`gap` as
    /// [`SplitTree::layout`].
    pub fn dividers(&self, area: Rect, gap: f32) -> Vec<Divider> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            dividers_rec(root, area, gap, Vec::new(), &mut out);
        }
        out
    }

    /// Sets the ratio of the split at `path`, clamped to [`MIN_RATIO`]..=[`MAX_RATIO`]. Returns
    /// `false` if `path` doesn't address a [`Node::Split`].
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) -> bool {
        let clamped = ratio.clamp(MIN_RATIO, MAX_RATIO);
        let Some(root) = self.root.as_mut() else {
            return false;
        };
        let Some(node) = node_at_path_mut(root, path) else {
            return false;
        };
        let Node::Split { ratio: r, .. } = node else {
            return false;
        };
        *r = clamped;
        true
    }

    /// Resets every split's ratio to 0.5.
    pub fn equalize(&mut self) {
        if let Some(root) = self.root.as_mut() {
            equalize_rec(root);
        }
    }

    /// Splits the leaf equal to `target`, inserting `new_leaf` on `side`. Returns `false`
    /// (leaving the tree unchanged) if no leaf equals `target`.
    pub fn split(&mut self, target: &T, side: Side, new_leaf: T) -> bool
    where
        T: PartialEq,
    {
        let Some(root) = self.root.take() else {
            return false;
        };
        let mut pending = Some(new_leaf);
        self.root = Some(split_rec(root, target, side, &mut pending));
        pending.is_none()
    }

    /// Removes the leaf equal to `target`, collapsing its parent split onto the sibling.
    /// Removing the last leaf empties the tree (see [`SplitTree::is_empty`]). Returns the
    /// removed value, or `None` if no leaf equals `target`.
    pub fn remove(&mut self, target: &T) -> Option<T>
    where
        T: PartialEq,
    {
        let root = self.root.take()?;
        match remove_rec(root, target) {
            RemoveResult::Removed { remaining, removed } => {
                self.root = remaining;
                Some(removed)
            }
            RemoveResult::NotFound(node) => {
                self.root = Some(node);
                None
            }
        }
    }

    /// Moves the ratio of the nearest enclosing split that has a sibling of `target` in
    /// direction `dir` by `delta` (clamped), growing `target` toward `dir`. Returns `false` if
    /// `target` isn't found or has no such split (e.g. it already fills the tree in that axis).
    pub fn resize_step(&mut self, target: &T, dir: Dir, delta: f32) -> bool
    where
        T: PartialEq,
    {
        let Some(path) = self.find_path(target) else {
            return false;
        };
        let Some(root) = self.root.as_mut() else {
            return false;
        };
        for i in (0..path.len()).rev() {
            let Some(ancestor_path) = path.get(..i) else {
                continue;
            };
            let Some(&branch_is_b) = path.get(i) else {
                continue;
            };
            let Some(node) = node_at_path_mut(root, ancestor_path) else {
                continue;
            };
            let Node::Split {
                dir: orientation,
                ratio,
                ..
            } = node
            else {
                continue;
            };
            let adjust = match (*orientation, branch_is_b, dir) {
                (Orientation::Horizontal, false, Dir::Right)
                | (Orientation::Vertical, false, Dir::Down) => Some(delta),
                (Orientation::Horizontal, true, Dir::Left)
                | (Orientation::Vertical, true, Dir::Up) => Some(-delta),
                _ => None,
            };
            if let Some(d) = adjust {
                *ratio = (*ratio + d).clamp(MIN_RATIO, MAX_RATIO);
                return true;
            }
        }
        false
    }

    /// The leaf whose rectangle is adjacent to `target`'s in direction `dir` and nearest to its
    /// centre, geometrically. `None` if `target` isn't found or has no neighbour that way.
    pub fn neighbor(&self, target: &T, dir: Dir) -> Option<&T>
    where
        T: PartialEq + Clone,
    {
        let rects = self.layout(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            0.0,
        );
        let target_center = rects.iter().find(|(v, _)| v == target)?.1.center();

        let mut best: Option<(&T, f32)> = None;
        for (v, rect) in &rects {
            if v == target {
                continue;
            }
            let center = rect.center();
            let in_direction = match dir {
                Dir::Left => center.0 < target_center.0,
                Dir::Right => center.0 > target_center.0,
                Dir::Up => center.1 < target_center.1,
                Dir::Down => center.1 > target_center.1,
            };
            if !in_direction {
                continue;
            }
            let dist = distance(target_center, center);
            let better = match best {
                None => true,
                Some((_, best_dist)) => dist < best_dist,
            };
            if better {
                best = Some((v, dist));
            }
        }
        let best_value = best.map(|(v, _)| v.clone())?;
        self.leaves().into_iter().find(|v| **v == best_value)
    }

    /// Swaps the positions (not the surrounding structure) of the leaves equal to `a` and `b`.
    /// Returns `false` if `a == b` or either isn't found.
    pub fn swap(&mut self, a: &T, b: &T) -> bool
    where
        T: PartialEq + Clone,
    {
        if a == b {
            return false;
        }
        let leaves = self.leaves();
        let has_a = leaves.iter().any(|v| **v == *a);
        let has_b = leaves.iter().any(|v| **v == *b);
        drop(leaves);
        if !has_a || !has_b {
            return false;
        }
        let a_val = a.clone();
        let b_val = b.clone();
        for_each_leaf_mut(self, &mut |v| {
            if *v == a_val {
                *v = b_val.clone();
            } else if *v == b_val {
                *v = a_val.clone();
            }
        });
        true
    }

    /// Removes `target` and splits it back in on `side` of `onto`. Returns `false` (leaving the
    /// tree unchanged) if `target == onto`, `target` isn't found, or `onto` isn't found.
    pub fn move_leaf(&mut self, target: &T, onto: &T, side: Side) -> bool
    where
        T: PartialEq,
    {
        if target == onto || !self.leaves().iter().any(|v| **v == *onto) {
            return false;
        }
        let Some(removed) = self.remove(target) else {
            return false;
        };
        self.split(onto, side, removed)
    }

    /// Transforms every leaf, keeping the tree's shape and ratios. Used by tab recovery to
    /// turn a live `SplitTree<PaneId>` into a serializable `SplitTree<PaneSpec>` and back.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> SplitTree<U> {
        let mut f = f;
        SplitTree {
            root: self.root.map(|r| map_node(r, &mut f)),
        }
    }

    /// Path to the leaf equal to `target`, or `None` if there isn't one.
    fn find_path(&self, target: &T) -> Option<NodePath>
    where
        T: PartialEq,
    {
        let root = self.root.as_ref()?;
        let mut path = Vec::new();
        if find_path_rec(root, target, &mut path) {
            Some(path)
        } else {
            None
        }
    }
}

fn collect_leaves<'a, T>(node: &'a Node<T>, out: &mut Vec<&'a T>) {
    match node {
        Node::Leaf(v) => out.push(v),
        Node::Split { a, b, .. } => {
            collect_leaves(a, out);
            collect_leaves(b, out);
        }
    }
}

/// Splits `area` into the two child rectangles of a split with this `dir`/`ratio`/`gap`.
fn split_rect(area: Rect, dir: Orientation, ratio: f32, gap: f32) -> (Rect, Rect) {
    match dir {
        Orientation::Horizontal => {
            let avail = (area.w - gap).max(0.0);
            let a_w = avail * ratio;
            let b_w = avail - a_w;
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    w: a_w,
                    h: area.h,
                },
                Rect {
                    x: area.x + a_w + gap,
                    y: area.y,
                    w: b_w,
                    h: area.h,
                },
            )
        }
        Orientation::Vertical => {
            let avail = (area.h - gap).max(0.0);
            let a_h = avail * ratio;
            let b_h = avail - a_h;
            (
                Rect {
                    x: area.x,
                    y: area.y,
                    w: area.w,
                    h: a_h,
                },
                Rect {
                    x: area.x,
                    y: area.y + a_h + gap,
                    w: area.w,
                    h: b_h,
                },
            )
        }
    }
}

fn layout_rec<T: Clone>(node: &Node<T>, area: Rect, gap: f32, out: &mut Vec<(T, Rect)>) {
    match node {
        Node::Leaf(v) => out.push((v.clone(), area)),
        Node::Split { dir, ratio, a, b } => {
            let (rect_a, rect_b) = split_rect(area, *dir, *ratio, gap);
            layout_rec(a, rect_a, gap, out);
            layout_rec(b, rect_b, gap, out);
        }
    }
}

fn dividers_rec<T>(node: &Node<T>, area: Rect, gap: f32, path: NodePath, out: &mut Vec<Divider>) {
    if let Node::Split { dir, ratio, a, b } = node {
        let (rect_a, rect_b) = split_rect(area, *dir, *ratio, gap);
        let divider_rect = match dir {
            Orientation::Horizontal => Rect {
                x: rect_a.x + rect_a.w,
                y: area.y,
                w: gap,
                h: area.h,
            },
            Orientation::Vertical => Rect {
                x: area.x,
                y: rect_a.y + rect_a.h,
                w: area.w,
                h: gap,
            },
        };
        out.push(Divider {
            path: path.clone(),
            rect: divider_rect,
            dir: *dir,
            area,
        });
        let mut path_a = path.clone();
        path_a.push(false);
        dividers_rec(a, rect_a, gap, path_a, out);
        let mut path_b = path;
        path_b.push(true);
        dividers_rec(b, rect_b, gap, path_b, out);
    }
}

fn node_at_path_mut<'a, T>(root: &'a mut Node<T>, path: &[bool]) -> Option<&'a mut Node<T>> {
    let mut node = root;
    for &to_b in path {
        node = match node {
            Node::Split { a, b, .. } => {
                if to_b {
                    b.as_mut()
                } else {
                    a.as_mut()
                }
            }
            Node::Leaf(_) => return None,
        };
    }
    Some(node)
}

fn equalize_rec<T>(node: &mut Node<T>) {
    if let Node::Split { ratio, a, b, .. } = node {
        *ratio = 0.5;
        equalize_rec(a);
        equalize_rec(b);
    }
}

/// Rebuilds `node` with `new_leaf` spliced in next to the leaf equal to `target`, taken from
/// `pending` the first (and only) time it's found; later matches (shouldn't happen with unique
/// leaves) are left alone.
fn split_rec<T: PartialEq>(
    node: Node<T>,
    target: &T,
    side: Side,
    pending: &mut Option<T>,
) -> Node<T> {
    match node {
        Node::Leaf(v) => {
            if v == *target {
                if let Some(new_leaf) = pending.take() {
                    let (a, b) = if side.new_leaf_is_a() {
                        (Node::Leaf(new_leaf), Node::Leaf(v))
                    } else {
                        (Node::Leaf(v), Node::Leaf(new_leaf))
                    };
                    return Node::Split {
                        dir: side.orientation(),
                        ratio: 0.5,
                        a: Box::new(a),
                        b: Box::new(b),
                    };
                }
                Node::Leaf(v)
            } else {
                Node::Leaf(v)
            }
        }
        Node::Split { dir, ratio, a, b } => Node::Split {
            dir,
            ratio,
            a: Box::new(split_rec(*a, target, side, pending)),
            b: Box::new(split_rec(*b, target, side, pending)),
        },
    }
}

/// Outcome of removing a leaf from a subtree: either it wasn't there (`NotFound`, subtree
/// unchanged) or it was (`Removed`), with `remaining` being `None` when the whole subtree was
/// just that one leaf.
enum RemoveResult<T> {
    NotFound(Node<T>),
    Removed {
        remaining: Option<Node<T>>,
        removed: T,
    },
}

fn remove_rec<T: PartialEq>(node: Node<T>, target: &T) -> RemoveResult<T> {
    match node {
        Node::Leaf(v) => {
            if v == *target {
                RemoveResult::Removed {
                    remaining: None,
                    removed: v,
                }
            } else {
                RemoveResult::NotFound(Node::Leaf(v))
            }
        }
        Node::Split { dir, ratio, a, b } => match remove_rec(*a, target) {
            RemoveResult::Removed { remaining, removed } => {
                let node = match remaining {
                    Some(a_node) => Node::Split {
                        dir,
                        ratio,
                        a: Box::new(a_node),
                        b,
                    },
                    None => *b,
                };
                RemoveResult::Removed {
                    remaining: Some(node),
                    removed,
                }
            }
            RemoveResult::NotFound(a_node) => match remove_rec(*b, target) {
                RemoveResult::Removed { remaining, removed } => {
                    let node = match remaining {
                        Some(b_node) => Node::Split {
                            dir,
                            ratio,
                            a: Box::new(a_node),
                            b: Box::new(b_node),
                        },
                        None => a_node,
                    };
                    RemoveResult::Removed {
                        remaining: Some(node),
                        removed,
                    }
                }
                RemoveResult::NotFound(b_node) => RemoveResult::NotFound(Node::Split {
                    dir,
                    ratio,
                    a: Box::new(a_node),
                    b: Box::new(b_node),
                }),
            },
        },
    }
}

fn find_path_rec<T: PartialEq>(node: &Node<T>, target: &T, path: &mut NodePath) -> bool {
    match node {
        Node::Leaf(v) => v == target,
        Node::Split { a, b, .. } => {
            path.push(false);
            if find_path_rec(a, target, path) {
                return true;
            }
            path.pop();
            path.push(true);
            if find_path_rec(b, target, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}

fn for_each_leaf_mut<T>(tree: &mut SplitTree<T>, f: &mut dyn FnMut(&mut T)) {
    if let Some(root) = tree.root.as_mut() {
        for_each_leaf_mut_rec(root, f);
    }
}

fn for_each_leaf_mut_rec<T>(node: &mut Node<T>, f: &mut dyn FnMut(&mut T)) {
    match node {
        Node::Leaf(v) => f(v),
        Node::Split { a, b, .. } => {
            for_each_leaf_mut_rec(a, f);
            for_each_leaf_mut_rec(b, f);
        }
    }
}

fn map_node<T, U>(node: Node<T>, f: &mut impl FnMut(T) -> U) -> Node<U> {
    match node {
        Node::Leaf(v) => Node::Leaf(f(v)),
        Node::Split { dir, ratio, a, b } => Node::Split {
            dir,
            ratio,
            a: Box::new(map_node(*a, f)),
            b: Box::new(map_node(*b, f)),
        },
    }
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

    use super::*;

    /// Builds a tree from leaf `0`, applying `(new_leaf_id, target, side)` splits in order.
    fn build(ops: &[(u32, u32, Side)]) -> SplitTree<u32> {
        let mut t = SplitTree::new(0);
        for &(new_leaf, target, side) in ops {
            assert!(t.split(&target, side, new_leaf), "split target not found");
        }
        t
    }

    fn area() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        }
    }

    #[test]
    fn new_single_leaf_fills_the_area() {
        let t = SplitTree::new("only");
        assert_eq!(t.leaves(), vec![&"only"]);
        assert_eq!(t.layout(area(), 4.0), vec![("only", area())]);
        assert_eq!(t.dividers(area(), 4.0).len(), 0);
    }

    #[test]
    fn split_right_adds_leaf_in_order() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        assert_eq!(t.leaves(), vec![&0, &1]);
    }

    #[test]
    fn split_left_puts_new_leaf_first() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Left, 1));
        assert_eq!(t.leaves(), vec![&1, &0]);
    }

    #[test]
    fn split_missing_target_returns_false_and_is_a_no_op() {
        let mut t = SplitTree::new(0u32);
        let before = t.clone();
        assert!(!t.split(&99, Side::Right, 1));
        assert_eq!(t, before);
    }

    #[test]
    fn remove_last_leaf_empties_the_tree() {
        let mut t = SplitTree::new(0u32);
        assert_eq!(t.remove(&0), Some(0));
        assert!(t.is_empty());
        assert_eq!(t.remove(&0), None);
    }

    #[test]
    fn remove_collapses_parent_onto_sibling() {
        let mut t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom)]);
        assert_eq!(t.leaves(), vec![&0, &1, &2]);
        assert_eq!(t.remove(&1), Some(1));
        assert_eq!(t.leaves(), vec![&0, &2]);
        assert_eq!(t, build(&[(2, 0, Side::Right)]));
    }

    #[test]
    fn split_then_remove_round_trips_for_several_shapes() {
        let shapes: &[&[(u32, u32, Side)]] = &[
            &[(1, 0, Side::Right)],
            &[(1, 0, Side::Right), (2, 0, Side::Bottom)],
            &[(1, 0, Side::Right), (2, 1, Side::Bottom), (3, 0, Side::Top)],
        ];
        for ops in shapes {
            let original = build(ops);
            let (new_leaf, target, side) = *ops.last().unwrap();
            let mut rebuilt = original.clone();
            assert_eq!(rebuilt.remove(&new_leaf), Some(new_leaf));
            assert!(rebuilt.split(&target, side, new_leaf));
            assert_eq!(rebuilt, original);
        }
    }

    fn rects_overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn layout_tiles_the_area_without_overlap() {
        let t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom), (3, 0, Side::Top)]);
        let a = area();
        let rects = t.layout(a, 4.0);
        assert_eq!(rects.len(), 4);

        for (_, r) in &rects {
            assert!(r.x >= a.x - f32::EPSILON);
            assert!(r.y >= a.y - f32::EPSILON);
            assert!(r.x + r.w <= a.x + a.w + f32::EPSILON);
            assert!(r.y + r.h <= a.y + a.h + f32::EPSILON);
        }
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                assert!(!rects_overlap(rects[i].1, rects[j].1));
            }
        }
    }

    #[test]
    fn single_split_areas_sum_to_area_minus_gap() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        let a = area();
        let gap = 4.0;
        let rects = t.layout(a, gap);
        let covered: f32 = rects.iter().map(|(_, r)| r.w * r.h).sum();
        let expected = (a.w - gap) * a.h;
        assert!((covered - expected).abs() < 0.01);
    }

    #[test]
    fn dividers_count_matches_split_count() {
        let t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom), (3, 0, Side::Top)]);
        assert_eq!(t.dividers(area(), 4.0).len(), 3);
        assert_eq!(t.leaves().len(), 4);
    }

    #[test]
    fn set_ratio_clamps_to_bounds() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        let path = t.dividers(area(), 4.0)[0].path.clone();

        assert!(t.set_ratio(&path, 5.0));
        let wide_a = t.layout(area(), 4.0)[0].1.w;

        assert!(t.set_ratio(&path, -5.0));
        let narrow_a = t.layout(area(), 4.0)[0].1.w;

        assert!(narrow_a < wide_a);
        assert!(!t.set_ratio(&[true], 0.5)); // no split at that path (leaf, not a Split)
    }

    #[test]
    fn divider_ratio_at_inverts_the_layout() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Bottom, 1));
        assert!(t.set_ratio(&[], 0.3));
        let d = t.dividers(area(), 4.0)[0].clone();
        assert_eq!(d.area, area());
        let centre_y = d.rect.y + d.rect.h / 2.0;
        assert!((d.ratio_at(0.0, centre_y, 4.0) - 0.3).abs() < 1e-4);
        assert!(
            d.ratio_at(0.0, -100.0, 4.0) < 0.0,
            "unclamped; set_ratio clamps"
        );
    }

    #[test]
    fn equalize_resets_every_ratio() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        let path = t.dividers(area(), 0.0)[0].path.clone();
        assert!(t.set_ratio(&path, 0.8));
        t.equalize();
        let rects = t.layout(area(), 0.0);
        assert!((rects[0].1.w - rects[1].1.w).abs() < 0.01);
    }

    #[test]
    fn resize_step_adjusts_the_nearest_enclosing_split() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        let before = t.layout(area(), 0.0)[0].1.w;
        assert!(t.resize_step(&0, Dir::Right, 0.1));
        let after = t.layout(area(), 0.0)[0].1.w;
        assert!(after > before);
    }

    #[test]
    fn resize_step_without_a_matching_ancestor_returns_false() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        // `0` is the `a` (left) child; there's nothing further left to grow into.
        assert!(!t.resize_step(&0, Dir::Left, 0.1));
        assert!(!t.resize_step(&99, Dir::Right, 0.1));
    }

    #[test]
    fn neighbor_is_symmetric_for_two_leaves() {
        let mut t = SplitTree::new(0u32);
        assert!(t.split(&0, Side::Right, 1));
        assert_eq!(t.neighbor(&0, Dir::Right), Some(&1));
        assert_eq!(t.neighbor(&1, Dir::Left), Some(&0));
        assert_eq!(t.neighbor(&0, Dir::Left), None);
        assert_eq!(t.neighbor(&1, Dir::Right), None);
    }

    #[test]
    fn neighbor_picks_the_nearest_candidate() {
        // 0 | 1 on top of 2 (right column split top/bottom); skew the inner split so 1's centre
        // is unambiguously closer to 0's than 2's is.
        let mut t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom)]);
        let inner_path = t.dividers(area(), 0.0)[1].path.clone();
        assert!(t.set_ratio(&inner_path, 0.8));
        assert_eq!(t.neighbor(&0, Dir::Right), Some(&1));
    }

    #[test]
    fn swap_exchanges_leaf_values_in_place() {
        let mut t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom)]);
        let shape_before = t.clone().map(|_| ());
        assert!(t.swap(&0, &2));
        assert_eq!(t.leaves(), vec![&2, &1, &0]);
        assert_eq!(t.clone().map(|_| ()), shape_before);
        assert!(!t.swap(&0, &0));
        assert!(!t.swap(&0, &999));
    }

    #[test]
    fn move_leaf_relocates_without_losing_other_leaves() {
        let mut t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom)]);
        assert!(t.move_leaf(&2, &0, Side::Top));
        assert_eq!(t.leaves(), vec![&2, &0, &1]);
        assert!(!t.move_leaf(&0, &0, Side::Top));
        assert!(!t.move_leaf(&99, &0, Side::Top));
    }

    #[test]
    fn map_transforms_leaves_and_preserves_shape() {
        let t = build(&[(1, 0, Side::Right), (2, 1, Side::Bottom)]);
        let mapped = t.clone().map(|v| v.to_string());
        assert_eq!(
            mapped.leaves(),
            vec![&"0".to_string(), &"1".to_string(), &"2".to_string()]
        );
        // Shape (splits/ratios) is unchanged: mapping back by parsing restores the original.
        let round_tripped = mapped.map(|s| s.parse::<u32>().unwrap());
        assert_eq!(round_tripped, t);
    }
}
