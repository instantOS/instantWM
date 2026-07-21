//! Backend-independent manual tiling tree.
//!
//! A tree belongs to one monitor/tag-mask pair.  Splits are weighted n-ary
//! runs.  Construction is deliberately private and canonicalises adjacent
//! splits on the same axis, so the rest of the window manager cannot create a
//! one-child split, a non-positive weight, or redundant same-axis nesting.

use std::collections::{HashMap, HashSet};

use crate::types::{Point, Rect, WindowId};

const EPSILON: f64 = 1.0e-9;
const DEFAULT_RESIZE_STEP: f64 = 0.05;
const DEFAULT_MINIMUM_WEIGHT: f64 = 0.15;

#[derive(Debug, Clone, Copy)]
pub struct CommandConfig {
    pub resize_step: f64,
    pub minimum_weight: f64,
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            resize_step: DEFAULT_RESIZE_STEP,
            minimum_weight: DEFAULT_MINIMUM_WEIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Divide a rectangle into left-to-right children.
    Vertical,
    /// Divide a rectangle into top-to-bottom children.
    Horizontal,
}

impl Axis {
    pub const fn other(self) -> Self {
        match self {
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Vertical,
            Self::Top | Self::Bottom => Axis::Horizontal,
        }
    }

    pub const fn is_leading(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
}

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

/// Opaque semantic target used by keyboard placement. Pointer and keyboard
/// paths resolve through the same edge-candidate generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementTarget {
    pub target: WindowId,
    pub side: Option<Side>,
    pub candidate_index: usize,
    pub position: Point,
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

/// One-shot transformations replacing the old continuously active algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    MasterStack,
    Grid,
    HorizontalGrid,
    BottomStack,
    BottomStackHorizontal,
    /// Preserve every leaf while giving the selected one a dominant slot.
    Focus,
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

