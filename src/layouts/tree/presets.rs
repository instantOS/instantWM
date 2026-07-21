use super::*;

pub(super) fn equal_run(
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

pub(super) fn build_master_stack(
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

pub(super) fn build_grid(
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

pub(super) fn build_focus(
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
