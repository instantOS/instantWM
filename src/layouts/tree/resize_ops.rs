use super::*;

#[derive(Debug, Clone, Copy)]
enum PeerScope {
    All,
    BeforeTarget,
    AfterTarget,
}

impl PeerScope {
    fn contains(self, candidate: usize, target: usize) -> bool {
        match self {
            Self::All => candidate != target,
            Self::BeforeTarget => candidate < target,
            Self::AfterTarget => candidate > target,
        }
    }
}

/// Resize one child while proportionally scaling the selected peer group.
/// Keeping this policy in one place makes keyboard resizing (`All`) and
/// physical-edge dragging (`BeforeTarget`/`AfterTarget`) differ only in which
/// peers are allowed to yield space.
fn resize_split_child(
    split: &mut Split,
    target: usize,
    delta: f64,
    minimum_weight: f64,
    peers: PeerScope,
) -> bool {
    let peer_count = split
        .children
        .iter()
        .enumerate()
        .filter(|(index, _)| peers.contains(*index, target))
        .count();
    if peer_count == 0 {
        return false;
    }

    let current = split.children[target].weight;
    let selected_peer_weight = split
        .children
        .iter()
        .enumerate()
        .filter(|(index, _)| peers.contains(*index, target))
        .map(|(_, child)| child.weight)
        .sum::<f64>();
    // Canonical splits sum to one. Use that exact invariant for the all-peer
    // path to retain its established rounding behavior.
    let adjustable_weight = match peers {
        PeerScope::All => 1.0,
        PeerScope::BeforeTarget | PeerScope::AfterTarget => current + selected_peer_weight,
    };
    let peer_weight = adjustable_weight - current;
    let configured_minimum = finite_clamp(minimum_weight, 0.001, 0.49, DEFAULT_MINIMUM_WEIGHT);
    let minimum = match peers {
        // Preserve the keyboard command's existing range: it always leaves at
        // least half the run available to its peers collectively.
        PeerScope::All => configured_minimum.min(0.5 / split.children.len() as f64),
        // A pointer edge can only consume the weight on that edge's side.
        PeerScope::BeforeTarget | PeerScope::AfterTarget => {
            configured_minimum.min(adjustable_weight / (peer_count + 1) as f64)
        }
    };
    let maximum = adjustable_weight - minimum * peer_count as f64;
    let requested = current + if delta.is_finite() { delta } else { 0.0 };
    let target_weight = requested.clamp(minimum, maximum);
    if (target_weight - current).abs() < EPSILON || peer_weight <= EPSILON {
        return false;
    }

    let peer_scale = (adjustable_weight - target_weight) / peer_weight;
    for (index, child) in split.children.iter_mut().enumerate() {
        if index == target {
            child.weight = target_weight;
        } else if peers.contains(index, target) {
            child.weight *= peer_scale;
        }
    }
    true
}

pub(super) fn immediate_parent_axis(node: &Node, target: WindowId) -> Option<Axis> {
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

pub(super) fn resize_deepest_run(
    node: Node,
    target: WindowId,
    axis: Axis,
    grow: bool,
    config: CommandConfig,
) -> (Node, bool) {
    let step = finite_clamp(config.resize_step, 0.001, 0.5, DEFAULT_RESIZE_STEP);
    let delta = if grow { step } else { -step };
    resize_deepest_run_by(node, target, axis, delta, config.minimum_weight)
}

pub(super) fn resize_deepest_run_by(
    node: Node,
    target: WindowId,
    axis: Axis,
    delta: f64,
    minimum_weight: f64,
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
    let (resized_child, changed) =
        resize_deepest_run_by(child_node, target, axis, delta, minimum_weight);
    split.children[index].node = resized_child;
    if changed {
        return (Node::Split(split), true);
    }
    if split.axis != axis || split.children.len() < 2 {
        return (Node::Split(split), false);
    }

    let changed = resize_split_child(&mut split, index, delta, minimum_weight, PeerScope::All);
    (Node::Split(split), changed)
}

pub(super) fn deepest_resize_split(node: &Node, target: WindowId, axis: Axis) -> Option<SplitId> {
    let Node::Split(split) = node else {
        return None;
    };
    let child = split
        .children
        .iter()
        .find(|child| child.node.contains(target))?;
    deepest_resize_split(&child.node, target, axis)
        .or_else(|| (split.axis == axis && split.children.len() >= 2).then_some(split.id))
}

pub(super) fn deepest_resize_edge_split(
    node: &Node,
    target: WindowId,
    side: Side,
) -> Option<SplitId> {
    let Node::Split(split) = node else {
        return None;
    };
    let index = split
        .children
        .iter()
        .position(|child| child.node.contains(target))?;
    deepest_resize_edge_split(&split.children[index].node, target, side).or_else(|| {
        let has_neighbor = if side.is_leading() {
            index > 0
        } else {
            index + 1 < split.children.len()
        };
        (split.axis == side.axis() && has_neighbor).then_some(split.id)
    })
}

pub(super) fn resize_deepest_edge_by(
    node: Node,
    target: WindowId,
    side: Side,
    delta: f64,
    minimum_weight: f64,
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

    let child = split.children[index].node.clone();
    let (child, changed) = resize_deepest_edge_by(child, target, side, delta, minimum_weight);
    split.children[index].node = child;
    if changed {
        return (Node::Split(split), true);
    }

    if split.axis != side.axis() {
        return (Node::Split(split), false);
    }
    let peers = if side.is_leading() {
        PeerScope::BeforeTarget
    } else {
        PeerScope::AfterTarget
    };
    let changed = resize_split_child(&mut split, index, delta, minimum_weight, peers);
    (Node::Split(split), changed)
}

pub(super) fn redistribute_containing_run(node: Node, target: WindowId, axis: Axis) -> Node {
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

pub(super) fn visual_neighbor_in(
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

pub(super) fn path_to<'a>(
    node: &'a Node,
    source: WindowId,
    path: &mut Vec<(&'a Split, usize)>,
) -> bool {
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

pub(super) fn cross_center(rect: FRect, axis: Axis) -> f64 {
    match axis {
        Axis::Vertical => rect.y + rect.h / 2.0,
        Axis::Horizontal => rect.x + rect.w / 2.0,
    }
}

pub(super) fn shared_border_overlap(source: FRect, candidate: FRect, side: Side) -> f64 {
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
