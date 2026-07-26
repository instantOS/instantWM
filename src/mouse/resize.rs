//! Interactive mouse-resize operations.
//!
//! Three distinct resize modes are provided:
//!
//! | Function                  | Description                                                  |
//! |---------------------------|--------------------------------------------------------------|
//! | [`resize_mouse`]          | Drag the bottom-right corner to resize                      |
//! | [`resize_aspect_mouse`]   | Same, but clamps to the window's declared aspect-ratio hints |
//! | [`force_resize_mouse`]    | Alias for `resize_mouse` (bypasses fullscreen guard)        |
//!
//! All three share the same grab/event-loop/ungrab skeleton; they differ only
//! in how they compute the new width and height from the pointer position.
//!
//! On Wayland, `resize_mouse_from_cursor` and `resize_aspect_mouse` bypass the
//! title-drag state machine and instead directly activate a
//! `DragInteraction`.  This reuses the same directional-resize event loop
//! that hover-border drags use, giving correct per-quadrant behaviour without
//! any cursor warp or anchor chaos.

use crate::client::geometry::FloatingPlacementIntent;
use crate::contexts::WmCtx;
use crate::types::*;

use super::drag::lifecycle::{ResizeDragParams, begin_resize};
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

/// Begin resizing `win` using the pointer's current quadrant.
///
/// The fullscreen check intentionally remains here even though title-drag
/// arming performs the same eligibility check: Wayland can change window mode
/// between the button press and the later drag-threshold event.
pub fn resize_mouse_from_cursor(ctx: &mut WmCtx, win: WindowId, btn: MouseButton) {
    let Some((geo, is_floating)) = ctx.core().model().client(win).and_then(|client| {
        (!client.mode().is_true_fullscreen()).then_some((client.geo, client.mode().is_floating()))
    }) else {
        return;
    };

    let Some(ptr) = ctx.pointer_backend().pointer_location() else {
        return;
    };

    if let Some(tree_resize) = crate::layouts::manager::pointer_tree_resize_start(ctx, win, ptr) {
        match ctx {
            WmCtx::X11(x11) => {
                crate::backend::x11::mouse::resize_tree_mouse_x11(
                    x11,
                    win,
                    btn,
                    ptr,
                    geo,
                    tree_resize,
                );
            }
            WmCtx::Wayland(wl) => {
                begin_wayland_tree_resize(wl, win, btn, ptr, geo, tree_resize);
            }
        }
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
            FloatingPlacementIntent::PreservePointerAnchor(ptr),
        ) else {
            return;
        };

        let dir = ResizeDirection::from_hit(new_geo.size(), new_geo.local_point(ptr));

        match ctx {
            WmCtx::X11(x11) => {
                crate::backend::x11::mouse::resize_mouse_directional(x11, Some(dir), btn);
            }
            WmCtx::Wayland(wl) => {
                begin_wayland_super_resize(wl, win, btn, dir, new_geo);
            }
        }
        return;
    }

    let dir = ResizeDirection::from_hit(geo.size(), geo.local_point(ptr));

    match ctx {
        WmCtx::X11(x11) => {
            crate::backend::x11::mouse::resize_mouse_directional(x11, Some(dir), btn);
        }
        WmCtx::Wayland(wl) => {
            begin_wayland_super_resize(wl, win, btn, dir, geo);
        }
    }
}

fn begin_wayland_tree_resize(
    wl: &mut crate::contexts::WmCtxWayland<'_>,
    win: WindowId,
    btn: MouseButton,
    start: Point,
    geo: Rect,
    resize: crate::layouts::manager::PointerTreeResizeStart,
) {
    if wl
        .core
        .drag_state_mut()
        .begin_tree_resize(win, btn, resize.direction, start, geo, resize.origin)
        .is_err()
    {
        return;
    }
    prepare_wayland_resize(wl, win, resize.direction);
}

fn prepare_wayland_resize(
    wl: &mut crate::contexts::WmCtxWayland<'_>,
    win: WindowId,
    direction: ResizeDirection,
) {
    let mut ctx = WmCtx::Wayland(wl.reborrow());
    ctx.set_cursor_style(AltCursor::Resize(direction));
    crate::focus::focus(&mut ctx, Some(win));
    ctx.raise_client(win);
}

/// Activate a `DragInteraction` for a Super+RMB resize initiated anywhere
/// on a Wayland window (not just the hover-border zone).  This reuses the same
/// directional-resize event loop as hover-border resizes, giving correct
/// per-quadrant behaviour with cursor warped to the nearest edge/corner.
fn begin_wayland_super_resize(
    wl: &mut crate::contexts::WmCtxWayland<'_>,
    win: WindowId,
    btn: MouseButton,
    dir: ResizeDirection,
    geo: Rect,
) {
    // Warp the cursor to the nearest edge/corner for this direction so the
    // visual position of the cursor matches what is being dragged.  The resize
    // math in hover_resize_drag_motion uses root_x/root_y directly
    // against the window edges, so the first motion event is correct regardless
    // of where the cursor started — but warping gives immediate visual feedback
    // and prevents the cursor sitting in the middle of the window while a corner
    // is moving.
    let root = {
        let mut wmctx = WmCtx::Wayland(wl.reborrow());
        match super::warp::warp_to_resize_corner(&mut wmctx, win, dir) {
            Some(p) => p,
            None => return,
        }
    };

    if begin_resize(
        wl.core.drag_state_mut(),
        wl.wayland,
        ResizeDragParams {
            win,
            button: btn,
            direction: dir,
            start: root,
            geometry: geo,
        },
    )
    .is_err()
    {
        return;
    }
    prepare_wayland_resize(wl, win, dir);
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
pub fn resize_aspect_mouse(ctx: &mut WmCtx, win: WindowId, btn: MouseButton) {
    let Some(ptr) = ctx.pointer_backend().pointer_location() else {
        return;
    };

    let Some(geo) = ctx.client_geo(win) else {
        return;
    };

    let dir = ResizeDirection::from_hit(geo.size(), geo.local_point(ptr));

    match ctx {
        WmCtx::X11(x11) => {
            crate::backend::x11::mouse::resize_aspect_mouse_x11(x11, win, btn);
        }
        WmCtx::Wayland(wl) => {
            begin_wayland_super_resize(wl, win, btn, dir, geo);
        }
    }
}

// Hover-offer loops live in `super::hover`.
