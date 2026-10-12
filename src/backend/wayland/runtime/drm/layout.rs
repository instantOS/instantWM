//! Output geometry, automatic placement, and pointer recovery.

use crate::backend::output::{OutputPlacement, plan_automatic_output_positions};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::render::drm::{OutputSurfaceEntry, ScanoutId};

#[derive(Debug)]
pub(super) struct DrmLayoutState {
    pub(super) layout: crate::types::Rect,
    pub(super) output_hit_regions: Vec<OutputHitRegion>,
}

/// Render-scheduling geometry for one scanout. This deliberately lives in the
/// DRM runtime rather than the renderer: deciding which CRTC needs a frame is
/// event-loop policy, and the renderer already owns the authoritative output
/// geometry through [`OutputSurfaceEntry`].
#[derive(Debug, Clone, Copy)]
pub(super) struct OutputHitRegion {
    pub(super) scanout: ScanoutId,
    pub(super) rect: crate::types::Rect,
}

/// Every output whose hit region contains the pointer, in layout order.
///
/// Output-management clients may position outputs so that they overlap, so
/// more than one CRTC can match a single pointer position; callers must dirty
/// all of them.
pub(super) fn outputs_at_pointer(
    regions: &[OutputHitRegion],
    pointer: crate::types::Point,
) -> impl Iterator<Item = ScanoutId> + '_ {
    regions
        .iter()
        .filter(move |region| region.rect.contains_point(pointer))
        .map(|region| region.scanout)
}

pub(super) fn compute_total_dimensions(
    output_surfaces: &[OutputSurfaceEntry],
) -> crate::types::Rect {
    output_layout_bounds(
        output_surfaces.iter().map(|surface| surface.rect),
        crate::types::Size::new(
            crate::backend::wayland::render::drm::DEFAULT_SCREEN_WIDTH,
            crate::backend::wayland::render::drm::DEFAULT_SCREEN_HEIGHT,
        ),
    )
}

pub(super) fn output_layout_bounds(
    rects: impl IntoIterator<Item = crate::types::Rect>,
    fallback: crate::types::Size,
) -> crate::types::Rect {
    let mut min_x = None::<i32>;
    let mut min_y = None::<i32>;
    let mut max_x = None::<i32>;
    let mut max_y = None::<i32>;
    for rect in rects {
        let right = rect.x.saturating_add(rect.w);
        let bottom = rect.y.saturating_add(rect.h);
        min_x = Some(min_x.map_or(rect.x, |v| v.min(rect.x)));
        min_y = Some(min_y.map_or(rect.y, |v| v.min(rect.y)));
        max_x = Some(max_x.map_or(right, |v| v.max(right)));
        max_y = Some(max_y.map_or(bottom, |v| v.max(bottom)));
    }
    match (min_x, min_y, max_x, max_y) {
        (Some(min_x), Some(min_y), Some(max_x), Some(max_y)) => crate::types::Rect::new(
            min_x,
            min_y,
            max_x.saturating_sub(min_x).max(1),
            max_y.saturating_sub(min_y).max(1),
        ),
        _ => crate::types::Rect::new(0, 0, fallback.w.max(1), fallback.h.max(1)),
    }
}

#[cfg(test)]
pub(super) fn output_layout_size(
    rects: impl IntoIterator<Item = crate::types::Rect>,
    fallback: crate::types::Size,
) -> crate::types::Size {
    output_layout_bounds(rects, fallback).size()
}

pub(super) fn init_layout_state(
    output_surfaces: &[OutputSurfaceEntry],
    layout: crate::types::Rect,
) -> DrmLayoutState {
    layout_state_for_outputs(output_surfaces.iter(), layout)
}

fn layout_state_for_outputs<'a>(
    outputs: impl IntoIterator<Item = &'a OutputSurfaceEntry>,
    layout: crate::types::Rect,
) -> DrmLayoutState {
    DrmLayoutState {
        layout,
        output_hit_regions: outputs
            .into_iter()
            .map(|entry| OutputHitRegion {
                scanout: entry.scanout(),
                rect: entry.rect,
            })
            .collect(),
    }
}

pub(super) fn refresh_drm_layout_state(
    state: &WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    layout_state: &mut DrmLayoutState,
) {
    for entry in output_surfaces.iter_mut().filter(|entry| entry.enabled) {
        if let Some(geometry) = state.native.space.output_geometry(&entry.output) {
            entry.rect = crate::types::Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            );
        }
    }
    // Realized mirrors own no region of the layout; they redraw with their
    // source (see `propagate_mirror_render_flags`).
    let active: Vec<_> = output_surfaces
        .iter()
        .filter(|entry| {
            entry.enabled && state.native.space.output_geometry(&entry.output).is_some()
        })
        .collect();
    let layout = output_layout_bounds(
        active.iter().map(|entry| entry.rect),
        crate::types::Size::new(1, 1),
    );
    *layout_state = layout_state_for_outputs(active, layout);
}

/// A topology change can leave the pointer on an output that no longer exists.
/// Recover immediately, including for explicit/client-managed layouts with gaps.
pub(super) fn recover_pointer_after_output_change(state: &mut WaylandState) {
    if state
        .native
        .space
        .output_under(state.native.runtime.pointer_location)
        .next()
        .is_some()
    {
        return;
    }
    let target = state
        .wm
        .core
        .state
        .model
        .selected_monitor()
        .and_then(|monitor| {
            state
                .native
                .space
                .outputs()
                .find(|output| output.name() == monitor.name)
        })
        .or_else(|| state.native.space.outputs().next());
    let center = target
        .and_then(|output| state.native.space.output_geometry(output))
        .map(|geometry| {
            (
                geometry.loc.x as f64 + geometry.size.w as f64 / 2.0,
                geometry.loc.y as f64 + geometry.size.h as f64 / 2.0,
            )
        });
    if let Some((x, y)) = center {
        state.native.request_warp(x, y);
    }
}

pub(super) fn compact_drm_automatic_layout(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
) {
    let mut placements: Vec<_> = output_surfaces
        .iter()
        .filter(|entry| {
            entry.enabled && state.native.space.output_geometry(&entry.output).is_some()
        })
        .map(|entry| OutputPlacement {
            id: entry.output.name(),
            rect: entry.rect,
            source: state.native.output_position_source(&entry.output.name()),
        })
        .collect();
    for (name, position) in plan_automatic_output_positions(&mut placements) {
        let Some(entry) = output_surfaces
            .iter_mut()
            .find(|entry| entry.output.name() == name)
        else {
            continue;
        };
        entry.rect.x = position.x;
        entry.rect.y = position.y;
        entry
            .output
            .change_current_state(None, None, None, Some((position.x, position.y).into()));
        state
            .native
            .space
            .map_output(&entry.output, (position.x, position.y));
    }
}
