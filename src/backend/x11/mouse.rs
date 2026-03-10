//! X11 mouse backend helpers.

use crate::contexts::WmCtxX11;
use crate::mouse::drag::{
    clear_bar_hover, complete_move_drop, on_motion, prepare_drag_target, MoveState,
};
use crate::mouse::warp::get_root_ptr_ctx_x11;
use crate::types::{MouseButton, Rect};

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

    let Some((start_x, start_y)) = get_root_ptr_ctx_x11(ctx) else {
        return;
    };

    // Use override from title drag if available (preserves pre-drag floating dimensions),
    // otherwise get the current client geometry.
    let grab_start_rect = float_restore_geo
        .or_else(|| ctx.core.client(win).map(|c| c.geo))
        .unwrap_or(Rect::default());

    let mut state = MoveState {
        start_x,
        start_y,
        grab_start_rect,
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };

    crate::mouse::grab::mouse_drag_loop(ctx, btn, 2, false, |ctx, event| {
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
