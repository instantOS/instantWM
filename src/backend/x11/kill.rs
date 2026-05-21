//! X11-specific client kill helpers.

use crate::backend::x11::focus::send_event_x11;
use crate::contexts::WmCtxX11;
use crate::types::WindowId;
use x11rb::CURRENT_TIME;
use x11rb::protocol::xproto::{ConnectionExt, Window};

/// Attempt a graceful `WM_DELETE_WINDOW`, falling back to `XKillClient`.
pub fn force_close_x11(ctx_x11: &mut WmCtxX11<'_>, win: WindowId, wmatom_delete: u32) {
    let x11_win: Window = win.into();
    let sent = send_event_x11(
        &ctx_x11.x11,
        ctx_x11.x11_runtime,
        win,
        wmatom_delete,
        0,
        wmatom_delete as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    );

    if !sent {
        let _grab = crate::backend::x11::ServerGrab::new(ctx_x11.x11.conn);
        let _ = ctx_x11.x11.conn.kill_client(x11_win);
    }
}
