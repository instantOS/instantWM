//! Backend-independent manual tiling tree.
//!
//! A tree belongs to one monitor/tag-mask pair.  Splits are weighted n-ary
//! runs.  Construction is deliberately private and canonicalises adjacent
//! splits on the same axis, so the rest of the window manager cannot create a
//! one-child split, a non-positive weight, or redundant same-axis nesting.

use std::collections::{HashMap, HashSet};

use crate::types::{Point, Rect, WindowId};

const EPSILON: f64 = 1.0e-9;

mod placement_ops;
mod presets;
mod resize_ops;
mod types;
use placement_ops::*;
#[cfg(test)]
use presets::equal_run;
use presets::{build_focus, build_grid, build_master_stack};
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

/// Maximum per-edge/extent difference in unit-tree coordinates for two
/// placement results to represent the same user-visible destination.
const PLACEMENT_EQUIVALENCE_TOLERANCE: f64 = 0.04;

#[derive(Debug)]
struct PlacementOutcome {
    leaves: Vec<WindowId>,
    rects: Vec<(WindowId, FRect)>,
}

impl PlacementOutcome {
    fn approximately_eq(&self, other: &Self) -> bool {
        self.leaves == other.leaves
            && self.rects.len() == other.rects.len()
            && self.rects.iter().zip(&other.rects).all(
                |((left_window, left), (right_window, right))| {
                    left_window == right_window
                        && (left.x - right.x).abs() <= PLACEMENT_EQUIVALENCE_TOLERANCE
                        && (left.y - right.y).abs() <= PLACEMENT_EQUIVALENCE_TOLERANCE
                        && (left.w - right.w).abs() <= PLACEMENT_EQUIVALENCE_TOLERANCE
                        && (left.h - right.h).abs() <= PLACEMENT_EQUIVALENCE_TOLERANCE
                },
            )
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
    next_axis: Axis,
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self {
            root: None,
            next_split_id: 1,
            next_axis: Axis::Vertical,
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

    /// Make the leaf set exactly match `visible`, retaining all surviving
    /// topology. New windows split the least-populated branch and alternate
    /// axes, matching the prototype's balanced spawn policy.
    pub fn reconcile(&mut self, visible: &[WindowId]) {
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
                self.insert_balanced(window);
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
        true
    }

    fn deepest_balanced(node: &Node) -> WindowId {
        match node {
            Node::Window(window) => *window,
            Node::Split(split) => {
                let child = split
                    .children
                    .iter()
                    .min_by_key(|child| child.node.leaf_count())
                    .expect("canonical split has children");
                Self::deepest_balanced(&child.node)
            }
        }
    }

    pub fn insert_balanced(&mut self, window: WindowId) {
        let Some(root) = self.root.take() else {
            self.root = Some(Node::Window(window));
            return;
        };
        if root.contains(window) {
            self.root = Some(root);
            return;
        }

        let target = Self::deepest_balanced(&root);
        let axis = self.next_axis;
        self.next_axis = axis.other();
        let id = self.allocate();
        let split = make_split(
            id,
            axis,
            vec![
                WeightedNode {
                    node: Node::Window(target),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(window),
                    weight: 1.0,
                },
            ],
        )
        .expect("two leaves create a split");
        self.root = Some(root.replace_window(target, split));
        self.redistribute_axis(window, axis);
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
        selected: Option<WindowId>,
        master_count: usize,
        master_factor: f64,
    ) {
        self.reconcile(ordered_windows);
        let windows = self.leaves();
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
                master_factor,
                Axis::Vertical,
                &mut allocate,
            ),
            Preset::BottomStack => build_master_stack(
                &windows,
                master_count,
                master_factor,
                Axis::Horizontal,
                &mut allocate,
            ),
            Preset::Grid => build_grid(&windows, false, &mut allocate),
            Preset::HorizontalGrid => build_grid(&windows, true, &mut allocate),
            Preset::BottomStackHorizontal => {
                build_master_stack(&windows, 1, master_factor, Axis::Horizontal, &mut allocate)
            }
            Preset::Focus => build_focus(&windows, selected, &mut allocate),
        };
    }

    /// Swap the focused leaf with the topology-first visual neighbour.
    pub fn swap_with_neighbor(&mut self, source: WindowId, side: Side) -> Option<WindowId> {
        let neighbor = self.visual_neighbor(source, side)?;
        let root = self.root.take()?;
        self.root = Some(swap_windows(root, source, neighbor));
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
        changed
    }

    /// Whether `source` belongs to a split that can be resized on `axis`.
    pub fn can_resize_axis(&self, source: WindowId, axis: Axis) -> bool {
        self.resize_span(source, axis).is_some()
    }