    pub fn placement_targets(
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
                self.move_to_scope(source, target, side, candidates[index].scope.clone())
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

fn equal_run(
    windows: &[WindowId],
    axis: Axis,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    match windows {
        [] => None,
        [window] => Some(Node::Window(*window)),
        _ => {
            let id = allocate();
            make_split(
                id,
                axis,
                windows
                    .iter()
                    .map(|window| WeightedNode {
                        node: Node::Window(*window),
                        weight: 1.0,
                    })
                    .collect(),
            )
        }
    }
}

fn build_master_stack(
    windows: &[WindowId],
    requested_master_count: usize,
    master_factor: f64,
    outer_axis: Axis,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    if windows.len() <= 1 {
        return windows.first().copied().map(Node::Window);
    }
    let master_count = requested_master_count.max(1).min(windows.len() - 1);
    let masters = equal_run(&windows[..master_count], outer_axis.other(), allocate)?;
    let stack = equal_run(&windows[master_count..], outer_axis.other(), allocate)?;
    let id = allocate();
    make_split(
        id,
        outer_axis,
        vec![
            WeightedNode {
                node: masters,
                weight: master_factor.clamp(0.05, 0.95),
            },
            WeightedNode {
                node: stack,
                weight: 1.0 - master_factor.clamp(0.05, 0.95),
            },
        ],
    )
}

fn build_grid(
    windows: &[WindowId],
    rows_first: bool,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    if windows.len() <= 1 {
        return windows.first().copied().map(Node::Window);
    }
    let columns = (windows.len() as f64).sqrt().ceil() as usize;
    let rows = windows.len().div_ceil(columns);
    let (outer_axis, group_count) = if rows_first {
        (Axis::Horizontal, rows)
    } else {
        (Axis::Vertical, columns)
    };
    let mut groups = Vec::new();
    for group in 0..group_count {
        let members: Vec<_> = if rows_first {
            windows
                .iter()
                .skip(group * columns)
                .take(columns)
                .copied()
                .collect()
        } else {
            windows
                .iter()
                .skip(group)
                .step_by(columns)
                .copied()
                .collect()
        };
        if let Some(node) = equal_run(&members, outer_axis.other(), allocate) {
            groups.push(WeightedNode { node, weight: 1.0 });
        }
    }
    let id = allocate();
    make_split(id, outer_axis, groups)
}

fn build_focus(
    windows: &[WindowId],
    selected: Option<WindowId>,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    let focused = selected
        .filter(|window| windows.contains(window))
        .unwrap_or(windows[0]);
    let peers: Vec<_> = windows
        .iter()
        .copied()
        .filter(|window| *window != focused)
        .collect();
    if peers.is_empty() {
        return Some(Node::Window(focused));
    }
    let peer_run = equal_run(&peers, Axis::Horizontal, allocate)?;
    let id = allocate();
    make_split(
        id,
        Axis::Vertical,
        vec![
            WeightedNode {
                node: Node::Window(focused),
                weight: 0.85,
            },
            WeightedNode {
                node: peer_run,
                weight: 0.15,
            },
        ],
    )
}

fn swap_windows(node: Node, first: WindowId, second: WindowId) -> Node {
    match node {
        Node::Window(window) if window == first => Node::Window(second),
        Node::Window(window) if window == second => Node::Window(first),
        Node::Window(window) => Node::Window(window),
        Node::Split(mut split) => {
            for child in &mut split.children {
                child.node = swap_windows(child.node.clone(), first, second);
            }
            Node::Split(split)
        }
    }
}

fn clone_node_by_key(node: &Node, key: NodeKey) -> Option<Node> {
    if node.key() == key {
        return Some(node.clone());
    }
    let Node::Split(split) = node else {
        return None;
    };
    split
        .children
        .iter()
        .find_map(|child| clone_node_by_key(&child.node, key))
}

fn bounding_rect(rects: &[FRect]) -> Option<FRect> {
    let first = *rects.first()?;
    let (mut left, mut top, mut right, mut bottom) =
        (first.x, first.y, first.x + first.w, first.y + first.h);
    for rect in &rects[1..] {
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.x + rect.w);
        bottom = bottom.max(rect.y + rect.h);
    }
    Some(FRect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

fn cross_size(rect: FRect, axis: Axis) -> f64 {
    match axis {
        Axis::Vertical => rect.h,
        Axis::Horizontal => rect.w,
    }
}

fn seam_partition(
    children: &[WeightedNode],
    seam: f64,
    axis: Axis,
    rects: &HashMap<WindowId, FRect>,
    tolerance: f64,
) -> Option<Vec<WindowId>> {
    let mut before = Vec::new();
    let mut after = false;
    for child in children {
        let mut leaves = Vec::new();
        child.node.leaves(&mut leaves);
        for window in leaves {
            let rect = rects.get(&window).copied()?;
            let start = rect.axis_start(axis);
            let end = start + rect.axis_size(axis);
            if end <= seam + tolerance {
                before.push(window);
            } else if start >= seam - tolerance {
                after = true;
            } else {
                return None;
            }
        }
    }
    (!before.is_empty() && after).then_some(before)
}

fn path_to_key<'a>(node: &'a Node, target: NodeKey, path: &mut Vec<&'a Split>) -> bool {
    if node.key() == target {
        return true;
    }
    let Node::Split(split) = node else {
        return false;
    };
    for child in &split.children {
        if child.node.contains_key(target) {
            path.push(split);
            return path_to_key(&child.node, target, path);
        }
    }
    false
}

fn insert_at_scope_edge(
    root: Node,
    scope_key: NodeKey,
    target: WindowId,
    source: WindowId,
    side: Side,
    new_id: SplitId,
) -> Option<Node> {
    let scope = clone_node_by_key(&root, scope_key)?;
    let axis = side.axis();
    let mut path = Vec::new();
    path_to_key(&root, scope_key, &mut path);
    let run_key = if matches!(&scope, Node::Split(split) if split.axis == axis) {
        scope_key
    } else if let Some(parent) = path.last().filter(|parent| parent.axis == axis) {
        NodeKey::Split(parent.id)
    } else {
        scope_key
    };
    let run = clone_node_by_key(&root, run_key)?;
    if let Node::Split(split) = &run
        && split.axis == axis
        && split.children.len() > 1
        && let Some(target_index) = split
            .children
            .iter()
            .position(|child| child.node.contains(target))
    {
        let count = split.children.len() as f64;
        let mut children = split
            .children
            .iter()
            .cloned()
            .map(|mut child| {
                child.weight *= count;
                child
            })
            .collect::<Vec<_>>();
        children.insert(
            target_index + usize::from(!side.is_leading()),
            WeightedNode {
                node: Node::Window(source),
                weight: 1.0,
            },
        );
        let rebuilt = make_split(split.id, axis, children)?;
        return Some(root.replace_key(run_key, rebuilt));
    }

    let children = if side.is_leading() {
        vec![
            WeightedNode {
                node: Node::Window(source),
                weight: 1.0,
            },
            WeightedNode {
                node: scope,
                weight: 1.0,
            },
        ]
    } else {
        vec![
            WeightedNode {
                node: scope,
                weight: 1.0,
            },
            WeightedNode {
                node: Node::Window(source),
                weight: 1.0,
            },
        ]
    };
    Some(root.replace_key(scope_key, make_split(new_id, axis, children)?))
}

fn filtered_node(
    node: &Node,
    before: &[WindowId],
    keep_before: bool,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    match node {
        Node::Window(window) => {
            (before.contains(window) == keep_before).then_some(Node::Window(*window))
        }
        Node::Split(split) => {
            let children = split
                .children
                .iter()
                .filter_map(|child| {
                    filtered_node(&child.node, before, keep_before, allocate).map(|node| {
                        WeightedNode {
                            node,
                            weight: child.weight,
                        }
                    })
                })
                .collect();
            make_split(allocate(), split.axis, children)
        }
    }
}

struct AlignedInsertion<'a> {
    seam: f64,
    before: &'a [WindowId],
    source: WindowId,
    axis: Axis,
}

fn aligned_replacement(
    scope: &Node,
    scope_rect: FRect,
    insertion: &AlignedInsertion<'_>,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    let before_node = filtered_node(scope, insertion.before, true, allocate)?;
    let after_node = filtered_node(scope, insertion.before, false, allocate)?;
    let ratio = ((insertion.seam - scope_rect.axis_start(insertion.axis))
        / scope_rect.axis_size(insertion.axis))
    .clamp(0.05, 0.95);
    make_split(
        allocate(),
        insertion.axis,
        vec![
            WeightedNode {
                node: before_node,
                weight: ratio * 2.0,
            },
            WeightedNode {
                node: Node::Window(insertion.source),
                weight: 1.0,
            },
            WeightedNode {
                node: after_node,
                weight: (1.0 - ratio) * 2.0,
            },
        ],
    )
}

fn unit_bounds(node: &Node) -> HashMap<NodeKey, FRect> {
    let mut rects = HashMap::new();
    node.all_bounds(
        FRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
        &mut rects,
    );
    rects
}

fn insert_across_aligned_node(
    root: Node,
    scope_key: NodeKey,
    insertion: &AlignedInsertion<'_>,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    let scope = clone_node_by_key(&root, scope_key)?;
    let rects = unit_bounds(&root);
    let replacement = aligned_replacement(&scope, *rects.get(&scope_key)?, insertion, allocate)?;
    Some(root.replace_key(scope_key, replacement))
}

fn insert_across_aligned_range(
    root: Node,
    parent_id: SplitId,
    child_keys: &[NodeKey],
    insertion: &AlignedInsertion<'_>,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    let parent_key = NodeKey::Split(parent_id);
    let Node::Split(mut parent) = clone_node_by_key(&root, parent_key)? else {
        return None;
    };
    let selected = parent
        .children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| child_keys.contains(&child.node.key()).then_some(index))
        .collect::<Vec<_>>();
    let first = *selected.first()?;
    let last = *selected.last()?;
    if selected.len() != last - first + 1 {
        return None;
    }
    let range = parent.children[first..=last].to_vec();
    let range_weight = range.iter().map(|child| child.weight).sum();
    let scope = make_split(allocate(), parent.axis, range)?;
    let rects = unit_bounds(&root);
    let selected_rects = parent.children[first..=last]
        .iter()
        .filter_map(|child| rects.get(&child.node.key()).copied())
        .collect::<Vec<_>>();
    let replacement =
        aligned_replacement(&scope, bounding_rect(&selected_rects)?, insertion, allocate)?;
    parent.children.splice(
        first..=last,
        [WeightedNode {
            node: replacement,
            weight: range_weight,
        }],
    );
    Some(root.replace_key(
        parent_key,
        make_split(parent.id, parent.axis, parent.children)?,
    ))
}

