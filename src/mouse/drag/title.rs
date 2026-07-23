//! Window title bar drag operations.
//!
//! This module handles click and drag interactions on window title bars,
//! supporting both left-click (move) and right-click (resize/zoom) actions.

use crate::backend::BackendEvent;
use crate::client::geometry::FloatingPlacementIntent;
use crate::contexts::WmCtx;
use crate::mouse::constants::DRAG_THRESHOLD;
use crate::mouse::drag::lifecycle::activate_armed_resize;
use crate::mouse::drag::move_drop::promote_to_floating;
use crate::mouse::resize::{resize_mouse_directional, resize_mouse_from_cursor};
use crate::mouse::warp;
use crate::types::geometry::Point;
use crate::types::*;

/// Initialise a title-bar click/drag interaction.
///
/// Returns `true` if the state machine was started.  On X11 the caller
/// continues into the synchronous grab loop; on Wayland the calloop drives
/// [`title_drag_motion`] and [`title_drag_finish`].
pub fn title_drag_begin(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root: Point,
    suppress_click_action: bool,
) -> bool {
    if btn == MouseButton::Right {
        let is_true_fullscreen = match ctx.core().model().client(win) {
            Some(c) => c.mode().is_true_fullscreen(),
            None => return false,
        };
        if is_true_fullscreen {
            return false;
        }
        crate::focus::focus(ctx, Some(win));
    }

    let sel = ctx.core().model().selected_win();
    let (win_start_geo, drop_restore_geo) = match ctx.core().model().client(win) {
        Some(c) => {
            let restore = c.saved_floating_rect().unwrap_or(c.geo);
            (c.geo, restore)
        }
        None => return false,
    };
    let was_hidden = ctx
        .core()
        .model()
        .client(win)
        .is_some_and(|client| client.is_hidden);
    ctx.core_mut()
        .drag_state_mut()
        .arm_title_drag(crate::core_state::ArmedDragParams {
            win,
            button: btn,
            start: click_root,
            geometry: win_start_geo,
            restore_geometry: drop_restore_geo,
            was_focused: sel == Some(win),
            was_hidden,
            suppress_click_action,
        })
        .is_ok()
}

/// Handle the transition from click to drag on Wayland when the threshold is exceeded.
fn title_drag_start_wayland(ctx: &mut WmCtx, root: Point, direct_position: bool) -> bool {
    let (win, btn, start_point, suppress_click_action) = {
        let Some(drag) = ctx.core().drag_state().armed_interaction() else {
            return false;
        };
        (
            drag.win(),
            drag.button(),
            drag.start_point(),
            drag.suppress_click_action(),
        )
    };
    let is_right_click = btn == MouseButton::Right;

    if is_right_click {
        if crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win) {
            // Bar-title resizing retains its established bottom-right handle;
            // Super+right-drag uses the pointer's quadrant instead.
            if !suppress_click_action {
                let _ = warp::warp_to_resize_corner(ctx, win, ResizeDirection::BottomRight);
            }
            // Tree resize owns an initial tree snapshot, so replace the armed
            // click with the authoritative resize interaction after the drag
            // threshold has been crossed.
            let _ = ctx.core_mut().drag_state_mut().finish_armed();
            resize_mouse_from_cursor(ctx, win, btn);
            return true;
        }

        // Right-click: promote to floating, set up resize mode, warp cursor.
        let Some((current_geo, _)) = promote_to_floating(
            ctx,
            win,
            FloatingPlacementIntent::PreservePointerAnchor(start_point),
        ) else {
            return false;
        };

        let dir = if suppress_click_action {
            ResizeDirection::from_hit(current_geo.size(), current_geo.local_point(start_point))
        } else {
            ResizeDirection::BottomRight
        };

        let Some(warp_point) = warp::warp_to_resize_corner(ctx, win, dir) else {
            return true;
        };

        if let WmCtx::Wayland(wl) = ctx {
            if activate_armed_resize(
                wl.core.drag_state_mut(),
                wl.wayland,
                dir,
                warp_point,
                current_geo,
            )
            .is_err()
            {
                return false;
            }
            WmCtx::Wayland(wl.reborrow()).set_cursor_style(AltCursor::Resize(dir));
        }
        return true;
    }

    // A tiled left-drag is a manual-tree placement gesture. Floating windows
    // continue to move directly. Keeping the tiled source in its original slot
    // also makes cancellation lossless.
    let uses_tree = crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win);
    let (current_geo, anchor_rebased) = if uses_tree {
        let Some(geo) = ctx.client_geo(win) else {
            return false;
        };
        (geo, false)
    } else {
        let intent = if suppress_click_action || direct_position {
            FloatingPlacementIntent::PreservePointerAnchor(root)
        } else {
            FloatingPlacementIntent::RestoreOrCenter
        };
        let Some(result) = promote_to_floating(ctx, win, intent) else {
            return false;
        };
        result
    };

    let start = if uses_tree {
        start_point
    } else if anchor_rebased {
        root
    } else if direct_position {
        root
    } else {
        warp::warp_into(ctx, win);
        let ptr = ctx.pointer_backend().pointer_location().unwrap_or(root);
        let pad = warp::WARP_INTO_PADDING;
        let clamped_x = ptr
            .x
            .clamp(current_geo.x + pad, current_geo.x + current_geo.w - pad);
        let clamped_y = ptr
            .y
            .clamp(current_geo.y + pad, current_geo.y + current_geo.h - pad);
        Point::new(clamped_x, clamped_y)
    };

    if ctx
        .core_mut()
        .drag_state_mut()
        .activate_armed(crate::core_state::ArmedDragType::Move, start, current_geo)
        .is_err()
    {
        return false;
    }
    ctx.set_cursor_style(AltCursor::Move);
    true
}

