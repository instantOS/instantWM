//! Window title bar drag operations.
//!
//! This module handles click and drag interactions on window title bars,
//! supporting both left-click (move) and right-click (resize/zoom) actions.

use crate::backend::x11::grab::mouse_drag_loop;
use crate::contexts::WmCtx;
use crate::layouts::restack;
use crate::mouse::constants::DRAG_THRESHOLD;
use crate::mouse::cursor::set_cursor_style;
use crate::mouse::drag::move_drop::promote_to_floating;
use crate::mouse::resize::resize_mouse_directional;
use crate::mouse::warp;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

/// Initialise a title-bar click/drag interaction.
///
/// Returns `true` if the state machine was started.  On X11 the caller
/// continues into the synchronous grab loop; on Wayland the calloop drives
/// [`title_drag_motion`] and [`title_drag_finish`].
pub fn title_drag_begin(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
    suppress_click_action: bool,
) -> bool {
    if btn == MouseButton::Right {
        let is_true_fullscreen = match ctx.client(win) {
            Some(c) => c.is_true_fullscreen(),
            None => return false,
        };
        if is_true_fullscreen {
            return false;
        }
        crate::focus::focus_soft(ctx, Some(win));
    }

    let sel = ctx.selected_client();
    let (win_start_geo, drop_restore_geo) = match ctx.client(win) {
        Some(c) => {
            let restore = c.restore_geo_for_float();
            (c.geo, restore)
        }
        None => return false,
    };
    ctx.core_mut().globals_mut().drag.interactive = crate::globals::DragInteraction {
        active: true,
        win,
        button: btn,
        was_focused: sel == Some(win),
        was_hidden: ctx.core_mut().globals_mut().clients.is_hidden(win),
        start_x: click_root_x,
        start_y: click_root_y,
        win_start_geo,
        drop_restore_geo,
        last_root_x: click_root_x,
        last_root_y: click_root_y,
        dragging: false,
        suppress_click_action,
        drag_type: crate::globals::DragType::Move,
    };
    true
}

/// Handle the transition from click to drag on Wayland when the threshold is exceeded.
fn title_drag_start_wayland(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    let (win, btn, start_x, start_y) = {
        let drag = &ctx.core().globals().drag.interactive;
        (drag.win, drag.button, drag.start_x, drag.start_y)
    };
    let is_right_click = btn == MouseButton::Right;

    if is_right_click {
        // Right-click: promote to floating, set up resize mode, warp cursor.
        let (current_geo, _) = promote_to_floating(ctx, win, None);

        let hit_x = start_x - current_geo.x;
        let hit_y = start_y - current_geo.y;
        let dir =
            crate::types::input::get_resize_direction(current_geo.w, current_geo.h, hit_x, hit_y);

        let bw = match ctx.client(win) {
            Some(c) => c.border_width,
            None => return true,
        };
        let (x_off, y_off) = dir.warp_offset(current_geo.w, current_geo.h, bw);
        let warp_x = current_geo.x + x_off;
        let warp_y = current_geo.y + y_off;

        if let WmCtx::Wayland(wl) = ctx {
            wl.wayland
                .backend
                .warp_pointer(warp_x as f64, warp_y as f64);
            wl.core.globals_mut().drag.interactive = crate::globals::DragInteraction {
                active: true,
                win,
                button: btn,
                dragging: true,
                drag_type: crate::globals::DragType::Resize(dir),
                win_start_geo: current_geo,
                start_x: warp_x,
                start_y: warp_y,
                last_root_x: warp_x,
                last_root_y: warp_y,
                drop_restore_geo: current_geo,
                ..Default::default()
            };
            set_cursor_style(&mut WmCtx::Wayland(wl.reborrow()), AltCursor::Resize(dir));
        }
        return true;
    }

    // Left-click: promote to floating (centering under pointer if newly floated),
    // and keep title drag active so calloop drives it.
    let (current_geo, anchor_rebased) = promote_to_floating(ctx, win, Some((root_x, root_y)));

    if anchor_rebased {
        ctx.core_mut().globals_mut().drag.interactive.win_start_geo = current_geo;
        ctx.core_mut().globals_mut().drag.interactive.start_x = root_x;
        ctx.core_mut().globals_mut().drag.interactive.start_y = root_y;
    } else {
        warp::warp_into(ctx, win);
        let ptr = warp::get_root_ptr(ctx).unwrap_or((root_x, root_y));
        let pad = warp::WARP_INTO_PADDING;
        let clamped_x = ptr
            .0
            .clamp(current_geo.x + pad, current_geo.x + current_geo.w - pad);
        let clamped_y = ptr
            .1
            .clamp(current_geo.y + pad, current_geo.y + current_geo.h - pad);
        ctx.core_mut().globals_mut().drag.interactive.start_x = clamped_x;
        ctx.core_mut().globals_mut().drag.interactive.start_y = clamped_y;
    }

    set_cursor_style(ctx, AltCursor::Move);
    ctx.core_mut().globals_mut().drag.interactive.dragging = true;
    title_drag_motion(ctx, root_x, root_y)
}

