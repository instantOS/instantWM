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
mod placement_ops;
mod presets;
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
pub struct SplitId(u64);

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

/// Minimum overlap between two source-window previews for them to represent
/// the same user-visible destination. Placement candidates may reach the same
/// slot through different structural scopes, which can redistribute space
/// among peers and give the source slightly different dimensions. The preview
/// only exposes the source rectangle, so comparing every hidden peer makes
/// those indistinguishable choices leak into keyboard and pointer navigation.
const PLACEMENT_PREVIEW_OVERLAP: f64 = 0.75;

#[derive(Debug)]
struct PlacementOutcome {
    leaves: Vec<WindowId>,
    preview: FRect,
}

impl PlacementOutcome {
    fn approximately_eq(&self, other: &Self) -> bool {
        if self.leaves != other.leaves {
            return false;
        }
        placement_previews_approximately_eq(self.preview, other.preview)
    }
}

fn placement_previews_approximately_eq(first: FRect, second: FRect) -> bool {
    let intersection_width = (first.right().min(second.right()) - first.x.max(second.x)).max(0.0);
    let intersection_height =
        (first.bottom().min(second.bottom()) - first.y.max(second.y)).max(0.0);
    let intersection = intersection_width * intersection_height;
    let union = first.w * first.h + second.w * second.h - intersection;
    union > EPSILON && intersection / union + EPSILON >= PLACEMENT_PREVIEW_OVERLAP
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

    /// Reconcile visible tiled windows using an explicit new-window policy.
    ///
    /// Existing leaves retain their topology and weights. The policy is
    /// consulted only for genuinely absent leaves, so changing configuration
    /// never rewrites an established manual layout.
    pub fn reconcile_for_layout(
        &mut self,
        visible: &[WindowId],
        policy: NewWindowPlacement,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) {
        let wanted: HashSet<_> = visible.iter().copied().collect();
        for stale in self
            .leaves()
            .into_iter()
            .filter(|window| !wanted.contains(window))
            .collect::<Vec<_>>()
        {
            self.remove(stale);
        }

        for &window in visible {
            if !self.root.as_ref().is_some_and(|root| root.contains(window)) {
                self.insert_new(window, policy, work_rect, minimums);
            }
        }
    }

    pub fn remove(&mut self, window: WindowId) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(window) {
            self.root = Some(root);
            return false;
        }
        self.root = root.remove(window);
        self.untouched_force_windows
            .retain(|candidate| *candidate != window);
        true
    }

    fn split_leaf(root: Node, target: WindowId, window: WindowId, axis: Axis, id: SplitId) -> Node {
        let split = make_split(
            id,
            axis,
            vec![
                WeightedNode {
                    node: Node::Window(window),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(target),
                    weight: 1.0,
                },
            ],
        )
        .expect("two leaves create a split");
        root.replace_window(target, split)
    }

    fn root_split(
        root: Node,
        window: WindowId,
        axis: Axis,
        newcomer_weight: f64,
        id: SplitId,
    ) -> Node {
        make_split(
            id,
            axis,
            vec![
                WeightedNode {
                    node: Node::Window(window),
                    weight: newcomer_weight,
                },
                WeightedNode {
                    node: root,
                    weight: 1.0 - newcomer_weight,
                },
            ],
        )
        .expect("a newcomer and an existing root create a split")
    }

    fn preferred_axis(rect: Rect, leading_fraction: f64) -> Axis {
        let quality = |axis| {
            let (width, height) = match axis {
                Axis::Vertical => (f64::from(rect.w) * leading_fraction, f64::from(rect.h)),
                Axis::Horizontal => (f64::from(rect.w), f64::from(rect.h) * leading_fraction),
            };
            ((width.max(1.0) / height.max(1.0)) / IDEAL_TILED_ASPECT_RATIO)
                .ln()
                .abs()
        };
        if quality(Axis::Vertical) <= quality(Axis::Horizontal) {
            Axis::Vertical
        } else {
            Axis::Horizontal
        }
    }

    /// Score every `(leaf, axis)` insertion by materializing a candidate split
    /// and solving it.
    ///
    /// This is quadratic in the leaf count — each candidate clones and re-solves
    /// the whole tree — but the leaf count per tag stays small in practice, and
    /// `make_split`'s same-axis canonicalization means a candidate's real slots
    /// can only be obtained by rebuilding the tree, not by solving the existing
    /// layout with a virtual split.
    fn automatic_insertion(
        root: &Node,
        window: WindowId,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Option<AutomaticInsertion> {
        let work_area = f64::from(work_rect.w.max(1)) * f64::from(work_rect.h.max(1));
        let mut best: Option<AutomaticInsertion> = None;

        let mut leaves = Vec::new();
        root.leaves(&mut leaves);
        for target in leaves {
            for axis in [Axis::Vertical, Axis::Horizontal] {
                let candidate = Self::split_leaf(root.clone(), target, window, axis, SplitId(0));
                let tree = Self {
                    root: Some(candidate),
                    next_split_id: 1,
                    untouched_force_windows: Vec::new(),
                };
                let constrained = tree.constrained_bounds(work_rect, minimums);
                let fits_constraints = constrained.is_some();
                let slots = constrained.unwrap_or_else(|| tree.bounds(work_rect));
                let Some(slot) = slots.get(&window).copied() else {
                    continue;
                };
                let Some(target_slot) = slots.get(&target).copied() else {
                    continue;
                };
                let aspect = f64::from(slot.w.max(1)) / f64::from(slot.h.max(1));
                let target_aspect =
                    f64::from(target_slot.w.max(1)) / f64::from(target_slot.h.max(1));
                let aspect_penalty = (aspect / IDEAL_TILED_ASPECT_RATIO).ln().abs();
                let target_aspect_penalty = (target_aspect / IDEAL_TILED_ASPECT_RATIO).ln().abs();
                let area_fraction = (f64::from(slot.w.max(1)) * f64::from(slot.h.max(1))
                    / work_area)
                    .clamp(EPSILON, 1.0);
                let area_penalty = -area_fraction.ln() * 0.25;
                let constraint_penalty = if fits_constraints { 0.0 } else { 1000.0 };
                let score =
                    constraint_penalty + aspect_penalty + target_aspect_penalty + area_penalty;

                let candidate = AutomaticInsertion {
                    score,
                    target,
                    axis,
                    newcomer_slot: slot,
                    target_slot,
                    fits_constraints,
                };
                if best.is_none_or(|best| candidate.score < best.score) {
                    best = Some(candidate);
                }
            }
        }

        best
    }

    fn insert_new(
        &mut self,
        window: WindowId,
        policy: NewWindowPlacement,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) {
        let Some(root) = self.root.take() else {
            self.root = Some(Node::Window(window));
            self.invalidate_force_provenance();
            return;
        };
        if root.contains(window) {
            self.root = Some(root);
            return;
        }

        if policy == NewWindowPlacement::Force {
            self.insert_force(root, window, work_rect, minimums);
            return;
        }

        self.root = Some(match policy {
            NewWindowPlacement::Force => unreachable!("handled above"),
            NewWindowPlacement::Auto | NewWindowPlacement::AutoResize => {
                self.invalidate_force_provenance();
                let Some(candidate) = Self::automatic_insertion(&root, window, work_rect, minimums)
                else {
                    self.root = Some(root);
                    return;
                };
                let id = self.allocate();
                if policy == NewWindowPlacement::AutoResize && !candidate.is_healthy(work_rect) {
                    let assisted_axis =
                        Self::preferred_axis(work_rect, AUTO_RESIZE_NEW_ROOT_WEIGHT);
                    let assisted = Self::root_split(
                        root.clone(),
                        window,
                        assisted_axis,
                        AUTO_RESIZE_NEW_ROOT_WEIGHT,
                        id,
                    );
                    let assisted_fits_constraints = Self {
                        root: Some(assisted.clone()),
                        next_split_id: self.next_split_id,
                        untouched_force_windows: Vec::new(),
                    }
                    .constrained_bounds(work_rect, minimums)
                    .is_some();
                    if assisted_fits_constraints || !candidate.fits_constraints {
                        assisted
                    } else {
                        Self::split_leaf(root, candidate.target, window, candidate.axis, id)
                    }
                } else {
                    Self::split_leaf(root, candidate.target, window, candidate.axis, id)
                }
            }
        });
    }

    fn insert_force(
        &mut self,
        root: Node,
        window: WindowId,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) {
        if self.untouched_force_windows.is_empty() {
            let id = self.allocate();
            self.root = Some(Self::root_split(root, window, Axis::Vertical, 0.5, id));
            self.untouched_force_windows.push(window);
            return;
        }

        let mut base = Some(root);
        let mut generated = Vec::new();
        for &candidate in &self.untouched_force_windows {
            let Some(current) = base.take() else {
                break;
            };
            if current.contains(candidate) {
                generated.push(Node::Window(candidate));
                base = current.remove(candidate);
            } else {
                base = Some(current);
            }
        }

        // A force cohort always has an older base when it is created, but that
        // base may since have closed. Keep every surviving generated leaf and
        // treat the surviving non-generated tree as one opaque layout item.
        let mut items = Vec::with_capacity(generated.len() + 2);
        items.push(Node::Window(window));
        items.extend(generated);
        if let Some(base) = base {
            items.push(base);
        }

        let (root, next_split_id) =
            Self::best_force_packing(items, work_rect, minimums, self.next_split_id);
        self.root = Some(root);
        self.next_split_id = next_split_id;
        self.untouched_force_windows.insert(0, window);
    }

    fn best_force_packing(
        items: Vec<Node>,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
        first_split_id: u64,
    ) -> (Node, u64) {
        debug_assert!(items.len() >= 2);
        let work_aspect = f64::from(work_rect.w.max(1)) / f64::from(work_rect.h.max(1));
        let mut best: Option<(f64, Node, u64)> = None;

        for outer_axis in [Axis::Vertical, Axis::Horizontal] {
            for group_count in 1..=items.len() {
                let group_sizes = balanced_group_sizes(items.len(), group_count);
                let mut penalties = Vec::with_capacity(items.len());
                for &group_size in &group_sizes {
                    let aspect = match outer_axis {
                        Axis::Vertical => work_aspect * group_size as f64 / group_count as f64,
                        Axis::Horizontal => work_aspect * group_count as f64 / group_size as f64,
                    };
                    let penalty = (aspect / IDEAL_TILED_ASPECT_RATIO).ln().abs();
                    penalties.extend(std::iter::repeat_n(penalty, group_size));
                }
                let worst = penalties.iter().copied().fold(0.0, f64::max);
                let average = penalties.iter().sum::<f64>() / penalties.len() as f64;

                let (candidate, next_split_id) =
                    build_grouped_nodes(&items, outer_axis, &group_sizes, first_split_id);
                let fits_constraints = Self {
                    root: Some(candidate.clone()),
                    next_split_id,
                    untouched_force_windows: Vec::new(),
                }
                .constrained_bounds(work_rect, minimums)
                .is_some();
                let score = worst * 2.0 + average + if fits_constraints { 0.0 } else { 1000.0 };
                if best
                    .as_ref()
                    .is_none_or(|(best_score, _, _)| score + EPSILON < *best_score)
                {
                    best = Some((score, candidate, next_split_id));
                }
            }
        }

        let (_, root, next_split_id) = best.expect("at least one force packing candidate");
        (root, next_split_id)
    }

    pub fn bounds(&self, rect: Rect) -> HashMap<WindowId, Rect> {
        let mut float_bounds = HashMap::new();
        if let Some(root) = &self.root {
            root.bounds(FRect::from_rect(rect), &mut float_bounds);
        }
        float_bounds
            .into_iter()
            .map(|(window, rect)| (window, rect.to_rect()))
            .collect()
    }

    /// Resolve tree slots while enforcing each leaf's minimum outer size.
    ///
    /// Returns `None` when the tree cannot fit all requirements in `rect`.
    /// Callers can therefore reject a placement or resize atomically instead
    /// of asking a backend to realize overlapping geometry.
    pub fn constrained_bounds(
        &self,
        rect: Rect,
        minimums: &HashMap<WindowId, Size>,
    ) -> Option<HashMap<WindowId, Rect>> {
        let root = self.root.as_ref()?;
        let required = required_size(root, minimums);
        if required.w > rect.w || required.h > rect.h {
            return None;
        }
        let mut output = HashMap::new();
        constrained_node_bounds(root, FRect::from_rect(rect), minimums, &mut output)?;
        Some(
            output
                .into_iter()
                .map(|(window, bounds)| (window, bounds.to_rect()))
                .collect(),
        )
    }

    fn float_bounds(&self) -> HashMap<WindowId, FRect> {
        let mut output = HashMap::new();
        if let Some(root) = &self.root {
            root.bounds(
                FRect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                &mut output,
            );
        }
        output
    }

    fn all_float_bounds(&self) -> HashMap<NodeKey, FRect> {
        let mut output = HashMap::new();
        if let Some(root) = &self.root {
            root.all_bounds(
                FRect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                &mut output,
            );
        }
        output
    }

    pub fn apply_preset(
        &mut self,
        preset: Preset,
        ordered_windows: &[WindowId],
        master_count: usize,
    ) {
        self.invalidate_force_provenance();
        let master_ratio = match preset {
            Preset::MasterStack => self.root_leading_ratio(Axis::Vertical),
            Preset::BottomStack | Preset::BottomStackHorizontal => {
                self.root_leading_ratio(Axis::Horizontal)
            }
            Preset::Grid | Preset::HorizontalGrid => 0.5,
        };
        let wanted = ordered_windows.iter().copied().collect::<HashSet<_>>();
        for stale in self
            .leaves()
            .into_iter()
            .filter(|window| !wanted.contains(window))
            .collect::<Vec<_>>()
        {
            self.remove(stale);
        }
        let mut windows = self.leaves();
        for &window in ordered_windows {
            if !windows.contains(&window) {
                windows.push(window);
            }
        }
        if windows.is_empty() {
            return;
        }
        let next = &mut self.next_split_id;
        let mut allocate = || {
            let id = SplitId(*next);
            *next = next
                .checked_add(1)
                .expect("manual-layout split id space exhausted");
            id
        };
        self.root = match preset {
            Preset::MasterStack => build_master_stack(
                &windows,
                master_count,
                master_ratio,
                Axis::Vertical,
                &mut allocate,
            ),
            Preset::BottomStack => build_master_stack(
                &windows,
                master_count,
                master_ratio,
                Axis::Horizontal,
                &mut allocate,
            ),
            Preset::Grid => build_grid(&windows, false, &mut allocate),
            Preset::HorizontalGrid => build_grid(&windows, true, &mut allocate),
            Preset::BottomStackHorizontal => {
                build_master_stack(&windows, 1, master_ratio, Axis::Horizontal, &mut allocate)
            }
        };
    }

    /// Preserve the leading share of an existing root split when a preset is
    /// reapplied. The tree is the source of truth for proportions; presets use
    /// an even split when the current root has no corresponding axis.
    fn root_leading_ratio(&self, axis: Axis) -> f64 {
        let Some(Node::Split(split)) = &self.root else {
            return 0.5;
        };
        if split.axis != axis {
            return 0.5;
        }
        split.children.first().map_or(0.5, |child| child.weight)
    }

    /// Swap the focused leaf with the topology-first visual neighbour.
    pub fn swap_with_neighbor(&mut self, source: WindowId, side: Side) -> Option<WindowId> {
        let neighbor = self.visual_neighbor(source, side)?;
        let root = self.root.take()?;
        self.root = Some(swap_windows(root, source, neighbor));
        self.invalidate_force_provenance();
        Some(neighbor)
    }

    pub fn visual_neighbor(&self, source: WindowId, side: Side) -> Option<WindowId> {
        let root = self.root.as_ref()?;
        let rects = self.float_bounds();
        let source_rect = *rects.get(&source)?;
        visual_neighbor_in(root, source, source_rect, side, &rects).0
    }

    /// Resize the nearest applicable axis run. Peer ratios are preserved.
    pub fn resize(&mut self, source: WindowId, side: Side) -> bool {
        self.resize_with_config(source, side, CommandConfig::default())
    }

    pub fn resize_with_config(
        &mut self,
        source: WindowId,
        side: Side,
        config: CommandConfig,
    ) -> bool {
        let axis = side.axis();
        let grow = matches!(side, Side::Top | Side::Right);
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, changed) = resize_deepest_run(root, source, axis, grow, config);
        self.root = Some(root);
        if changed {
            self.invalidate_force_provenance();
        }
        changed
    }

    /// Resize the deepest applicable run by a pointer displacement.
    ///
    /// `pixels` is the movement of the grabbed physical edge: positive means
    /// right/down and negative means left/up. It is normalized against the
    /// actual containing run rather than the whole monitor, so nested splits
    /// track the pointer one-for-one.
    pub fn resize_by_pixels(
        &mut self,
        source: WindowId,
        side: Side,
        pixels: i32,
        layout_rect: Rect,
        minimum_weight: f64,
    ) -> bool {
        if pixels == 0 {
            return false;
        }
        let axis = side.axis();
        let Some(normalized_span) = self.resize_span(source, axis) else {
            return false;
        };
        let layout_span = match axis {
            Axis::Vertical => layout_rect.w,
            Axis::Horizontal => layout_rect.h,
        };
        let physical_span = normalized_span * f64::from(layout_span.max(1));
        if physical_span <= EPSILON {
            return false;
        }
        let edge_delta = f64::from(pixels) / physical_span;
        let weight_delta = if side.is_leading() {
            -edge_delta
        } else {
            edge_delta
        };
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, changed) =
            resize_deepest_run_by(root, source, axis, weight_delta, minimum_weight);
        self.root = Some(root);
        if changed {
            self.invalidate_force_provenance();
        }
        changed
    }

    /// Move only the grabbed seam, keeping the source's opposite edge fixed.
    ///
    /// Unlike keyboard growth, pointer resizing transfers space solely from
    /// the source branch's peers on `side`. Their existing ratios are
    /// preserved, while peers beyond the source's opposite edge stay fixed.
    pub fn resize_edge_by_pixels(
        &mut self,
        source: WindowId,
        side: Side,
        pixels: i32,
        layout_rect: Rect,
        minimum_weight: f64,
    ) -> bool {
        if pixels == 0 {
            return false;
        }
        let axis = side.axis();
        let Some(normalized_span) = self.resize_edge_span(source, side) else {
            return false;
        };
        let layout_span = match axis {
            Axis::Vertical => layout_rect.w,
            Axis::Horizontal => layout_rect.h,
        };
        let physical_span = normalized_span * f64::from(layout_span.max(1));
        if physical_span <= EPSILON {
            return false;
        }
        let edge_delta = f64::from(pixels) / physical_span;
        let source_delta = if side.is_leading() {
            -edge_delta
        } else {
            edge_delta
        };
        let Some(root) = self.root.take() else {
            return false;
        };
        let (root, changed) =
            resize_deepest_edge_by(root, source, side, source_delta, minimum_weight);
        self.root = Some(root);
        if changed {
            self.invalidate_force_provenance();
        }
        changed
    }

    /// Whether `source` belongs to a split that can be resized on `axis`.
    pub fn can_resize_axis(&self, source: WindowId, axis: Axis) -> bool {
        self.resize_span(source, axis).is_some()
    }

    /// Whether the physical edge is backed by an adjustable tree seam.
    pub fn can_resize_side(&self, source: WindowId, side: Side) -> bool {
        self.resize_edge_span(source, side).is_some()
    }

    fn resize_span(&self, source: WindowId, axis: Axis) -> Option<f64> {
        let root = self.root.as_ref()?;
        let split = deepest_resize_split(root, source, axis)?;
        let bounds = self.all_float_bounds();
        let rect = bounds.get(&NodeKey::Split(split))?;
        Some(rect.axis_size(axis))
    }

    fn resize_edge_span(&self, source: WindowId, side: Side) -> Option<f64> {
        let root = self.root.as_ref()?;
        let split = deepest_resize_edge_split(root, source, side)?;
        let bounds = self.all_float_bounds();
        let rect = bounds.get(&NodeKey::Split(split))?;
        Some(rect.axis_size(side.axis()))
    }

    pub fn resize_smart(&mut self, source: WindowId, grow: bool) -> bool {
        self.resize_smart_with_config(source, grow, CommandConfig::default())
    }

    pub fn resize_smart_with_config(
        &mut self,
        source: WindowId,
        grow: bool,
        config: CommandConfig,
    ) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        let Some(axis) = immediate_parent_axis(root, source) else {
            return false;
        };
        let side = match (axis, grow) {
            (Axis::Vertical, true) => Side::Right,
            (Axis::Vertical, false) => Side::Left,
            (Axis::Horizontal, true) => Side::Top,
            (Axis::Horizontal, false) => Side::Bottom,
        };
        self.resize_with_config(source, side, config)
    }

    /// Move `source` beside `target`. The requested side selects the split axis;
    /// canonicalisation automatically inserts into an existing matching run.
    pub fn move_beside(&mut self, source: WindowId, target: WindowId, side: Side) -> bool {
        if source == target {
            return false;
        }
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(source) || !root.contains(target) {
            self.root = Some(root);
            return false;
        }
        let id = self.allocate();
        let without_source = root
            .remove(source)
            .expect("moving one of at least two leaves leaves a root");
        let (first, second) = if side.is_leading() {
            (source, target)
        } else {
            (target, source)
        };
        let replacement = make_split(
            id,
            side.axis(),
            vec![
                WeightedNode {
                    node: Node::Window(first),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(second),
                    weight: 1.0,
                },
            ],
        )
        .expect("two leaves create a split");
        self.root = Some(without_source.replace_window(target, replacement));
        self.invalidate_force_provenance();
        true
    }

    fn edge_candidates(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
    ) -> Vec<EdgeCandidate> {
        let rects = self.all_float_bounds();
        let leaf_rects = self.float_bounds();
        self.edge_candidates_with_geometry(source, target, side, &rects, &leaf_rects)
    }

    fn edge_candidates_with_geometry(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
        rects: &HashMap<NodeKey, FRect>,
        leaf_rects: &HashMap<WindowId, FRect>,
    ) -> Vec<EdgeCandidate> {
        self.resolved_edge_candidates_with_geometry(source, target, side, rects, leaf_rects)
            .into_iter()
            .map(|(candidate, _)| candidate)
            .collect()
    }

    fn resolved_edge_candidates_with_geometry(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
        rects: &HashMap<NodeKey, FRect>,
        leaf_rects: &HashMap<WindowId, FRect>,
    ) -> Vec<(EdgeCandidate, LayoutTree)> {
        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };
        let Some(target_rect) = leaf_rects.get(&target).copied() else {
            return Vec::new();
        };
        let mut path = Vec::new();
        if !path_to(root, target, &mut path) {
            return Vec::new();
        }

        let axis = side.axis();
        let mut exposed = vec![NodeKey::Window(target)];
        for (split, branch_index) in path.iter().rev() {
            if split.axis == axis {
                let edge_index = if side.is_leading() {
                    0
                } else {
                    split.children.len() - 1
                };
                if *branch_index != edge_index {
                    break;
                }
            }
            exposed.push(NodeKey::Split(split.id));
        }

        let seam = target_rect.axis_start(axis)
            + if side.is_leading() {
                0.0
            } else {
                target_rect.axis_size(axis)
            };
        let mut candidates = exposed
            .into_iter()
            .filter_map(|key| {
                rects.get(&key)?;
                let scope_depth = match key {
                    NodeKey::Window(_) => 0,
                    NodeKey::Split(id) => path
                        .iter()
                        .position(|(split, _)| split.id == id)
                        .map_or(0, |index| path.len() - index),
                };
                Some(EdgeCandidate {
                    scope: PlacementScope::Node(key),
                    scope_depth,
                })
            })
            .collect::<Vec<_>>();

        // Recover aligned pseudo-seams, including rectangular contiguous child
        // ranges hidden by canonical same-axis flattening.
        for (split, branch_index) in &path {
            let scope_key = NodeKey::Split(split.id);
            let rect = rects[&scope_key];
            let tolerance = rect.axis_size(axis) * 0.04;
            let target_cross_size = cross_size(target_rect, axis);
            let parent_cross_size = cross_size(rect, axis);
            if parent_cross_size > target_cross_size + tolerance
                && seam > rect.axis_start(axis) + tolerance
                && seam < rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                && let Some(before) =
                    seam_partition(&split.children, seam, axis, leaf_rects, tolerance)
            {
                candidates.push(EdgeCandidate {
                    scope: PlacementScope::AlignedNode {
                        key: scope_key,
                        seam,
                        before,
                    },
                    scope_depth: path
                        .iter()
                        .position(|(candidate, _)| candidate.id == split.id)
                        .map_or(0, |index| path.len() - index),
                });
            }
            // Every contiguous child range is contained by its parent. If the
            // parent is no wider than the target on the cross axis, no range
            // can expose an aligned seam either. Avoid the O(k²) range scan
            // (and its repeated O(k) geometry collection) for common flat
            // k-window runs.
            if parent_cross_size <= target_cross_size {
                continue;
            }
            for first in 0..=*branch_index {
                for last in *branch_index..split.children.len() {
                    if first == 0 && last + 1 == split.children.len() {
                        continue;
                    }
                    let children = &split.children[first..=last];
                    let selected_rects = children
                        .iter()
                        .filter_map(|child| rects.get(&child.node.key()).copied())
                        .collect::<Vec<_>>();
                    let Some(rect) = bounding_rect(&selected_rects) else {
                        continue;
                    };
                    let tolerance = rect.axis_size(axis) * 0.04;
                    if cross_size(rect, axis) <= target_cross_size + tolerance
                        || seam <= rect.axis_start(axis) + tolerance
                        || seam >= rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                    {
                        continue;
                    }
                    let Some(before) = seam_partition(children, seam, axis, leaf_rects, tolerance)
                    else {
                        continue;
                    };
                    candidates.push(EdgeCandidate {
                        scope: PlacementScope::AlignedChildRange {
                            parent: split.id,
                            children: children.iter().map(|child| child.node.key()).collect(),
                            seam,
                            before,
                        },
                        scope_depth: path
                            .iter()
                            .position(|(candidate, _)| candidate.id == split.id)
                            .map_or(0, |index| path.len() - index),
                    });
                }
            }
        }

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.scope_depth));
        // Different structural scopes can normalize to the same visual
        // result. The prototype exposes one band per distinct result, so
        // preview each command and collapse geometry-equivalent candidates.
        let mut geometries = HashSet::new();
        let mut distinct = Vec::new();
        for candidate in candidates.into_iter().rev() {
            let mut preview = self.clone();
            if !preview.move_to_scope(source, target, side, candidate.scope.clone()) {
                continue;
            }
            let rects = preview.float_bounds();
            let mut signature = rects
                .into_iter()
                .map(|(window, rect)| {
                    (
                        window,
                        (rect.x * 10_000.0).round() as i64,
                        (rect.y * 10_000.0).round() as i64,
                        (rect.w * 10_000.0).round() as i64,
                        (rect.h * 10_000.0).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            signature.sort_by_key(|item| item.0);
            if geometries.insert(signature) {
                distinct.push((candidate, preview));
            }
        }
        distinct.reverse();
        distinct
    }

    /// Enumerate distinct user-visible placement results. Raw hit regions can
    /// describe one result in several ways (for example, either side of a
    /// shared seam); only the first deterministic representative is exposed.
    pub fn placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<PlacementTarget> {
        self.normalized_placement_candidates(source, layout_rect, edge_fraction)
            .into_iter()
            .map(|(target, _)| target)
            .collect()
    }

    pub(crate) fn constrained_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
        minimums: &HashMap<WindowId, Size>,
    ) -> Vec<PlacementTarget> {
        self.normalized_placement_candidates(source, layout_rect, edge_fraction)
            .into_iter()
            .filter_map(|(target, candidate)| {
                candidate
                    .constrained_bounds(layout_rect, minimums)
                    .is_some()
                    .then_some(target)
            })
            .collect()
    }

    fn normalized_placement_candidates(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<(PlacementTarget, LayoutTree)> {
        let mut previews_by_order = HashMap::<Vec<WindowId>, Vec<FRect>>::new();
        let mut distinct = Vec::new();
        for resolved in self.raw_resolved_placement_targets(source, layout_rect, edge_fraction) {
            let candidate = resolved.candidate;
            let Some(preview) = candidate.float_bounds().get(&source).copied() else {
                continue;
            };
            let order = candidate.leaves();
            let previews = previews_by_order.entry(order).or_default();
            if previews
                .iter()
                .any(|existing| placement_previews_approximately_eq(*existing, preview))
            {
                continue;
            }
            previews.push(preview);
            distinct.push((resolved.target, candidate));
        }
        distinct
    }

    fn raw_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<PlacementTarget> {
        self.raw_resolved_placement_targets(source, layout_rect, edge_fraction)
            .into_iter()
            .map(|resolved| resolved.target)
            .collect()
    }

    fn raw_resolved_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<ResolvedPlacementTarget> {
        let bounds = self.bounds(layout_rect);
        let node_bounds = self.all_float_bounds();
        let leaf_bounds = self.float_bounds();
        let fraction = finite_clamp(edge_fraction, 0.05, 0.49, 0.34);
        let mut output = Vec::new();
        for target in self.leaves().into_iter().filter(|window| *window != source) {
            let Some(rect) = bounds.get(&target).copied() else {
                continue;
            };
            output.push(ResolvedPlacementTarget {
                target: PlacementTarget {
                    target,
                    side: None,
                    candidate_index: 0,
                    position: rect.center(),
                },
                candidate: {
                    let mut candidate = self.clone();
                    let _ = candidate.swap_windows(source, target);
                    candidate
                },
            });
            for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
                let candidates = self.resolved_edge_candidates_with_geometry(
                    source,
                    target,
                    side,
                    &node_bounds,
                    &leaf_bounds,
                );
                let candidate_count = candidates.len();
                for (index, (_edge, candidate)) in candidates.into_iter().enumerate() {
                    let band_fraction = fraction * (index as f64 + 0.5) / candidate_count as f64;
                    let position = match side {
                        Side::Left => Point::new(
                            rect.x + (f64::from(rect.w) * band_fraction).round() as i32,
                            rect.center().y,
                        ),
                        Side::Right => Point::new(
                            rect.right() - (f64::from(rect.w) * band_fraction).round() as i32,
                            rect.center().y,
                        ),
                        Side::Top => Point::new(
                            rect.center().x,
                            rect.y + (f64::from(rect.h) * band_fraction).round() as i32,
                        ),
                        Side::Bottom => Point::new(
                            rect.center().x,
                            rect.bottom() - (f64::from(rect.h) * band_fraction).round() as i32,
                        ),
                    };
                    output.push(ResolvedPlacementTarget {
                        target: PlacementTarget {
                            target,
                            side: Some(side),
                            candidate_index: index,
                            position,
                        },
                        candidate,
                    });
                }
            }
        }
        output
    }

    fn placement_outcome(
        &self,
        source: WindowId,
        target: PlacementTarget,
    ) -> Option<PlacementOutcome> {
        let mut preview = self.clone();
        if !preview.apply_placement_target(source, target) {
            return None;
        }
        let preview_rect = preview.float_bounds().get(&source).copied()?;
        Some(PlacementOutcome {
            leaves: preview.leaves(),
            preview: preview_rect,
        })
    }

    pub fn apply_placement_target(&mut self, source: WindowId, target: PlacementTarget) -> bool {
        let Some(side) = target.side else {
            return self.swap_windows(source, target.target);
        };
        let candidates = self.edge_candidates(source, target.target, side);
        let Some(candidate) = candidates.get(target.candidate_index) else {
            return false;
        };
        self.move_to_scope(source, target.target, side, candidate.scope.clone())
    }

    /// Return the source slot produced by a semantic target without mutating
    /// the authoritative tree.
    pub fn preview_placement_target(
        &self,
        source: WindowId,
        target: PlacementTarget,
        layout_rect: Rect,
    ) -> Option<Rect> {
        let mut preview = self.clone();
        preview
            .apply_placement_target(source, target)
            .then(|| preview.bounds(layout_rect).get(&source).copied())
            .flatten()
    }

    /// Return the source slot produced by dropping at `point`, without
    /// mutating the authoritative tree. This deliberately calls
    /// [`Self::place_at_point`] on a clone so pointer previews and releases
    /// cannot drift into subtly different target-resolution rules.
    pub fn preview_placement_at_point(
        &self,
        source: WindowId,
        point: Point,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Option<Rect> {
        let mut preview = self.clone();
        preview
            .place_at_point(source, point, layout_rect, edge_fraction)
            .then(|| preview.bounds(layout_rect).get(&source).copied())
            .flatten()
    }

    fn move_to_scope(
        &mut self,
        source: WindowId,
        target: WindowId,
        side: Side,
        scope: PlacementScope,
    ) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(source) || !root.contains(target) || root.leaf_count() < 2 {
            self.root = Some(root);
            return false;
        }
        let Some(without_source) = root.remove(source) else {
            self.root = Some(Node::Window(source));
            return false;
        };
        let next = &mut self.next_split_id;
        let mut allocate = || {
            let id = SplitId(*next);
            *next = next
                .checked_add(1)
                .expect("manual-layout split id space exhausted");
            id
        };
        let rebuilt = match scope {
            PlacementScope::Node(mut key) => {
                if !without_source.contains_key(key) {
                    key = NodeKey::Window(target);
                }
                insert_at_scope_edge(
                    without_source.clone(),
                    key,
                    target,
                    source,
                    side,
                    allocate(),
                )
            }
            PlacementScope::AlignedNode { key, seam, before } => {
                let insertion = AlignedInsertion {
                    seam,
                    before: &before,
                    source,
                    axis: side.axis(),
                };
                insert_across_aligned_node(without_source.clone(), key, &insertion, &mut allocate)
            }
            PlacementScope::AlignedChildRange {
                parent,
                children,
                seam,
                before,
            } => {
                let insertion = AlignedInsertion {
                    seam,
                    before: &before,
                    source,
                    axis: side.axis(),
                };
                insert_across_aligned_range(
                    without_source.clone(),
                    parent,
                    &children,
                    &insertion,
                    &mut allocate,
                )
            }
        };
        let Some(rebuilt) = rebuilt else {
            self.root = Some(without_source);
            return false;
        };
        self.root = Some(rebuilt);
        self.invalidate_force_provenance();
        true
    }

    /// Resolve a pointer drop into a semantic local edge placement. The centre
    /// swaps slots; edge bands choose the corresponding side. This backend-free
    /// operation is shared by the X11 and Wayland drag completion paths.
    pub fn place_at_point(
        &mut self,
        source: WindowId,
        point: Point,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> bool {
        let bounds = self.bounds(layout_rect);
        let target = bounds.iter().find_map(|(&window, rect)| {
            (window != source && rect.contains_point(point)).then_some((window, *rect))
        });
        let Some((target, rect)) = target else {
            return false;
        };
        let edge_fraction = finite_clamp(edge_fraction, 0.05, 0.49, 0.34);
        let inset_x = (f64::from(rect.w) * edge_fraction).max(1.0);
        let inset_y = (f64::from(rect.h) * edge_fraction).max(1.0);
        let distances = [
            (Side::Left, f64::from(point.x - rect.x) / inset_x),
            (Side::Right, f64::from(rect.right() - point.x) / inset_x),
            (Side::Top, f64::from(point.y - rect.y) / inset_y),
            (Side::Bottom, f64::from(rect.bottom() - point.y) / inset_y),
        ];
        let nearest = distances
            .into_iter()
            .min_by(|left, right| left.1.total_cmp(&right.1));
        match nearest {
            Some((side, distance)) if distance <= 1.0 => {
                let candidates = self.edge_candidates(source, target, side);
                if candidates.is_empty() {
                    return self.move_beside(source, target, side);
                }
                let index = ((distance.max(0.0) * candidates.len() as f64).floor() as usize)
                    .min(candidates.len() - 1);
                let target = PlacementTarget {
                    target,
                    side: Some(side),
                    candidate_index: index,
                    position: point,
                };
                // The candidate under the pointer already describes the desired
                // placement. `placement_targets` normalizes equivalent commands
                // for keyboard navigation, but doing that global enumeration
                // here used to rebuild and compare every possible placement on
                // every pointer-motion event. Besides being unnecessary for a
                // direct hit test, that work grows polynomially with the number
                // of leaves and made interactive dragging noticeably laggy.
                self.apply_placement_target(source, target)
            }
            _ => self.swap_windows(source, target),
        }
    }

    pub fn swap_windows(&mut self, first: WindowId, second: WindowId) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if first == second || !root.contains(first) || !root.contains(second) {
            self.root = Some(root);
            return false;
        }
        self.root = Some(swap_windows(root, first, second));
        self.invalidate_force_provenance();
        true
    }

    /// Promote a window to the primary (master) slot using force insertion,
    /// or cycle the primary slot if the window is already primary.
    ///
    /// If `window` is not currently the first visual leaf (master), it is
    /// removed from its position and re-inserted using force placement,
    /// placing it into the primary slot.
    ///
    /// If `window` is already the first visual leaf, it is swapped with the
    /// next tiled window in `candidate_order`. This changes which window
    /// occupies the primary slot without changing the tree topology, split
    /// weights, or slot geometry.
    ///
    /// Returns `Some(promoted_window_id)` if the layout was updated, or `None`
    /// if no promotion was possible (e.g. single window or empty tree).
    pub fn promote(
        &mut self,
        window: WindowId,
        work_rect: Rect,
        minimums: &HashMap<WindowId, Size>,
        candidate_order: &[WindowId],
    ) -> Option<WindowId> {
        let leaves = self.leaves();
        if leaves.len() <= 1 {
            return None;
        }

        let is_primary = leaves.first() == Some(&window);
        if !is_primary {
            if !self.remove(window) {
                return None;
            }
            self.insert_new(window, NewWindowPlacement::Force, work_rect, minimums);
            Some(window)
        } else {
            let next_primary = candidate_order
                .iter()
                .position(|candidate| *candidate == window)
                .and_then(|current| {
                    candidate_order
                        .iter()
                        .cycle()
                        .skip(current + 1)
                        .take(candidate_order.len().saturating_sub(1))
                        .find(|candidate| leaves.contains(candidate))
                        .copied()
                })
                .or_else(|| {
                    leaves
                        .iter()
                        .copied()
                        .find(|candidate| *candidate != window)
                })?;
            self.swap_windows(window, next_primary)
                .then_some(next_primary)
        }
    }
}

#[cfg(test)]
mod tests;
