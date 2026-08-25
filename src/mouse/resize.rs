//! Backend-neutral interactive resize policy and geometry helpers.
//!
//! Recognition creates a shared `DragInteraction`; native backends only feed
//! normalized input to it. Free and aspect-preserving resize therefore use
//! identical geometry and lifecycle behavior on X11 and Wayland.

use crate::client::geometry::FloatingPlacementIntent;
use crate::contexts::WmCtx;
use crate::types::*;

use super::drag::move_drop::promote_to_floating;

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Calculate the new (position, dimension) for a single axis during a resize.
///
/// The `affects_start` flag indicates the start edge (left or top) is being
/// dragged, `affects_end` indicates the end edge (right or bottom) is being
/// dragged. Exactly one may be true; if neither, the axis is unchanged.
///
/// When dragging the start edge, the window position moves with the pointer
/// and size is reduced. When dragging the end edge, position stays fixed and
/// size grows. The `border_width` parameter is the border width, used to keep the
/// opposite border stationary when resizing from a corner.
pub(crate) fn compute_axis_resize(
    pointer: i32,
    orig_start: i32,
    orig_end: i32,
    border_width: i32,
    affects_start: bool,
    affects_end: bool,
) -> (i32, i32) {
    if affects_start {
        let nx = pointer;
        let nw = (orig_end - pointer).max(1);
        (nx, nw)
    } else if affects_end {
        // New width = pointer offset from start minus the two borders.
        // Adding 1 accounts for the pixel offset in the event coordinates.
        let nw = (pointer - orig_start - 2 * border_width + 1).max(1);
        (orig_start, nw)
    } else {
        (orig_start, orig_end - orig_start)
    }
}

pub(crate) fn constrain_aspect_size(
    ctx: &WmCtx<'_>,
    win: WindowId,
    raw_width: i32,
    raw_height: i32,
) -> (i32, i32) {
    let Some(client) = ctx.core().model().client(win) else {
        return (raw_width.max(1), raw_height.max(1));
    };
    let mut width = raw_width.max(1);
    let mut height = raw_height.max(1);
    let hints = client.size_hints;
    if hints.min_width > 0 {
        width = width.max(hints.min_width);
    }
    if hints.min_height > 0 {
        height = height.max(hints.min_height);
    }
    if hints.max_width > 0 {
        width = width.min(hints.max_width);
    }
    if hints.max_height > 0 {
        height = height.min(hints.max_height);
    }
    if client.min_aspect > 0.0 && client.max_aspect > 0.0 {
        let ratio = width as f32 / height as f32;
        if ratio > client.max_aspect {
            width = (height as f32 * client.max_aspect) as i32;
        } else if ratio < client.min_aspect {
            height = (width as f32 / client.min_aspect) as i32;
        }
    }
    (width.max(1), height.max(1))
}

/// Begin resizing `win` using the supplied input position's current quadrant.
///
/// The fullscreen check intentionally remains here even though title-drag
/// arming performs the same eligibility check: Wayland can change window mode
/// between the button press and the later drag-threshold event.
pub fn resize_from_point(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
    point: Point,
) {
    crate::client::fullscreen::leave_maximized(ctx, win);
    let Some((geo, is_floating)) = ctx.core().model().client(win).and_then(|client| {
        (!client.mode().is_true_fullscreen())
            .then_some((client.geo, client.mode().is_normal_floating()))
    }) else {
        return;
    };

    if let Some(tree_resize) = crate::layouts::manager::pointer_tree_resize_start(ctx, win, point) {
        let _ =
            crate::mouse::drag::tree_resize_begin(ctx, win, btn, source, point, geo, tree_resize);
        return;
    }

    // Promote tiled windows to floating before starting the resize.
    let has_tiling = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout();
    if !is_floating && has_tiling {
        let Some((new_geo, _)) = promote_to_floating(
            ctx,
            win,
            FloatingPlacementIntent::PreservePointerAnchor(point),
        ) else {
            return;
        };

        let dir = ResizeDirection::from_hit(new_geo.size(), new_geo.local_point(point));

        let _ = crate::mouse::drag::directional_resize_begin(ctx, win, btn, source, dir, new_geo);
        return;
    }

    let dir = ResizeDirection::from_hit(geo.size(), geo.local_point(point));

    let _ = crate::mouse::drag::directional_resize_begin(ctx, win, btn, source, dir, geo);
}

// ── resize_aspect_mouse ───────────────────────────────────────────────────────

/// Interactive resize that respects the window's declared aspect-ratio hints.
///
/// Reads `client.min_aspect`, `client.max_aspect`, and `client.size_hints` to clamp the
/// new dimensions so the window's aspect ratio stays within the range it
/// advertised via `WM_NORMAL_HINTS`.
///
/// Unlike [`resize_mouse`] this function does **not** toggle floating; it is
/// intended for use on windows that are already floating (e.g. video players
/// with a fixed aspect ratio).
pub fn resize_aspect_mouse(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
) {
    let Some(geo) = ctx.core().client_geo(win) else {
        return;
    };

    let _ = crate::mouse::drag::directional_resize_begin_with_policy(
        ctx,
        win,
        btn,
        source,
        ResizeDirection::BottomRight,
        geo,
        crate::core_state::ResizePolicy::PreserveAspect,
    );
}

// Hover-offer loops live in `super::hover`.