/// Process a pointer motion event during an active title drag.
///
/// Returns `true` if the drag threshold was exceeded and the drag action
/// (move/resize) was initiated — the caller should consider the interaction
/// consumed.
pub fn title_drag_motion(ctx: &mut WmCtx, root: Point) -> bool {
    title_drag_motion_at(ctx, root, false)
}

/// Process motion from an absolute interaction that is independent of the
/// compositor pointer, such as a touchscreen sequence captured by the bar.
///
/// Unlike pointer motion this preserves the contact point as the window
/// anchor and does not warp or consult the unrelated mouse cursor.
pub(crate) fn title_drag_motion_at(ctx: &mut WmCtx, root: Point, direct_position: bool) -> bool {
    let Some(armed) = ctx.core().drag_state().armed_interaction() else {
        return false;
    };

    if root.manhattan_distance(&armed.start_point()) <= DRAG_THRESHOLD {
        ctx.core_mut()
            .drag_state_mut()
            .record_interactive_motion(root);
        return false;
    }

    // Threshold exceeded — start the drag action.
    let drag = armed.clone();
    let win = drag.win();
    let btn = drag.button();
    let was_hidden = drag.was_hidden();
    let is_right_click = btn == MouseButton::Right;

    if was_hidden {
        crate::client::show_window(ctx, win);
    }
    crate::focus::focus(ctx, Some(win));
    ctx.raise_client(win);

    if ctx.is_wayland() {
        return title_drag_start_wayland(ctx, root, direct_position);
    }

    // X11 uses a nested synchronous grab loop. Consume the armed click
    // interaction before starting the immediate move/resize interaction.
    let Some(armed) = ctx.core_mut().drag_state_mut().finish_armed() else {
        return false;
    };

    if is_right_click {
        if crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win) {
            if !armed.suppress_click_action() {
                let _ = warp::warp_to_resize_corner(ctx, win, ResizeDirection::BottomRight);
            }
            resize_mouse_from_cursor(ctx, win, btn);
            return true;
        }

        // The initial title/client drag already crossed the threshold. Promote
        // now rather than making the user cross a second resize threshold.
        let direction = if armed.suppress_click_action() {
            let Some(geo) = ctx.client_geo(win) else {
                return false;
            };
            ResizeDirection::from_hit(geo.size(), geo.local_point(armed.start_point()))
        } else {
            ResizeDirection::BottomRight
        };
        let Some((_current_geo, _)) = promote_to_floating(
            ctx,
            win,
            FloatingPlacementIntent::PreservePointerAnchor(armed.start_point()),
        ) else {
            return false;
        };
        let _ = warp::warp_to_resize_corner(ctx, win, direction);
        if let WmCtx::X11(x11) = ctx {
            resize_mouse_directional(x11, Some(direction), btn);
        }
    } else {
        let float_restore_geo = armed.drop_restore_geo();
        if !crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win)
            && promote_to_floating(
                ctx,
                win,
                if armed.suppress_click_action() {
                    FloatingPlacementIntent::PreservePointerAnchor(root)
                } else {
                    FloatingPlacementIntent::RestoreOrCenter
                },
            )
            .is_none()
        {
            return false;
        }
        // Pass saved floating dimensions to preserve them when dropping on the bar
        if let WmCtx::X11(x11) = ctx {
            let mut wmctx = WmCtx::X11(x11.reborrow());
            warp::warp_into(&mut wmctx, win);
            crate::backend::x11::mouse::move_mouse(x11, btn, Some(float_restore_geo));
        }
    }
    true
}