fn immediate_parent_axis(node: &Node, target: WindowId) -> Option<Axis> {
    let Node::Split(split) = node else {
        return None;
    };
    for child in &split.children {
        if matches!(child.node, Node::Window(window) if window == target) {
            return Some(split.axis);
        }
        if child.node.contains(target) {
            return immediate_parent_axis(&child.node, target).or(Some(split.axis));
        }
    }
    None
}

fn resize_deepest_run(
    node: Node,
    target: WindowId,
    axis: Axis,
    grow: bool,
    config: CommandConfig,
) -> (Node, bool) {
    let Node::Split(mut split) = node else {
        return (node, false);
    };
    let Some(index) = split
        .children
        .iter()
        .position(|child| child.node.contains(target))
    else {
        return (Node::Split(split), false);
    };

    let child_node = split.children[index].node.clone();
    let (resized_child, changed) = resize_deepest_run(child_node, target, axis, grow, config);
    split.children[index].node = resized_child;
    if changed {
        return (Node::Split(split), true);
    }
    if split.axis != axis || split.children.len() < 2 {
        return (Node::Split(split), false);
    }

    let count = split.children.len();
    let minimum = finite_clamp(config.minimum_weight, 0.001, 0.49, DEFAULT_MINIMUM_WEIGHT)
        .min(0.5 / count as f64);
    let maximum = 1.0 - minimum * (count - 1) as f64;
    let current = split.children[index].weight;
    let step = finite_clamp(config.resize_step, 0.001, 0.5, DEFAULT_RESIZE_STEP);
    let requested = current + if grow { step } else { -step };
    let target_weight = requested.clamp(minimum, maximum);
    if (target_weight - current).abs() < EPSILON {
        return (Node::Split(split), false);
    }
    let peer_scale = (1.0 - target_weight) / (1.0 - current);
    for (child_index, child) in split.children.iter_mut().enumerate() {
        child.weight = if child_index == index {
            target_weight
        } else {
            child.weight * peer_scale
        };
    }
    (Node::Split(split), true)
}

