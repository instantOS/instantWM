//! X11 client lifecycle entry points.
//!
//! These are backend-scoped wrappers around the client lifecycle implementation.

use crate::contexts::WmCtxX11;
use crate::types::{Rect, WindowId};

pub fn manage(ctx: &mut WmCtxX11, w: WindowId, wa_geo: Rect, wa_border_width: u32) {
    crate::client::lifecycle::manage_x11(ctx, w, wa_geo, wa_border_width);
}

pub fn unmanage(ctx: &mut WmCtxX11, win: WindowId, destroyed: bool) {
    crate::client::lifecycle::unmanage_x11(ctx, win, destroyed);
}
