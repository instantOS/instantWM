use super::*;

pub(super) fn swap_windows(node: Node, first: WindowId, second: WindowId) -> Node {
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

pub(super) fn clone_node_by_key(node: &Node, key: NodeKey) -> Option<Node> {
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

pub(super) fn bounding_rect(rects: &[FRect]) -> Option<FRect> {
    let first = *rects.first()?;
    let (mut left, mut top, mut right, mut bottom) =
        (first.x, first.y, first.right(), first.bottom());
    for rect in &rects[1..] {
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.right());
        bottom = bottom.max(rect.bottom());
    }
    Some(FRect {
        x: left,
        y: top,
        w: right - left,
        h: bottom - top,
    })
}

pub(super) fn cross_size(rect: FRect, axis: Axis) -> f64 {
    match axis {
        Axis::Vertical => rect.h,
        Axis::Horizontal => rect.w,
    }
}

pub(super) fn seam_partition(
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

pub(super) fn path_to_key<'a>(node: &'a Node, target: NodeKey, path: &mut Vec<&'a Split>) -> bool {
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

pub(super) fn insert_at_scope_edge(
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

pub(super) fn filtered_node(
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

pub(super) struct AlignedInsertion<'a> {
    pub(super) seam: f64,
    pub(super) before: &'a [WindowId],
    pub(super) source: WindowId,
    pub(super) axis: Axis,
}

pub(super) fn aligned_replacement(
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

pub(super) fn unit_bounds(node: &Node) -> HashMap<NodeKey, FRect> {
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

pub(super) fn insert_across_aligned_node(
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

pub(super) fn insert_across_aligned_range(
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
