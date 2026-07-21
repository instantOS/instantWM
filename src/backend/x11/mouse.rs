//! X11 mouse backend helpers.

use crate::backend::BackendEvent;
use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::contexts::WmCtxX11;
use crate::mouse::drag::{MoveState, on_motion, prepare_drag_target};
use crate::types::{AltCursor, MouseButton, Rect, WindowId};
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
/// Grab → event loop → release handling. This is deliberately reachable only
/// from pointer-driven actions; keyboard tree placement is a separate modal
/// interaction shared by both backends.
pub fn move_mouse(ctx: &mut WmCtxX11, btn: MouseButton, float_restore_geo: Option<Rect>) {
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
    let Some(grab_start_rect) =
        float_restore_geo.or_else(|| ctx.core.model().client(win).map(|client| client.geo))
    else {
        return;
    };

    let mut state = MoveState {
        start_point: start,
        grab_start_rect,
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };

    if ctx
        .core
        .drag_state_mut()
        .begin_move(win, btn, start, grab_start_rect)
        .is_err()
    {
        return;
    }

    crate::backend::x11::grab::mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let BackendEvent::Motion { root, .. } = event {
            let root = *root;
            ctx.core.drag_state_mut().record_interactive_motion(root);
            let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
            on_motion(&mut wm_ctx, win, root, root, &mut state);
        }
        true
    });

    crate::mouse::drag::lifecycle::finish(ctx.core.drag_state_mut(), &ctx.x11, btn)
        .expect("X11 drag loop must finish the interaction using its grab button");
    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    crate::mouse::drag::finish_drag_move(
        &mut wm_ctx,
        win,
        state.grab_start_rect,
        state.edge_snap_indicator,
        None,
    );
}

pub fn cursor_client_win(
    globals: &crate::core_state::CoreState,
    conn: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
) -> Option<WindowId> {
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;

    if reply.child == x11rb::NONE {
        return None;
    }

    let win = WindowId::from(reply.child);
    if globals.model.client(win).is_some() {
        Some(win)
    } else {
        None
    }
}