    fn resize_span(&self, source: WindowId, axis: Axis) -> Option<f64> {
        let root = self.root.as_ref()?;
        let split = deepest_resize_split(root, source, axis)?;
        let bounds = self.all_float_bounds();
        let rect = bounds.get(&NodeKey::Split(split))?;
        Some(rect.axis_size(axis))
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

    fn redistribute_axis(&mut self, target: WindowId, axis: Axis) {
        let Some(root) = self.root.take() else {
            return;
        };
        self.root = Some(redistribute_containing_run(root, target, axis));
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
        true
    }

    fn edge_candidates(
        &self,
        source: WindowId,
        target: WindowId,
        side: Side,
    ) -> Vec<EdgeCandidate> {
        let Some(root) = self.root.as_ref() else {
            return Vec::new();
        };
        let rects = self.all_float_bounds();
        let leaf_rects = self.float_bounds();
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
            if cross_size(rect, axis) > target_cross_size + tolerance
                && seam > rect.axis_start(axis) + tolerance
                && seam < rect.axis_start(axis) + rect.axis_size(axis) - tolerance
                && let Some(before) =
                    seam_partition(&split.children, seam, axis, &leaf_rects, tolerance)
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
                    let Some(before) = seam_partition(children, seam, axis, &leaf_rects, tolerance)
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
                distinct.push(candidate);
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
        self.normalized_placement_targets(source, layout_rect, edge_fraction)
            .into_iter()
            .map(|(target, _)| target)
            .collect()
    }

    fn normalized_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<(PlacementTarget, PlacementOutcome)> {
        let mut distinct = Vec::<(PlacementTarget, PlacementOutcome)>::new();
        for target in self.raw_placement_targets(source, layout_rect, edge_fraction) {
            let Some(outcome) = self.placement_outcome(source, target) else {
                continue;
            };
            if distinct
                .iter()
                .any(|(_, existing)| existing.approximately_eq(&outcome))
            {
                continue;
            }
            distinct.push((target, outcome));
        }
        distinct
    }

    fn raw_placement_targets(
        &self,
        source: WindowId,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> Vec<PlacementTarget> {
        let bounds = self.bounds(layout_rect);
        let fraction = finite_clamp(edge_fraction, 0.05, 0.49, 0.34);
        let mut output = Vec::new();
        for target in self.leaves().into_iter().filter(|window| *window != source) {
            let Some(rect) = bounds.get(&target).copied() else {
                continue;
            };
            output.push(PlacementTarget {
                target,
                side: None,
                candidate_index: 0,
                position: rect.center(),
            });
            for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
                let candidates = self.edge_candidates(source, target, side);
                for index in 0..candidates.len() {
                    let band_fraction = fraction * (index as f64 + 0.5) / candidates.len() as f64;
                    let position = match side {
                        Side::Left => Point::new(
                            rect.x + (f64::from(rect.w) * band_fraction).round() as i32,
                            rect.center().y,
                        ),
                        Side::Right => Point::new(
                            rect.x + rect.w - (f64::from(rect.w) * band_fraction).round() as i32,
                            rect.center().y,
                        ),
                        Side::Top => Point::new(
                            rect.center().x,
                            rect.y + (f64::from(rect.h) * band_fraction).round() as i32,
                        ),
                        Side::Bottom => Point::new(
                            rect.center().x,
                            rect.y + rect.h - (f64::from(rect.h) * band_fraction).round() as i32,
                        ),
                    };
                    output.push(PlacementTarget {
                        target,
                        side: Some(side),
                        candidate_index: index,
                        position,
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
        let mut rects = preview.float_bounds().into_iter().collect::<Vec<_>>();
        rects.sort_by_key(|(window, _)| *window);
        Some(PlacementOutcome {
            leaves: preview.leaves(),
            rects,
        })
    }

    fn canonical_placement_target(
        &self,
        source: WindowId,
        target: PlacementTarget,
        layout_rect: Rect,
        edge_fraction: f64,
    ) -> PlacementTarget {
        let Some(outcome) = self.placement_outcome(source, target) else {
            return target;
        };
        self.normalized_placement_targets(source, layout_rect, edge_fraction)
            .into_iter()
            .find(|(_, candidate_outcome)| candidate_outcome.approximately_eq(&outcome))
            .map_or(target, |(candidate, _)| candidate)
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
            (Side::Right, f64::from(rect.x + rect.w - point.x) / inset_x),
            (Side::Top, f64::from(point.y - rect.y) / inset_y),
            (Side::Bottom, f64::from(rect.y + rect.h - point.y) / inset_y),
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
                let target =
                    self.canonical_placement_target(source, target, layout_rect, edge_fraction);
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
        true
    }

    /// Swap a window into the first visual leaf. If it is already first, swap
    /// the second leaf in instead so repeated promotion remains useful.
    pub fn promote(&mut self, window: WindowId) -> bool {
        let leaves = self.leaves();
        let Some(target) = (if leaves.first() == Some(&window) {
            leaves.get(1)
        } else {
            leaves.first()
        })
        .copied() else {
            return false;
        };
        self.swap_windows(window, target)
    }
}

#[cfg(test)]
mod tests;