fn redistribute_containing_run(node: Node, target: WindowId, axis: Axis) -> Node {
    let Node::Split(mut split) = node else {
        return node;
    };
    let Some(index) = split
        .children
        .iter()
        .position(|child| child.node.contains(target))
    else {
        return Node::Split(split);
    };
    if split.axis == axis {
        let total: usize = split
            .children
            .iter()
            .map(|child| child.node.leaf_count())
            .sum();
        for child in &mut split.children {
            child.weight = child.node.leaf_count() as f64 / total as f64;
        }
        Node::Split(split)
    } else {
        let child = split.children[index].node.clone();
        split.children[index].node = redistribute_containing_run(child, target, axis);
        Node::Split(split)
    }
}

fn visual_neighbor_in(
    node: &Node,
    source: WindowId,
    source_rect: FRect,
    side: Side,
    rects: &HashMap<WindowId, FRect>,
) -> (Option<WindowId>, bool) {
    let mut path = Vec::new();
    if !path_to(node, source, &mut path) {
        return (None, false);
    }
    let direction = side.axis();
    for (parent, branch_index) in path.into_iter().rev() {
        if parent.axis != direction {
            continue;
        }
        let sibling_index = if side.is_leading() {
            branch_index.checked_sub(1)
        } else {
            branch_index
                .checked_add(1)
                .filter(|index| *index < parent.children.len())
        };
        let Some(sibling) = sibling_index.and_then(|index| parent.children.get(index)) else {
            continue;
        };
        let mut leaves = Vec::new();
        sibling.node.leaves(&mut leaves);
        let neighbor = leaves
            .into_iter()
            .filter_map(|window| {
                let rect = *rects.get(&window)?;
                let overlap = shared_border_overlap(source_rect, rect, side);
                (overlap > EPSILON).then_some((window, overlap, rect))
            })
            .max_by(|left, right| {
                left.1.total_cmp(&right.1).then_with(|| {
                    let source_center = cross_center(source_rect, direction);
                    let left_distance = (cross_center(left.2, direction) - source_center).abs();
                    let right_distance = (cross_center(right.2, direction) - source_center).abs();
                    right_distance.total_cmp(&left_distance)
                })
            })
            .map(|candidate| candidate.0);
        return (neighbor, false);
    }
    (None, true)
}

