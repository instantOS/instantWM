//! X11 mouse backend helpers.

use crate::backend::BackendEvent;
use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::contexts::WmCtxX11;
use crate::mouse::drag::{MoveState, on_motion, prepare_drag_target};
use crate::types::{AltCursor, MouseButton, Point, Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt};

/// Set the root window cursor for the given cursor style.
///
/// Looks up the corresponding X11 cursor in the cached array and applies it to
/// the root window.  No-op if the requested style is already active.
pub fn set_x11_root_cursor(
    x11: &X11BackendRef<'_>,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
) {
    if x11_runtime.last_x11_cursor == Some(cursor) {
        return;
    }
    let conn = x11.conn;
    let root = x11_runtime.root;
    let cursor_index = cursor.to_x11_index();
    if let Some(Some(loaded_cursor)) = x11_runtime.cursors.get(cursor_index) {
        let _ = xproto::change_window_attributes(
            conn,
            root,
            &xproto::ChangeWindowAttributesAux::new().cursor(loaded_cursor.cursor as u32),
        );
        let _ = conn.flush();
        x11_runtime.last_x11_cursor = Some(cursor);
    }
}

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
    let Some(start) = wm_ctx.pointer_backend().pointer_location() else {
        return;
    };

    // Use override from title drag if available (preserves pre-drag floating dimensions),
    // otherwise get the current client geometry.
    let grab_start_rect = float_restore_geo
        .or_else(|| ctx.core.model().clients.geo(win))
        .unwrap_or_default();

    let mut state = MoveState {
        start_point: start,
        grab_start_rect,
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };

    ctx.core.drag_state_mut().interactive =
        crate::core_state::DragInteraction::new_move(win, btn, start, grab_start_rect);

    crate::backend::x11::grab::mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let BackendEvent::Motion { root_x, root_y, .. } = event {
            let root = Point::new(*root_x as i32, *root_y as i32);
            ctx.core.drag_state_mut().interactive.last_root_point = root;
            let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
            on_motion(&mut wm_ctx, win, root, root, &mut state);
        }
        true
    });

    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    crate::mouse::drag::finish_drag_move(
        &mut wm_ctx,
        win,
        state.grab_start_rect,
        state.edge_snap_indicator,
        None,
    );
}

pub fn get_cursor_client_win_with_conn(
    globals: &crate::core_state::CoreState,
    conn: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
) -> Option<WindowId> {
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;

    if reply.child == x11rb::NONE {
        return None;
    }

    let win = WindowId::from(reply.child);
    if globals.model.clients.contains_key(&win) {
        Some(win)
    } else {
        None
    }
}
