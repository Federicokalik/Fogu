//! `SplitTree<T>`: pure binary split-tree model and operations. No I/O, no GUI: the split
//! container widget turns [`SplitTree::layout`]/[`SplitTree::dividers`] into iced primitives
//! and handles divider drag by calling [`SplitTree::set_ratio`]; tab recovery serializes a
//! `SplitTree<PaneSpec>` built from the live `SplitTree<PaneId>` via [`SplitTree::map`].
//! The recursive helpers live in `rec.rs`, the tests in `tests.rs`.

use serde::{Deserialize, Serialize};

use crate::action::Dir;

mod rec;
#[cfg(test)]
mod tests;

use rec::{
    RemoveResult, collect_leaves, distance, dividers_rec, equalize_rec, find_path_rec,
    for_each_leaf_mut, layout_rec, map_node, node_at_path_mut, remove_rec, split_rec,
};

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
    pub(super) const fn orientation(self) -> Orientation {
        match self {
            Self::Left | Self::Right => Orientation::Horizontal,
            Self::Top | Self::Bottom => Orientation::Vertical,
        }
    }

    /// Whether the new leaf becomes the `a` (first/top-left) child.
    pub(super) const fn new_leaf_is_a(self) -> bool {
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
        let mut pending = Some(Node::Leaf(new_leaf));
        self.root = Some(split_rec(root, target, side, &mut pending));
        pending.is_none()
    }

    /// Splits the leaf equal to `target`, inserting the whole of `other` on `side` (a tab
    /// dropped onto a pane). Returns `false` (leaving the tree unchanged) if no leaf equals
    /// `target` or `other` is empty.
    pub fn graft(&mut self, target: &T, side: Side, other: Self) -> bool
    where
        T: PartialEq,
    {
        let (Some(root), Some(other)) = (self.root.take(), other.root) else {
            return false;
        };
        let mut pending = Some(other);
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

    /// Puts every split ratio back into [`MIN_RATIO`]..=[`MAX_RATIO`] (`NaN` → even): for trees
    /// read from a file.
    pub fn clamp_ratios(&mut self) {
        fn walk<T>(node: &mut Node<T>) {
            if let Node::Split { ratio, a, b, .. } = node {
                *ratio = if ratio.is_nan() {
                    0.5
                } else {
                    ratio.clamp(MIN_RATIO, MAX_RATIO)
                };
                walk(a);
                walk(b);
            }
        }
        if let Some(root) = &mut self.root {
            walk(root);
        }
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