/// Process a pointer motion event during an active title drag.
///
/// Returns `true` if the drag threshold was exceeded and the drag action
/// (move/resize) was initiated — the caller should consider the interaction
/// consumed.
pub fn title_drag_motion(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    if !ctx.core().globals().drag.interactive.active {
        return false;
    }
    ctx.core_mut().globals_mut().drag.interactive.last_root_x = root_x;
    ctx.core_mut().globals_mut().drag.interactive.last_root_y = root_y;

    if ctx.core().globals().drag.interactive.dragging {
        // Once dragging is active the unified handler
        // (wayland_hover_resize_drag_motion) drives the interaction.
        return false;
    }

    let td = &ctx.core_mut().globals_mut().drag.interactive;
    if (root_x - td.start_x).abs() <= DRAG_THRESHOLD
        && (root_y - td.start_y).abs() <= DRAG_THRESHOLD
    {
        return false;
    }

    // Threshold exceeded — start the drag action.
    let drag = &ctx.core().globals().drag.interactive;
    let win = drag.win;
    let btn = drag.button;
    let was_hidden = drag.was_hidden;
    let is_right_click = btn == MouseButton::Right;

    if was_hidden {
        crate::client::show(ctx, win);
    }
    crate::focus::focus_soft(ctx, Some(win));
    ctx.raise_interactive(win);

    if ctx.is_wayland() {
        return title_drag_start_wayland(ctx, root_x, root_y);
    }

    // X11 specific start logic
    ctx.core_mut().globals_mut().drag.interactive.dragging = true;
    ctx.core_mut().globals_mut().drag.interactive.active = false;

    if is_right_click {
        if let Some(c) = ctx.core().client(win) {
            let (x_off, y_off) =
                ResizeDirection::BottomRight.warp_offset(c.geo.w, c.geo.h, c.border_width);
            if let WmCtx::X11(x11) = ctx {
                let x11_win: Window = win.into();
                let _ = x11.x11.conn.warp_pointer(
                    x11rb::NONE,
                    x11_win,
                    0i16,
                    0i16,
                    0u16,
                    0u16,
                    x_off as i16,
                    y_off as i16,
                );
                let _ = x11.x11.conn.flush();
            }
        }
        if let WmCtx::X11(x11) = ctx {
            resize_mouse_directional(x11, Some(ResizeDirection::BottomRight), btn);
        }
    } else {
        // Pass saved floating dimensions to preserve them when dropping on the bar
        let float_restore_geo = ctx
            .core_mut()
            .globals_mut()
            .drag
            .interactive
            .drop_restore_geo;
        if let WmCtx::X11(x11) = ctx {
            let mut wmctx = WmCtx::X11(x11.reborrow());
            warp::warp_into(&mut wmctx, win);
            crate::backend::x11::mouse::move_mouse_x11(x11, btn, Some(float_restore_geo));
        }
    }
    true
}

/// Finish a title drag interaction (button release without exceeding the
/// drag threshold).  Performs the click action (focus / hide / zoom).
///
/// When the drag threshold *was* exceeded (`dragging == true`), the
/// unified `wayland_hover_resize_drag_finish` handles the drop instead.
pub fn title_drag_finish(ctx: &mut WmCtx) {
    if !ctx.core_mut().globals_mut().drag.interactive.active
        || ctx.core_mut().globals_mut().drag.interactive.dragging
    {
        return;
    }

    let drag = &ctx.core().globals().drag.interactive;
    let win = drag.win;
    let is_right_click = drag.button == MouseButton::Right;
    let was_focused = drag.was_focused;
    let was_hidden = drag.was_hidden;
    let suppress_click_action = drag.suppress_click_action;

    ctx.core_mut().globals_mut().drag.interactive.active = false;
    if suppress_click_action {
        return;
    }

    if is_right_click {
        if was_hidden {
            crate::client::show(ctx, win);
            crate::focus::focus_soft(ctx, Some(win));
        }
        crate::client::zoom(ctx);
    } else if was_hidden {
        crate::client::show(ctx, win);
        crate::focus::focus_soft(ctx, Some(win));
        let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
        restack(ctx, selmon_id);
    } else if was_focused {
        crate::client::hide_for_user(ctx, win);
    } else {
        crate::focus::focus_soft(ctx, Some(win));
        let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
        restack(ctx, selmon_id);
    }
}

/// Left-click / drag handler for a window title bar entry.
///
/// Click: hidden → show+focus; focused → hide; otherwise → focus.
/// Drag > [`DRAG_THRESHOLD`]: show, focus, warp, hand off to [`crate::backend::x11::mouse::move_mouse_x11`].
/// Right Click: same as above but allows zoom to master and bottom-right resize on drag.
///
/// On Wayland, starts the async state machine and returns immediately.
/// On X11, runs a synchronous grab loop.
pub fn window_title_mouse_handler(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    if !title_drag_begin(ctx, win, btn, click_root_x, click_root_y, false) {
        return;
    }

    match ctx {
        WmCtx::X11(ctx_x11) => {
            let cursor = if btn == MouseButton::Right {
                AltCursor::Move
            } else {
                AltCursor::Default
            };
            mouse_drag_loop(ctx_x11, btn, cursor, false, |ctx, event| {
                if let x11rb::protocol::Event::MotionNotify(m) = event {
                    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                    if title_drag_motion(&mut wm_ctx, m.event_x as i32, m.event_y as i32) {
                        return false;
                    }
                }
                true
            });
            let mut wm_ctx = WmCtx::X11(ctx_x11.reborrow());
            title_drag_finish(&mut wm_ctx);
        }
        WmCtx::Wayland(_) => {}
    }
}

/// Right-click handler for a window title bar entry.
///
/// This is currently an alias for [`window_title_mouse_handler`] since both
/// left and right clicks are handled by the same function with different
/// behaviors based on the button.
pub fn window_title_mouse_handler_right(
    ctx: &mut WmCtx,
    win: WindowId,
    click_root_x: i32,
    click_root_y: i32,
) {
    window_title_mouse_handler(ctx, win, MouseButton::Right, click_root_x, click_root_y);
}
