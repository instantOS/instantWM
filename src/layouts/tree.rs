//! Backend-independent manual tiling tree.
//!
//! A tree belongs to one monitor/tag-mask pair.  Splits are weighted n-ary
//! runs.  Construction is deliberately private and canonicalises adjacent
//! splits on the same axis, so the rest of the window manager cannot create a
//! one-child split, a non-positive weight, or redundant same-axis nesting.

use std::collections::{HashMap, HashSet};

use crate::config::config_toml::NewWindowPlacement;
use crate::types::{Point, Rect, Size, WindowId};

const EPSILON: f64 = 1.0e-9;
const IDEAL_TILED_ASPECT_RATIO: f64 = 4.0 / 3.0;
const MIN_HEALTHY_ASPECT_RATIO: f64 = 0.5;
const MAX_HEALTHY_ASPECT_RATIO: f64 = 2.5;
const MIN_HEALTHY_WORK_FRACTION: i32 = 4;
const AUTO_RESIZE_NEW_ROOT_WEIGHT: f64 = 0.4;

mod constraints;
mod geometry;
mod insertion;
mod placement;
mod placement_ops;
mod pointer_cache;
mod presets;
mod resize_commands;
mod resize_ops;
mod types;
use constraints::*;
use placement_ops::*;
#[cfg(test)]
use presets::equal_run;
use presets::{build_grid, build_master_stack};
use resize_ops::*;
pub use types::{Axis, CommandConfig, PlacementTarget, Preset, Side};
use types::{DEFAULT_MINIMUM_WEIGHT, DEFAULT_RESIZE_STEP};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SplitId(u64);

#[derive(Debug, Clone)]
struct WeightedNode {
    node: Node,
    weight: f64,
}

#[derive(Debug, Clone)]
struct Split {
    id: SplitId,
    axis: Axis,
    // Invariant: at least two children, all finite positive weights summing to 1.
    children: Vec<WeightedNode>,
}

#[derive(Debug, Clone)]
enum Node {
    Window(WindowId),
    Split(Split),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum NodeKey {
    Window(WindowId),
    Split(SplitId),
}

impl Node {
    fn key(&self) -> NodeKey {
        match self {
            Self::Window(window) => NodeKey::Window(*window),
            Self::Split(split) => NodeKey::Split(split.id),
        }
    }

    fn contains_key(&self, key: NodeKey) -> bool {
        self.key() == key
            || matches!(self, Self::Split(split) if split.children.iter().any(|child| child.node.contains_key(key)))
    }

    fn replace_key(self, key: NodeKey, replacement: Node) -> Self {
        if self.key() == key {
            return replacement;
        }
        match self {
            Self::Window(window) => Self::Window(window),
            Self::Split(split) => {
                let id = split.id;
                let axis = split.axis;
                let children = split
                    .children
                    .into_iter()
                    .map(|child| WeightedNode {
                        node: child.node.replace_key(key, replacement.clone()),
                        weight: child.weight,
                    })
                    .collect();
                make_split(id, axis, children).expect("replacing a descendant cannot empty a split")
            }
        }
    }
}

impl Node {
    fn contains(&self, window: WindowId) -> bool {
        match self {
            Self::Window(candidate) => *candidate == window,
            Self::Split(split) => split
                .children
                .iter()
                .any(|child| child.node.contains(window)),
        }
    }

    fn leaf_count(&self) -> usize {
        match self {
            Self::Window(_) => 1,
            Self::Split(split) => split
                .children
                .iter()
                .map(|child| child.node.leaf_count())
                .sum(),
        }
    }

    fn leaves(&self, output: &mut Vec<WindowId>) {
        match self {
            Self::Window(window) => output.push(*window),
            Self::Split(split) => {
                for child in &split.children {
                    child.node.leaves(output);
                }
            }
        }
    }

    fn remove(self, window: WindowId) -> Option<Self> {
        match self {
            Self::Window(candidate) => (candidate != window).then_some(Self::Window(candidate)),
            Self::Split(split) => {
                let children = split
                    .children
                    .into_iter()
                    .filter_map(|child| {
                        child.node.remove(window).map(|node| WeightedNode {
                            node,
                            weight: child.weight,
                        })
                    })
                    .collect();
                make_split(split.id, split.axis, children)
            }
        }
    }

    fn replace_window(self, target: WindowId, replacement: Node) -> Self {
        match self {
            Self::Window(window) if window == target => replacement,
            Self::Window(window) => Self::Window(window),
            Self::Split(split) => {
                let children = split
                    .children
                    .into_iter()
                    .map(|child| WeightedNode {
                        node: child.node.replace_window(target, replacement.clone()),
                        weight: child.weight,
                    })
                    .collect();
                make_split(split.id, split.axis, children)
                    .expect("replacing a leaf cannot empty a split")
            }
        }
    }

