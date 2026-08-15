use super::*;

fn rectangle_union_area(rects: &[Rect]) -> i64 {
    let mut x_edges: Vec<_> = rects
        .iter()
        .flat_map(|rect| [rect.x, rect.right()])
        .collect();
    x_edges.sort_unstable();
    x_edges.dedup();

    x_edges
        .windows(2)
        .map(|x| {
            let mut intervals: Vec<_> = rects
                .iter()
                .filter(|rect| rect.x < x[1] && rect.right() > x[0])
                .map(|rect| (rect.y, rect.bottom()))
                .collect();
            intervals.sort_unstable();

            let mut covered_y = 0_i64;
            let mut current: Option<(i32, i32)> = None;
            for (start, end) in intervals {
                match current {
                    Some((current_start, current_end)) if start <= current_end => {
                        current = Some((current_start, current_end.max(end)));
                    }
                    Some((current_start, current_end)) => {
                        covered_y += i64::from(current_end) - i64::from(current_start);
                        current = Some((start, end));
                    }
                    None => current = Some((start, end)),
                }
            }
            if let Some((start, end)) = current {
                covered_y += i64::from(end) - i64::from(start);
            }
            (i64::from(x[1]) - i64::from(x[0])) * covered_y
        })
        .sum()
}

/// Determine whether at least 95% of the pixels in `larger_size` but not
/// `smaller_size` are outside the union of all outputs when both surfaces
/// follow the same anchored frame.
pub(super) fn extra_surface_pixels_are_offscreen(
    frame: Rect,
    smaller_size: Size<i32, Logical>,
    larger_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> bool {
    if outputs.is_empty()
        || smaller_size.w > larger_size.w
        || smaller_size.h > larger_size.h
        || smaller_size == larger_size
    {
        return false;
    }

    let smaller_loc = anchored_surface_location(frame, smaller_size, border_width, anchors);
    let larger_loc = anchored_surface_location(frame, larger_size, border_width, anchors);
    let smaller_rect = Rect::new(smaller_loc.x, smaller_loc.y, smaller_size.w, smaller_size.h);
    let larger_rect = Rect::new(larger_loc.x, larger_loc.y, larger_size.w, larger_size.h);
    let visible_intersections = |surface: Rect| {
        outputs
            .iter()
            .filter_map(|output| surface.intersection(output))
            .collect::<Vec<_>>()
    };
    let total_extra = i64::from(larger_size.w) * i64::from(larger_size.h)
        - i64::from(smaller_size.w) * i64::from(smaller_size.h);
    let visible_larger = rectangle_union_area(&visible_intersections(larger_rect));
    let visible_smaller = rectangle_union_area(&visible_intersections(smaller_rect));
    if visible_smaller > visible_larger {
        return false;
    }
    let visible_extra = visible_larger - visible_smaller;

    i128::from(visible_extra) * 100
        <= i128::from(total_extra) * i128::from(MAX_VISIBLE_OFFSCREEN_RESIZE_PERCENT)
}

#[cfg(test)]
pub(super) fn resize_removal_is_offscreen_at_target(
    target: Rect,
    committed_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> bool {
    let target_size = Size::from((target.w.max(1), target.h.max(1)));
    extra_surface_pixels_are_offscreen(
        target,
        target_size,
        committed_size,
        border_width,
        anchors,
        outputs,
    )
}

pub(super) fn resize_growth_is_offscreen_at_start(
    from: Rect,
    target: Rect,
    committed_size: Size<i32, Logical>,
    border_width: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> bool {
    let target_size = Size::from((target.w.max(1), target.h.max(1)));
    extra_surface_pixels_are_offscreen(
        from,
        committed_size,
        target_size,
        border_width,
        anchors,
        outputs,
    )
}

/// Find the first sampled point where at least 95% of a shrink's removed area
/// is outside all outputs. Sending there leaves a small amount of compositor
/// motion to hide clients that commit more than one relayout buffer.
pub(super) fn offscreen_shrink_configure_phase(
    from: Rect,
    target: Rect,
    committed_size: Size<i32, Logical>,
    from_border: i32,
    to_border: i32,
    anchors: SurfaceAnchors,
    outputs: &[Rect],
) -> Option<f64> {
    const PHASE_STEPS: u32 = 100;
    let target_size = Size::from((target.w.max(1), target.h.max(1)));

    // Earlier than the normal phase buys us nothing, and the endpoint leaves
    // no compositor movement to cover the client's relayout.
    (21..PHASE_STEPS).find_map(|step| {
        let phase = f64::from(step) / f64::from(PHASE_STEPS);
        let frame = crate::animation::interpolate_rect(from, target, phase);
        let border = interpolate_borders(from_border, to_border, ease_out_cubic(phase));
        let surface_location = anchored_surface_location(frame, committed_size, border, anchors);
        let target_surface_location =
            anchored_surface_location(target, committed_size, to_border, anchors);
        (surface_location != target_surface_location
            && extra_surface_pixels_are_offscreen(
                frame,
                target_size,
                committed_size,
                border,
                anchors,
                outputs,
            ))
        .then_some(phase)
    })
}