/// Finish a title drag interaction (button release without exceeding the
/// drag threshold).  Performs the click action (focus / hide / zoom).
///
/// Once the drag threshold promotes the interaction to `Active`, the unified
/// `hover_resize_drag_finish` handles the drop instead.
pub fn title_drag_finish(ctx: &mut WmCtx) {
    let Some(drag) = ctx.core_mut().drag_state_mut().finish_armed() else {
        return;
    };
    let win = drag.win();
    let is_right_click = drag.button() == MouseButton::Right;
    let was_focused = drag.was_focused();
    let was_hidden = drag.was_hidden();
    let suppress_click_action = drag.suppress_click_action();
    if suppress_click_action {
        return;
    }

    if is_right_click {
        if was_hidden {
            crate::client::show_window(ctx, win);
            crate::focus::focus(ctx, Some(win));
        }
        crate::client::zoom(ctx);
    } else if was_focused && !was_hidden {
        crate::client::hide_for_user(ctx, win);
    } else {
        if was_hidden {
            crate::client::show_window(ctx, win);
        }
        crate::focus::focus(ctx, Some(win));
        // A bar title is an explicit stacking handle even when ordinary
        // client-area click-to-raise is disabled.
        ctx.raise_client(win);
    }
}

/// Left-click / drag handler for a window title bar entry.
///
/// Click: hidden → show+focus; focused → hide; otherwise → focus.
/// Drag > [`DRAG_THRESHOLD`]: show, focus, warp, hand off to [`crate::backend::x11::mouse::move_mouse`].
/// Right Click: same as above but allows zoom to master and bottom-right resize on drag.
///
/// On Wayland, starts the async state machine and returns immediately.
/// On X11, runs a synchronous grab loop.
pub fn window_title_mouse_handler(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root: Point,
) {
    thresholded_client_drag(ctx, win, btn, click_root, false);
}

/// Start a client move/resize that remains a click until the pointer crosses
/// [`DRAG_THRESHOLD`]. Used by both Super+client drags and bar-title drags so
/// X11 and Wayland have identical activation semantics.
pub fn thresholded_client_drag(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root: Point,
    suppress_click_action: bool,
) {
    if !title_drag_begin(ctx, win, btn, click_root, suppress_click_action) {
        return;
    }

    match ctx {
        WmCtx::X11(ctx_x11) => {
            let cursor = if btn == MouseButton::Right {
                AltCursor::Move
            } else {
                AltCursor::Default
            };
            crate::backend::x11::grab::mouse_drag_loop(
                ctx_x11,
                btn,
                cursor,
                false,
                |ctx, event| {
                    if let BackendEvent::Motion { root, .. } = event {
                        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                        if title_drag_motion(&mut wm_ctx, *root) {
                            return false;
                        }
                    }
                    true
                },
            );
            let mut wm_ctx = WmCtx::X11(ctx_x11.reborrow());
            title_drag_finish(&mut wm_ctx);
        }
        WmCtx::Wayland(_) => {}
    }
}
