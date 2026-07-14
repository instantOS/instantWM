//! Window title bar drag operations.
//!
//! This module handles click and drag interactions on window title bars,
//! supporting both left-click (move) and right-click (resize/zoom) actions.

use crate::backend::BackendEvent;
use crate::contexts::WmCtx;
use crate::layouts::sync_monitor_z_order;
use crate::mouse::constants::DRAG_THRESHOLD;
use crate::mouse::cursor::set_cursor_style;
use crate::mouse::drag::lifecycle::activate_armed_resize;
use crate::mouse::drag::move_drop::promote_to_floating;
use crate::mouse::resize::resize_mouse_directional;
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
            Some(c) => c.mode.is_true_fullscreen(),
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
            let restore = c.restore_geo_for_float();
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
fn title_drag_start_wayland(ctx: &mut WmCtx, root: Point) -> bool {
    let (win, btn, start_point) = {
        let Some(drag) = ctx.core().drag_state().armed_interaction() else {
            return false;
        };
        (drag.win(), drag.button(), drag.start_point())
    };
    let is_right_click = btn == MouseButton::Right;

    if is_right_click {
        // Right-click: promote to floating, set up resize mode, warp cursor.
        let Some((current_geo, _)) = promote_to_floating(ctx, win, None) else {
            return false;
        };

        let hit_x = start_point.x - current_geo.x;
        let hit_y = start_point.y - current_geo.y;
        let dir =
            crate::types::input::get_resize_direction(current_geo.w, current_geo.h, hit_x, hit_y);

        let bw = match ctx.core().model().client(win) {
            Some(c) => c.border_width,
            None => return true,
        };
        let (x_off, y_off) = dir.warp_offset(current_geo.w, current_geo.h, bw);
        let warp_x = current_geo.x + x_off;
        let warp_y = current_geo.y + y_off;
        let warp_point = Point::new(warp_x, warp_y);

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
            wl.wayland.warp_pointer(warp_x as f64, warp_y as f64);
            set_cursor_style(&mut WmCtx::Wayland(wl.reborrow()), AltCursor::Resize(dir));
        }
        return true;
    }

    // Left-click: promote to floating (centering under pointer if newly floated),
    // and keep title drag active so calloop drives it.
    let Some((current_geo, anchor_rebased)) = promote_to_floating(ctx, win, Some(root)) else {
        return false;
    };

    let start = if anchor_rebased {
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
        .activate_armed(crate::core_state::DragType::Move, start, current_geo)
        .is_err()
    {
        return false;
    }
    set_cursor_style(ctx, AltCursor::Move);
    true
}

/// Process a pointer motion event during an active title drag.
///
/// Returns `true` if the drag threshold was exceeded and the drag action
/// (move/resize) was initiated — the caller should consider the interaction
/// consumed.
pub fn title_drag_motion(ctx: &mut WmCtx, root: Point) -> bool {
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
        return title_drag_start_wayland(ctx, root);
    }

    // X11 uses a nested synchronous grab loop. Consume the armed click
    // interaction before starting the immediate move/resize interaction.
    let Some(armed) = ctx.core_mut().drag_state_mut().finish_armed() else {
        return false;
    };

    if is_right_click {
        if let Some(c) = ctx.core().model().client(win) {
            let (x_off, y_off) =
                ResizeDirection::BottomRight.warp_offset(c.geo.w, c.geo.h, c.border_width);
            ctx.pointer_backend()
                .warp_pointer((c.geo.x + x_off) as f64, (c.geo.y + y_off) as f64);
        }
        if let WmCtx::X11(x11) = ctx {
            resize_mouse_directional(x11, Some(ResizeDirection::BottomRight), btn);
        }
    } else {
        // Pass saved floating dimensions to preserve them when dropping on the bar
        let float_restore_geo = armed.drop_restore_geo();
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
    } else if was_hidden {
        crate::client::show_window(ctx, win);
        crate::focus::focus(ctx, Some(win));
        let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();
        sync_monitor_z_order(ctx, selmon_id);
    } else if was_focused {
        crate::client::hide_for_user(ctx, win);
    } else {
        crate::focus::focus(ctx, Some(win));
        let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();
        sync_monitor_z_order(ctx, selmon_id);
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
    if !title_drag_begin(ctx, win, btn, click_root, false) {
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
