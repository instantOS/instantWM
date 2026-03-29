//! X11 client backend helpers.

use crate::backend::x11::X11BackendRef;
use crate::contexts::CoreCtx;
use crate::types::WindowId;
use x11rb::properties::WmSizeHints;

/// Read `WM_NORMAL_HINTS` from the X server and populate the client's size hints,
/// `min_aspect`, `max_aspect`, and `isfixed`.
pub fn update_size_hints_x11(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId) {
    let hints = match WmSizeHints::get_normal_hints(x11.conn, win.into()) {
        Ok(cookie) => match cookie.reply_unchecked() {
            Ok(hints) => hints,
            Err(_) => None,
        },
        Err(_) => None,
    };
    let Some(c) = core.globals_mut().clients.get_mut(&win) else {
        return;
    };
    crate::client::x11_policy::apply_size_hints_to_client(c, hints);
}
