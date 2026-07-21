//! Minimum-size-aware tree geometry.

use super::*;

pub(super) fn required_size(node: &Node, minimums: &HashMap<WindowId, Size>) -> Size {
    match node {
        Node::Window(window) => minimums
            .get(window)
            .copied()
            .unwrap_or_else(|| Size::new(1, 1)),
        Node::Split(split) => {
            let children = split
                .children
                .iter()
                .map(|child| required_size(&child.node, minimums));
            match split.axis {
                Axis::Vertical => children.fold(Size::new(0, 1), |total, child| {
                    Size::new(total.w.saturating_add(child.w), total.h.max(child.h))
                }),
                Axis::Horizontal => children.fold(Size::new(1, 0), |total, child| {
                    Size::new(total.w.max(child.w), total.h.saturating_add(child.h))
                }),
            }
        }
    }
}

pub(super) fn constrained_node_bounds(
    node: &Node,
    rect: FRect,
    minimums: &HashMap<WindowId, Size>,
    output: &mut HashMap<WindowId, FRect>,
) -> Option<()> {
    match node {
        Node::Window(window) => {
            output.insert(*window, rect);
            Some(())
        }
        Node::Split(split) => {
            let available = rect.axis_size(split.axis);
            let required = split
                .children
                .iter()
                .map(|child| {
                    let size = required_size(&child.node, minimums);
                    f64::from(match split.axis {
                        Axis::Vertical => size.w,
                        Axis::Horizontal => size.h,
                    })
                })
                .collect::<Vec<_>>();
            let spans = allocate_constrained_spans(&split.children, &required, available)?;
            let mut offset = 0.0;
            for (child, span) in split.children.iter().zip(spans) {
                let child_rect = match split.axis {
                    Axis::Vertical => FRect {
                        x: rect.x + offset,
                        y: rect.y,
                        w: span,
                        h: rect.h,
                    },
                    Axis::Horizontal => FRect {
                        x: rect.x,
                        y: rect.y + offset,
                        w: rect.w,
                        h: span,
                    },
                };
                constrained_node_bounds(&child.node, child_rect, minimums, output)?;
                offset += span;
            }
            Some(())
        }
    }
}

/// Project weighted spans onto lower bounds without changing their order.
/// Clamped children are removed from the weighted pool until every remaining
/// child can receive its proportional share.
fn allocate_constrained_spans(
    children: &[WeightedNode],
    minimums: &[f64],
    available: f64,
) -> Option<Vec<f64>> {
    if children.len() != minimums.len() || minimums.iter().sum::<f64>() > available + EPSILON {
        return None;
    }

    let mut spans = vec![0.0; children.len()];
    let mut free = (0..children.len()).collect::<Vec<_>>();
    let mut remaining = available;
    loop {
        let weight_sum = free
            .iter()
            .map(|&index| children[index].weight)
            .sum::<f64>();
        let Some(clamped) = free.iter().copied().find(|&index| {
            remaining * children[index].weight / weight_sum + EPSILON < minimums[index]
        }) else {
            for index in free {
                spans[index] = remaining * children[index].weight / weight_sum;
            }
            break;
        };
        spans[clamped] = minimums[clamped];
        remaining -= minimums[clamped];
        free.retain(|&index| index != clamped);
        if free.is_empty() {
            break;
        }
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_allocation_clamps_only_children_below_their_minimum() {
        let children = [
            WeightedNode {
                node: Node::Window(WindowId(1)),
                weight: 0.2,
            },
            WeightedNode {
                node: Node::Window(WindowId(2)),
                weight: 0.3,
            },
            WeightedNode {
                node: Node::Window(WindowId(3)),
                weight: 0.5,
            },
        ];
        let spans = allocate_constrained_spans(&children, &[30.0, 10.0, 10.0], 100.0).unwrap();
        assert_eq!(spans, vec![30.0, 26.25, 43.75]);
    }
}
