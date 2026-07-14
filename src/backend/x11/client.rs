//! X11 client backend helpers.

use crate::backend::x11::X11BackendRef;
use crate::model::WmModel;
use crate::types::WindowId;
use x11rb::properties::WmSizeHints;

/// Read `WM_NORMAL_HINTS` from the X server and populate the client's size hints,
/// `min_aspect`, `max_aspect`, and fixed-size state.
///
/// The parsed hints are returned so initial management can distinguish an
/// explicit USPosition/PPosition from compositor-owned placement.
pub fn update_size_hints(
    model: &mut WmModel,
    x11: &X11BackendRef,
    win: WindowId,
) -> Option<WmSizeHints> {
    let hints = match WmSizeHints::get_normal_hints(x11.conn, win.into()) {
        Ok(cookie) => match cookie.reply_unchecked() {
            Ok(hints) => hints,
            Err(_) => None,
        },
        Err(_) => None,
    };
    let Some(c) = model.client_mut(win) else {
        return hints;
    };
    crate::backend::x11::policy::apply_size_hints_to_client(c, hints);
    hints
}
