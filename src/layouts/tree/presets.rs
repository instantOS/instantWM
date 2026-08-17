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

/// Build a master-area-over-stack-area split.
///
/// The two areas are divided along `outer_axis`; their members run along the
/// orthogonal axis (a master column beside a stacked column for tile, a
/// master row over a side-by-side row for bottom-stack).
pub(super) fn build_master_stack(
    windows: &[WindowId],
    requested_master_count: usize,
    master_ratio: f64,
    outer_axis: Axis,
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    let inner_axis = outer_axis.other();
    if windows.len() <= 1 {
        return windows.first().copied().map(Node::Window);
    }
    let master_count = requested_master_count.min(windows.len());
    if master_count == 0 || master_count == windows.len() {
        return equal_run(windows, inner_axis, allocate);
    }
    let masters = equal_run(&windows[..master_count], inner_axis, allocate)?;
    let stack = equal_run(&windows[master_count..], inner_axis, allocate)?;
    let id = allocate();
    make_split(
        id,
        outer_axis,
        vec![
            WeightedNode {
                node: masters,
                weight: master_ratio.clamp(0.05, 0.95),
            },
            WeightedNode {
                node: stack,
                weight: 1.0 - master_ratio.clamp(0.05, 0.95),
            },
        ],
    )
}

/// Build a columns-first grid: `ceil(sqrt(n))` equal columns, each a
/// top-to-bottom run, with the final column soaking up the remainder.
pub(super) fn build_grid(
    windows: &[WindowId],
    allocate: &mut impl FnMut() -> SplitId,
) -> Option<Node> {
    if windows.len() <= 1 {
        return windows.first().copied().map(Node::Window);
    }
    let columns = (windows.len() as f64).sqrt().ceil() as usize;
    let rows = windows.len().div_ceil(columns);
    let mut groups = Vec::new();
    for group in 0..columns {
        let members: Vec<_> = windows
            .iter()
            .skip(group * rows)
            .take(rows)
            .copied()
            .collect();
        if let Some(node) = equal_run(&members, Axis::Horizontal, allocate) {
            groups.push(WeightedNode { node, weight: 1.0 });
        }
    }
    let id = allocate();
    make_split(id, Axis::Vertical, groups)
}
