#![allow(dead_code)]
//! X11 mouse backend helpers.

use crate::contexts::{CoreCtx, WmCtxX11};
use crate::mouse::drag::{
    clear_bar_hover, complete_move_drop, on_motion, prepare_drag_target, MoveState,
};
use crate::mouse::warp::get_root_ptr;
use crate::types::{AltCursor, MouseButton, Rect, WindowId};
use x11rb::protocol::xproto::ConnectionExt;

/// X11-only synchronous window move implementation.
///
/// Grab → event loop → release handling. This is the X11-specific synchronous
/// implementation. For the backend-agnostic keyboard shortcut, use
/// [`crate::mouse::drag::begin_keyboard_move`] instead.
pub fn move_mouse_x11(ctx: &mut WmCtxX11, btn: MouseButton, float_restore_geo: Option<Rect>) {
    let Some(win) = ({
        let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
        prepare_drag_target(&mut wm_ctx)
    }) else {
        return;
    };

    let wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    let Some((start_x, start_y)) = get_root_ptr(&wm_ctx) else {
        return;
    };

    // Use override from title drag if available (preserves pre-drag floating dimensions),
    // otherwise get the current client geometry.
    let grab_start_rect = float_restore_geo
        .or_else(|| ctx.core.g.clients.geo(win))
        .unwrap_or_default();

    let mut state = MoveState {
        start_x,
        start_y,
        grab_start_rect,
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };

    crate::backend::x11::grab::mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
            on_motion(
                &mut wm_ctx,
                win,
                m.event_x as i32,
                m.event_y as i32,
                m.root_x as i32,
                m.root_y as i32,
                &mut state,
            );
        }
        true
    });

    {
        let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
        clear_bar_hover(&mut wm_ctx);
    }

    {
        let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
        complete_move_drop(
            &mut wm_ctx,
            win,
            state.grab_start_rect,
            state.edge_snap_indicator,
            None,
        );
    }
}

pub fn get_cursor_client_win_with_conn(
    core: &CoreCtx,
    conn: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
) -> Option<WindowId> {
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;

    if reply.child == x11rb::NONE {
        return None;
    }

    let win = WindowId::from(reply.child);
    if core.g.clients.contains_key(&win) {
        Some(win)
    } else {
        None
    }
}
