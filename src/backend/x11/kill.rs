//! X11-specific client kill helpers.

use crate::backend::x11::focus::send_event;
use crate::contexts::WmCtxX11;
use crate::types::WindowId;
use x11rb::CURRENT_TIME;
use x11rb::protocol::xproto::{ConnectionExt, Window};

/// Attempt a graceful `WM_DELETE_WINDOW`, falling back to `XKillClient`.
pub fn force_close(ctx_x11: &mut WmCtxX11<'_>, win: WindowId, wmatom_delete: u32) {
    let x11_win: Window = win.into();
    let mut sent = send_event(
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

    let mut protocols_known = ctx_x11
        .x11_runtime
        .client_protocols
        .get(&win)
        .is_some_and(|protocols| protocols.is_known());
    if !sent && !protocols_known {
        let protocols = crate::backend::x11::focus::read_wm_protocols(
            ctx_x11.x11.conn,
            x11_win,
            ctx_x11.x11_runtime.wmatom.protocols,
        )
        .map(crate::backend::x11::X11ClientProtocols::Known)
        .unwrap_or_default();
        ctx_x11.x11_runtime.client_protocols.insert(win, protocols);
        protocols_known = ctx_x11
            .x11_runtime
            .client_protocols
            .get(&win)
            .is_some_and(|protocols| protocols.is_known());
        sent = send_event(
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
    }

    if should_xkill_client(sent, protocols_known) {
        let _grab = crate::backend::x11::ServerGrab::new(ctx_x11.x11.conn);
        let _ = ctx_x11.x11.conn.kill_client(x11_win);
    } else if !sent {
        log::warn!(
            "refusing to XKillClient window {:?}: WM_PROTOCOLS could not be read",
            win
        );
    }
}

fn should_xkill_client(delete_sent: bool, protocols_known: bool) -> bool {
    !delete_sent && protocols_known
}

#[cfg(test)]
mod tests {
    use super::should_xkill_client;

    #[test]
    fn unreadable_protocols_never_authorize_destructive_fallback() {
        assert!(!should_xkill_client(false, false));
        assert!(!should_xkill_client(true, false));
        assert!(!should_xkill_client(true, true));
        assert!(should_xkill_client(false, true));
    }
}

impl crate::backend::WindowCloseOps for WmCtxX11<'_> {
    fn close_window(&mut self, window: WindowId) {
        let delete_atom = self.x11_runtime.wmatom.delete;
        force_close(self, window, delete_atom);
    }
}