    fn bounds(&self, rect: FRect, output: &mut HashMap<WindowId, FRect>) {
        match self {
            Self::Window(window) => {
                output.insert(*window, rect);
            }
            Self::Split(split) => {
                let mut offset = 0.0;
                for (index, child) in split.children.iter().enumerate() {
                    // End the final child at the parent edge to contain accumulated
                    // floating point error.
                    let extent = if index + 1 == split.children.len() {
                        1.0 - offset
                    } else {
                        child.weight
                    };
                    let child_rect = match split.axis {
                        Axis::Vertical => FRect {
                            x: rect.x + rect.w * offset,
                            y: rect.y,
                            w: rect.w * extent,
                            h: rect.h,
                        },
                        Axis::Horizontal => FRect {
                            x: rect.x,
                            y: rect.y + rect.h * offset,
                            w: rect.w,
                            h: rect.h * extent,
                        },
                    };
                    child.node.bounds(child_rect, output);
                    offset += extent;
                }
            }
        }
    }

    fn all_bounds(&self, rect: FRect, output: &mut HashMap<NodeKey, FRect>) {
        output.insert(self.key(), rect);
        if let Self::Split(split) = self {
            let mut offset = 0.0;
            for (index, child) in split.children.iter().enumerate() {
                let extent = if index + 1 == split.children.len() {
                    1.0 - offset
                } else {
                    child.weight
                };
                let child_rect = match split.axis {
                    Axis::Vertical => FRect {
                        x: rect.x + rect.w * offset,
                        y: rect.y,
                        w: rect.w * extent,
                        h: rect.h,
                    },
                    Axis::Horizontal => FRect {
                        x: rect.x,
                        y: rect.y + rect.h * offset,
                        w: rect.w,
                        h: rect.h * extent,
                    },
                };
                child.node.all_bounds(child_rect, output);
                offset += extent;
            }
        }
    }
}

#[derive(Debug, Clone)]
enum PlacementScope {
    Node(NodeKey),
    ChildRange {
        parent: SplitId,
        children: Vec<NodeKey>,
    },
    AlignedNode {
        key: NodeKey,
        seam: f64,
        before: Vec<WindowId>,
    },
    AlignedChildRange {
        parent: SplitId,
        children: Vec<NodeKey>,
        seam: f64,
        before: Vec<WindowId>,
    },
}

#[derive(Debug, Clone)]
struct EdgeCandidate {
    scope: PlacementScope,
    scope_depth: usize,
}

#[derive(Debug, Clone)]
struct ResolvedPlacementTarget {
    target: PlacementTarget,
    candidate: LayoutTree,
}

/// One viable, normalized pointer outcome. Keeping its original semantic
/// target beside the constrained preview slot makes hover and release use the
/// same structural candidate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PointerPlacementResolution {
    pub(crate) target: PlacementTarget,
    pub(crate) slot: Rect,
}

#[derive(Debug, Clone, Copy)]
struct AutomaticInsertion {
    score: f64,
    target: WindowId,
    axis: Axis,
    newcomer_slot: Rect,
    target_slot: Rect,
    fits_constraints: bool,
}

impl AutomaticInsertion {
    fn is_healthy(self, work_rect: Rect) -> bool {
        let aspect =
            f64::from(self.newcomer_slot.w.max(1)) / f64::from(self.newcomer_slot.h.max(1));
        let target_aspect =
            f64::from(self.target_slot.w.max(1)) / f64::from(self.target_slot.h.max(1));
        self.fits_constraints
            && self
                .newcomer_slot
                .w
                .saturating_mul(MIN_HEALTHY_WORK_FRACTION)
                >= work_rect.w
            && self
                .newcomer_slot
                .h
                .saturating_mul(MIN_HEALTHY_WORK_FRACTION)
                >= work_rect.h
            && (MIN_HEALTHY_ASPECT_RATIO..=MAX_HEALTHY_ASPECT_RATIO).contains(&aspect)
            && (MIN_HEALTHY_ASPECT_RATIO..=MAX_HEALTHY_ASPECT_RATIO).contains(&target_aspect)
    }
}

fn sane_weight(weight: f64) -> f64 {
    if weight.is_finite() && weight > 0.0 {
        weight
    } else {
        1.0
    }
}

fn finite_clamp(value: f64, minimum: f64, maximum: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}

/// Construct a canonical split, collapsing zero/one-child results and folding
/// direct same-axis splits into the surrounding weighted run.
fn make_split(id: SplitId, axis: Axis, items: Vec<WeightedNode>) -> Option<Node> {
    let mut flattened = Vec::new();
    for item in items {
        let parent_weight = sane_weight(item.weight);
        match item.node {
            Node::Split(child_split) if child_split.axis == axis => {
                flattened.extend(child_split.children.into_iter().map(|child| WeightedNode {
                    node: child.node,
                    weight: parent_weight * child.weight,
                }));
            }
            node => flattened.push(WeightedNode {
                node,
                weight: parent_weight,
            }),
        }
    }

    match flattened.len() {
        0 => None,
        1 => Some(flattened.pop().expect("length checked").node),
        _ => {
            let total: f64 = flattened.iter().map(|child| child.weight).sum();
            for child in &mut flattened {
                child.weight /= total;
            }
            Some(Node::Split(Split {
                id,
                axis,
                children: flattened,
            }))
        }
    }
}

