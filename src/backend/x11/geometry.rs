//! X11-specific geometry helpers.

use crate::backend::x11::X11BackendRef;
use crate::contexts::CoreCtx;
use crate::types::{Rect, WindowId};

/// Apply ICCCM size hints for an X11 client.
pub fn apply_icccm_size_hints_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    win: WindowId,
    geo: &mut Rect,
) {
    let needs_update = core
        .client(win)
        .map(|c| !c.size_hints_dirty)
        .unwrap_or(false);

    if needs_update {
        crate::backend::x11::client::update_size_hints_x11(core, x11, win);
    }

    let client = match core.client(win) {
        Some(c) => c,
        None => return,
    };

    let (w, h) =
        client
            .size_hints
            .constrain_size(geo.w, geo.h, client.min_aspect, client.max_aspect);
    geo.w = w;
    geo.h = h;
}