fn path_to<'a>(node: &'a Node, source: WindowId, path: &mut Vec<(&'a Split, usize)>) -> bool {
    let Node::Split(split) = node else {
        return matches!(node, Node::Window(window) if *window == source);
    };
    for (index, child) in split.children.iter().enumerate() {
        if child.node.contains(source) {
            path.push((split, index));
            return path_to(&child.node, source, path);
        }
    }
    false
}

fn cross_center(rect: FRect, axis: Axis) -> f64 {
    match axis {
        Axis::Vertical => rect.y + rect.h / 2.0,
        Axis::Horizontal => rect.x + rect.w / 2.0,
    }
}

fn shared_border_overlap(source: FRect, candidate: FRect, side: Side) -> f64 {
    let axis = side.axis();
    let source_edge = source.axis_start(axis)
        + if side.is_leading() {
            0.0
        } else {
            source.axis_size(axis)
        };
    let candidate_edge = candidate.axis_start(axis)
        + if side.is_leading() {
            candidate.axis_size(axis)
        } else {
            0.0
        };
    if (source_edge - candidate_edge).abs() > 1.0e-6 {
        return 0.0;
    }
    let (source_start, source_end, candidate_start, candidate_end) = match axis {
        Axis::Vertical => (
            source.y,
            source.y + source.h,
            candidate.y,
            candidate.y + candidate.h,
        ),
        Axis::Horizontal => (
            source.x,
            source.x + source.w,
            candidate.x,
            candidate.x + candidate.w,
        ),
    };
    source_end.min(candidate_end) - source_start.max(candidate_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(count: u32) -> Vec<WindowId> {
        (1..=count).map(WindowId).collect()
    }

    fn assert_canonical(tree: &LayoutTree) {
        fn visit(node: &Node, leaves: &mut HashSet<WindowId>, splits: &mut HashSet<SplitId>) {
            match node {
                Node::Window(window) => assert!(leaves.insert(*window), "duplicate leaf"),
                Node::Split(split) => {
                    assert!(splits.insert(split.id), "duplicate split id");
                    assert!(split.children.len() >= 2);
                    assert!(split.children.iter().all(|child| {
                        child.weight.is_finite()
                            && child.weight > 0.0
                            && !matches!(&child.node, Node::Split(nested) if nested.axis == split.axis)
                    }));
                    assert!(
                        (split.children.iter().map(|child| child.weight).sum::<f64>() - 1.0).abs()
                            < EPSILON
                    );
                    for child in &split.children {
                        visit(&child.node, leaves, splits);
                    }
                }
            }
        }

        if let Some(root) = &tree.root {
            visit(root, &mut HashSet::new(), &mut HashSet::new());
        }
    }

    #[test]
    fn reconciliation_retains_surviving_topology_and_collapses_parents() {
        let mut tree = LayoutTree::default();
        tree.reconcile(&windows(4));
        let before = tree.bounds(Rect::new(0, 0, 100, 100));
        assert_eq!(tree.len(), 4);
        tree.reconcile(&[WindowId(1), WindowId(3), WindowId(4)]);
        assert_eq!(tree.len(), 3);
        assert!(!tree.leaves().contains(&WindowId(2)));
        assert!(before.contains_key(&WindowId(1)));
    }

    #[test]
    fn same_axis_insertions_form_one_n_ary_run() {
        let mut tree = LayoutTree::default();
        tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
        let Node::Split(root) = tree.root.as_ref().unwrap() else {
            panic!("expected split");
        };
        assert_eq!(root.children.len(), 3);
        assert!(
            root.children
                .iter()
                .all(|child| (child.weight - 1.0 / 3.0).abs() < EPSILON)
        );
    }

    #[test]
    fn grid_is_a_persistent_tree_transformation() {
        let mut tree = LayoutTree::default();
        let wins = windows(4);
        tree.apply_preset(Preset::Grid, &wins, None, 1, 0.55);
        let rects = tree.bounds(Rect::new(0, 0, 100, 100));
        assert_eq!(rects[&WindowId(1)], Rect::new(0, 0, 50, 50));
        assert_eq!(rects[&WindowId(4)], Rect::new(50, 50, 50, 50));
        assert!(tree.resize(WindowId(1), Side::Right));
        assert_ne!(tree.bounds(Rect::new(0, 0, 100, 100)), rects);
    }

    #[test]
    fn traversal_uses_first_structural_seam() {
        let mut tree = LayoutTree::default();
        let wins = windows(4);
        tree.apply_preset(Preset::Grid, &wins, None, 1, 0.55);
        assert_eq!(
            tree.visual_neighbor(WindowId(4), Side::Left),
            Some(WindowId(2))
        );
        assert_eq!(
            tree.visual_neighbor(WindowId(4), Side::Top),
            Some(WindowId(3))
        );
    }

    #[test]
    fn resize_preserves_peer_ratios() {
        let mut tree = LayoutTree::default();
        tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
        assert!(tree.resize(WindowId(2), Side::Right));
        let rects = tree.bounds(Rect::new(0, 0, 300, 100));
        assert!(rects[&WindowId(2)].w > 100);
        assert_eq!(rects[&WindowId(1)].w, rects[&WindowId(3)].w);
    }

    #[test]
    fn resize_stops_at_the_local_run_across_an_orthogonal_parent() {
        let mut tree = LayoutTree::default();
        let local_id = tree.allocate();
        let row_id = tree.allocate();
        let root_id = tree.allocate();
        let local = make_split(
            local_id,
            Axis::Vertical,
            windows(2)
                .into_iter()
                .map(|window| WeightedNode {
                    node: Node::Window(window),
                    weight: 1.0,
                })
                .collect(),
        )
        .unwrap();
        let row = make_split(
            row_id,
            Axis::Horizontal,
            vec![
                WeightedNode {
                    node: local,
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(WindowId(3)),
                    weight: 1.0,
                },
            ],
        )
        .unwrap();
        tree.root = make_split(
            root_id,
            Axis::Vertical,
            vec![
                WeightedNode {
                    node: Node::Window(WindowId(4)),
                    weight: 1.0,
                },
                WeightedNode {
                    node: row,
                    weight: 1.0,
                },
            ],
        );
        let before = tree.bounds(Rect::new(0, 0, 100, 100));
        assert!(tree.resize(WindowId(1), Side::Right));
        let after = tree.bounds(Rect::new(0, 0, 100, 100));
        assert_eq!(after[&WindowId(4)].w, before[&WindowId(4)].w);
        assert!(after[&WindowId(1)].w > before[&WindowId(1)].w);
        assert!(after[&WindowId(2)].w < before[&WindowId(2)].w);
    }

    #[test]
    fn repeated_swaps_walk_visual_neighbors_without_changing_topology() {
        let mut tree = LayoutTree::default();
        tree.root = equal_run(&windows(4), Axis::Horizontal, &mut || tree.allocate());
        for neighbor in [WindowId(3), WindowId(2), WindowId(1)] {
            assert_eq!(
                tree.swap_with_neighbor(WindowId(4), Side::Top),
                Some(neighbor)
            );
            assert_canonical(&tree);
        }
        assert_eq!(
            tree.leaves(),
            vec![WindowId(4), WindowId(1), WindowId(2), WindowId(3)]
        );
        assert_eq!(tree.swap_with_neighbor(WindowId(4), Side::Top), None);
    }

    #[test]
    fn t_junction_swap_exchanges_complete_visual_slots() {
        let mut tree = LayoutTree::default();
        let left_id = tree.allocate();
        let root_id = tree.allocate();
        let left = make_split(
            left_id,
            Axis::Horizontal,
            vec![
                WeightedNode {
                    node: Node::Window(WindowId(1)),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(WindowId(2)),
                    weight: 1.0,
                },
            ],
        )
        .unwrap();
        tree.root = make_split(
            root_id,
            Axis::Vertical,
            vec![
                WeightedNode {
                    node: left,
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(WindowId(3)),
                    weight: 1.0,
                },
            ],
        );
        assert_eq!(
            tree.swap_with_neighbor(WindowId(1), Side::Right),
            Some(WindowId(3))
        );
        let rects = tree.bounds(Rect::new(0, 0, 100, 100));
        assert_eq!(rects[&WindowId(1)], Rect::new(50, 0, 50, 100));
        assert_eq!(rects[&WindowId(3)], Rect::new(0, 0, 50, 50));
        assert_canonical(&tree);
    }

    #[test]
    fn pointer_center_swaps_and_edge_reparents() {
        let mut tree = LayoutTree::default();
        tree.root = equal_run(&windows(3), Axis::Vertical, &mut || tree.allocate());
        assert!(tree.place_at_point(
            WindowId(1),
            Point::new(150, 50),
            Rect::new(0, 0, 300, 100),
            0.3
        ));
        assert_eq!(tree.leaves(), vec![WindowId(2), WindowId(1), WindowId(3)]);
        assert!(tree.place_at_point(
            WindowId(1),
            Point::new(201, 50),
            Rect::new(0, 0, 300, 100),
            0.3
        ));
        assert_eq!(tree.len(), 3);
    }

    #[test]
    fn aligned_seam_can_target_a_contiguous_virtual_scope() {
        let mut tree = LayoutTree::default();
        let column_one_id = tree.allocate();
        let column_two_id = tree.allocate();
        let root_id = tree.allocate();
        let column_one = make_split(
            column_one_id,
            Axis::Horizontal,
            vec![
                WeightedNode {
                    node: Node::Window(WindowId(2)),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(WindowId(3)),
                    weight: 1.0,
                },
            ],
        )
        .unwrap();
        let column_two = make_split(
            column_two_id,
            Axis::Horizontal,
            vec![
                WeightedNode {
                    node: Node::Window(WindowId(4)),
                    weight: 1.0,
                },
                WeightedNode {
                    node: Node::Window(WindowId(5)),
                    weight: 1.0,
                },
            ],
        )
        .unwrap();
        tree.root = make_split(
            root_id,
            Axis::Vertical,
            vec![
                WeightedNode {
                    node: Node::Window(WindowId(1)),
                    weight: 0.25,
                },
                WeightedNode {
                    node: column_one,
                    weight: 0.25,
                },
                WeightedNode {
                    node: column_two,
                    weight: 0.25,
                },
                WeightedNode {
                    node: Node::Window(WindowId(6)),
                    weight: 0.25,
                },
            ],
        );

        // Window 6 moves to the seam above window 3. Window 1 crosses that
        // seam, so only the contiguous two-column grid is a valid scope.
        assert!(tree.place_at_point(
            WindowId(6),
            Point::new(26, 51),
            Rect::new(0, 0, 100, 100),
            0.34,
        ));
        let rects = tree.bounds(Rect::new(0, 0, 100, 100));
        // Removing the former right-hand peer normalises the three surviving
        // outer items, so the full-height leaf keeps its ratio to both grid
        // columns (1:1:1) rather than its old absolute percentage.
        assert_eq!(rects[&WindowId(1)], Rect::new(0, 0, 33, 100));
        assert_eq!(rects[&WindowId(6)].x, 33);
        assert_eq!(rects[&WindowId(6)].w, 67);
        // The source occupies the seam itself; the two original rows remain
        // on either side instead of being compressed together below it.
        assert_eq!(rects[&WindowId(6)].y, 33);
        assert_eq!(rects[&WindowId(6)].h, 34);
    }

    #[test]
    fn keyboard_and_pointer_targets_apply_the_same_semantic_candidate() {
        let mut original = LayoutTree::default();
        original.root = equal_run(&windows(3), Axis::Vertical, &mut || original.allocate());
        let rect = Rect::new(0, 0, 300, 100);
        let target = original
            .placement_targets(WindowId(1), rect, 0.34)
            .into_iter()
            .find(|target| target.target == WindowId(2) && target.side == Some(Side::Right))
            .unwrap();

        let mut keyboard = original.clone();
        let mut pointer = original;
        assert!(keyboard.apply_placement_target(WindowId(1), target));
        assert!(pointer.place_at_point(WindowId(1), target.position, rect, 0.34));
        assert_eq!(keyboard.bounds(rect), pointer.bounds(rect));
    }

    #[test]
    fn every_public_mutation_preserves_canonical_invariants() {
        let wins = windows(7);
        for preset in [
            Preset::MasterStack,
            Preset::Grid,
            Preset::HorizontalGrid,
            Preset::BottomStack,
            Preset::BottomStackHorizontal,
            Preset::Focus,
        ] {
            let mut tree = LayoutTree::default();
            tree.apply_preset(preset, &wins, Some(WindowId(4)), 2, 0.6);
            assert_canonical(&tree);
            tree.resize(WindowId(4), Side::Right);
            tree.resize_smart(WindowId(5), false);
            tree.resize_with_config(
                WindowId(3),
                Side::Left,
                CommandConfig {
                    resize_step: f64::NAN,
                    minimum_weight: f64::INFINITY,
                },
            );
            tree.swap_with_neighbor(WindowId(4), Side::Bottom);
            assert_canonical(&tree);
            let target = tree
                .placement_targets(WindowId(7), Rect::new(0, 0, 700, 500), 0.34)
                .into_iter()
                .find(|target| target.side.is_some());
            if let Some(target) = target {
                assert!(tree.apply_placement_target(WindowId(7), target));
            }
            tree.reconcile(&[WindowId(1), WindowId(3), WindowId(4), WindowId(8)]);
            assert_canonical(&tree);
            let leaves = tree.leaves().into_iter().collect::<HashSet<_>>();
            assert_eq!(
                leaves,
                [WindowId(1), WindowId(3), WindowId(4), WindowId(8)]
                    .into_iter()
                    .collect()
            );
        }
    }
}
