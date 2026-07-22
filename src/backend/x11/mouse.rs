//! X11 mouse backend helpers.

use crate::backend::BackendEvent;
use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::contexts::WmCtxX11;
use crate::mouse::drag::{MoveState, on_motion, prepare_drag_target};
use crate::types::{AltCursor, MouseButton, Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorUpdateTargets {
    root: bool,
    active_grab: bool,
}

fn cursor_update_targets(
    last_root_cursor: Option<AltCursor>,
    active_grab: Option<crate::backend::x11::ActivePointerGrab>,
    requested: AltCursor,
) -> CursorUpdateTargets {
    CursorUpdateTargets {
        root: last_root_cursor != Some(requested),
        active_grab: active_grab.is_some_and(|grab| grab.cursor != requested),
    }
}

/// Project the requested cursor to the root window and any active pointer grab.
///
/// Active grabs own their cursor independently of the root window, so both
/// projections must remain synchronized with the shared requested style.
pub fn set_x11_cursor(
    x11: &X11BackendRef<'_>,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
) {
    let targets = cursor_update_targets(
        x11_runtime.last_x11_cursor,
        x11_runtime.active_pointer_grab,
        cursor,
    );
    if !targets.root && !targets.active_grab {
        return;
    }
    let conn = x11.conn;
    let root = x11_runtime.root;
    let cursor_index = cursor.to_x11_index();
    if let Some(Some(loaded_cursor)) = x11_runtime.cursors.get(cursor_index) {
        if targets.root {
            let _ = xproto::change_window_attributes(
                conn,
                root,
                &xproto::ChangeWindowAttributesAux::new().cursor(loaded_cursor.cursor as u32),
            );
            x11_runtime.last_x11_cursor = Some(cursor);
        }
        if let Some(grab) = x11_runtime.active_pointer_grab.as_mut()
            && targets.active_grab
        {
            let _ = conn.change_active_pointer_grab(
                loaded_cursor.cursor as u32,
                x11rb::CURRENT_TIME,
                grab.event_mask,
            );
            grab.cursor = cursor;
        }
        let _ = conn.flush();
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::{CursorUpdateTargets, cursor_update_targets};
    use crate::backend::x11::ActivePointerGrab;
    use crate::types::AltCursor;
    use x11rb::protocol::xproto::EventMask;

    fn active(cursor: AltCursor) -> ActivePointerGrab {
        ActivePointerGrab {
            event_mask: EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            cursor,
        }
    }

    #[test]
    fn active_grab_cursor_updates_even_when_root_already_matches() {
        assert_eq!(
            cursor_update_targets(
                Some(AltCursor::Move),
                Some(active(AltCursor::Default)),
                AltCursor::Move,
            ),
            CursorUpdateTargets {
                root: false,
                active_grab: true,
            }
        );
    }

    #[test]
    fn matching_root_and_grab_suppress_redundant_native_updates() {
        assert_eq!(
            cursor_update_targets(
                Some(AltCursor::Move),
                Some(active(AltCursor::Move)),
                AltCursor::Move,
            ),
            CursorUpdateTargets {
                root: false,
                active_grab: false,
            }
        );
    }
}

impl crate::backend::CursorOps for WmCtxX11<'_> {
    fn apply_cursor_style(&mut self, style: AltCursor) {
        set_x11_cursor(&self.x11, self.x11_runtime, style);
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

    let Some(grab_start_rect) = ctx.core.client_geo(win) else {
        return;
    };

    let mut state = MoveState {
        start_point: start,
        grab_start_rect,
        drop_restore_rect: float_restore_geo.unwrap_or(grab_start_rect),
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

    let release_modifiers = crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::Move,
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, .. } = event {
                let root = *root;
                ctx.core.drag_state_mut().record_interactive_motion(root);
                let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
                on_motion(&mut wm_ctx, win, root, root, &mut state);
            }
            true
        },
    )
    .unwrap_or(0);

    crate::mouse::drag::lifecycle::finish(ctx.core.drag_state_mut(), &ctx.x11, btn)
        .expect("X11 drag loop must finish the interaction using its grab button");
    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    crate::mouse::drag::finish_drag_move(
        &mut wm_ctx,
        win,
        state.drop_restore_rect,
        state.edge_snap_indicator,
        None,
        release_modifiers,
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