fn balanced_group_sizes(item_count: usize, group_count: usize) -> Vec<usize> {
    debug_assert!(item_count > 0);
    debug_assert!((1..=item_count).contains(&group_count));
    let small = item_count / group_count;
    let large_groups = item_count % group_count;
    (0..group_count)
        .map(|index| {
            // Put larger groups at the trailing edge. For three items this
            // yields a half-height stack beside a full-height newcomer.
            small + usize::from(index >= group_count - large_groups)
        })
        .collect()
}

fn build_grouped_nodes(
    items: &[Node],
    outer_axis: Axis,
    group_sizes: &[usize],
    first_split_id: u64,
) -> (Node, u64) {
    let mut next_split_id = first_split_id;
    let mut allocate = || {
        let id = SplitId(next_split_id);
        next_split_id = next_split_id
            .checked_add(1)
            .expect("manual-layout split id space exhausted");
        id
    };
    let mut offset = 0;
    let mut groups = Vec::with_capacity(group_sizes.len());
    for &group_size in group_sizes {
        let members = &items[offset..offset + group_size];
        offset += group_size;
        let node = if let [node] = members {
            node.clone()
        } else {
            make_split(
                allocate(),
                outer_axis.other(),
                members
                    .iter()
                    .cloned()
                    .map(|node| WeightedNode { node, weight: 1.0 })
                    .collect(),
            )
            .expect("a non-empty force-packing group creates a node")
        };
        groups.push(WeightedNode { node, weight: 1.0 });
    }
    debug_assert_eq!(offset, items.len());

    let root = if let [group] = groups.as_slice() {
        group.node.clone()
    } else {
        make_split(allocate(), outer_axis, groups)
            .expect("non-empty force-packing groups create a root")
    };
    (root, next_split_id)
}

#[derive(Debug, Clone, Copy)]
struct FRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl FRect {
    fn from_rect(rect: Rect) -> Self {
        Self {
            x: f64::from(rect.x),
            y: f64::from(rect.y),
            w: f64::from(rect.w.max(0)),
            h: f64::from(rect.h.max(0)),
        }
    }

    fn to_rect(self) -> Rect {
        let x = self.x.round() as i32;
        let y = self.y.round() as i32;
        let right = (self.x + self.w).round() as i32;
        let bottom = (self.y + self.h).round() as i32;
        Rect::new(x, y, (right - x).max(1), (bottom - y).max(1))
    }

    fn right(self) -> f64 {
        self.x + self.w
    }

    fn bottom(self) -> f64 {
        self.y + self.h
    }

    fn axis_start(self, axis: Axis) -> f64 {
        match axis {
            Axis::Vertical => self.x,
            Axis::Horizontal => self.y,
        }
    }

    fn axis_size(self, axis: Axis) -> f64 {
        match axis {
            Axis::Vertical => self.w,
            Axis::Horizontal => self.h,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutTree {
    root: Option<Node>,
    next_split_id: u64,
    /// Newest-first windows placed by consecutive force insertions since the
    /// last manual or non-force tree edit. This is explicit provenance, not a
    /// geometry heuristic: while it remains valid, later force insertions may
    /// repack these leaves around the untouched pre-existing tree.
    untouched_force_windows: Vec<WindowId>,
}

/// Lazily memoized pointer-placement previews for one stable layout snapshot.
///
/// Pointer motion still resolves the exact target and trigger band on every
/// sample, but each target edge's structural candidates and constrained slots
/// are materialized at most once for the lifetime of this cache. Invalid
/// candidates and weight-only variants of the same canonical topology are
/// removed before the surviving outcomes divide the edge trigger zone.
#[derive(Debug, Clone)]
pub(crate) struct PointerPlacementCache {
    tree: LayoutTree,
    source: WindowId,
    layout_rect: Rect,
    edge_fraction: f64,
    minimums: HashMap<WindowId, Size>,
    bounds: HashMap<WindowId, Rect>,
    center_slots: HashMap<WindowId, Option<PointerPlacementResolution>>,
    edge_slots: HashMap<(WindowId, Side), Vec<PointerPlacementResolution>>,
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self {
            root: None,
            next_split_id: 1,
            untouched_force_windows: Vec::new(),
        }
    }
}

impl LayoutTree {
    fn allocate(&mut self) -> SplitId {
        let id = SplitId(self.next_split_id);
        self.next_split_id = self
            .next_split_id
            .checked_add(1)
            .expect("manual-layout split id space exhausted");
        id
    }

    fn invalidate_force_provenance(&mut self) {
        self.untouched_force_windows.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn len(&self) -> usize {
        self.root.as_ref().map_or(0, Node::leaf_count)
    }

    pub fn leaves(&self) -> Vec<WindowId> {
        let mut leaves = Vec::with_capacity(self.len());
        if let Some(root) = &self.root {
            root.leaves(&mut leaves);
        }
        leaves
    }
}

#[cfg(test)]
mod tests;
