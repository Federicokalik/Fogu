//! Recursive helpers behind [`SplitTree`]'s operations: every function here walks or rebuilds a
//! [`Node`] tree by value or by `&mut`, with no `unsafe` and no panics. Kept out of `mod.rs` so
//! the public model stays readable (and both files stay under the 500-line rule).

use super::{Divider, Node, NodePath, Orientation, Rect, Side, SplitTree};

pub(super) fn collect_leaves<'a, T>(node: &'a Node<T>, out: &mut Vec<&'a T>) {
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

pub(super) fn layout_rec<T: Clone>(node: &Node<T>, area: Rect, gap: f32, out: &mut Vec<(T, Rect)>) {
    match node {
        Node::Leaf(v) => out.push((v.clone(), area)),
        Node::Split { dir, ratio, a, b } => {
            let (rect_a, rect_b) = split_rect(area, *dir, *ratio, gap);
            layout_rec(a, rect_a, gap, out);
            layout_rec(b, rect_b, gap, out);
        }
    }
}

pub(super) fn dividers_rec<T>(
    node: &Node<T>,
    area: Rect,
    gap: f32,
    path: NodePath,
    out: &mut Vec<Divider>,
) {
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

pub(super) fn node_at_path_mut<'a, T>(
    root: &'a mut Node<T>,
    path: &[bool],
) -> Option<&'a mut Node<T>> {
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

pub(super) fn equalize_rec<T>(node: &mut Node<T>) {
    if let Node::Split { ratio, a, b, .. } = node {
        *ratio = 0.5;
        equalize_rec(a);
        equalize_rec(b);
    }
}

/// Rebuilds `node` with `new_leaf` spliced in next to the leaf equal to `target`, taken from
/// `pending` the first (and only) time it's found; later matches (shouldn't happen with unique
/// leaves) are left alone.
pub(super) fn split_rec<T: PartialEq>(
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
pub(super) enum RemoveResult<T> {
    NotFound(Node<T>),
    Removed {
        remaining: Option<Node<T>>,
        removed: T,
    },
}

pub(super) fn remove_rec<T: PartialEq>(node: Node<T>, target: &T) -> RemoveResult<T> {
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

pub(super) fn find_path_rec<T: PartialEq>(node: &Node<T>, target: &T, path: &mut NodePath) -> bool {
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

pub(super) fn for_each_leaf_mut<T>(tree: &mut SplitTree<T>, f: &mut dyn FnMut(&mut T)) {
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

pub(super) fn map_node<T, U>(node: Node<T>, f: &mut impl FnMut(T) -> U) -> Node<U> {
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

pub(super) fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}
