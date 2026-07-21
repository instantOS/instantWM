//! X11-specific geometry helpers.

use crate::backend::x11::X11BackendRef;
use crate::model::WmModel;
use crate::types::{Rect, WindowId};

/// Apply ICCCM size hints for an X11 client.
pub fn apply_icccm_size_hints(
    model: &mut WmModel,
    x11: &X11BackendRef,
    win: WindowId,
    geo: &mut Rect,
) {
    let needs_update = model
        .client(win)
        .map(|c| !c.size_hints_valid)
        .unwrap_or(false);

    if needs_update {
        let _ = crate::backend::x11::client::update_size_hints(model, x11, win);
    }

    let client = match model.client(win) {
        Some(c) => c,
        None => return,
    };

    let constrained =
        client
            .size_hints
            .constrain_size(geo.size(), client.min_aspect, client.max_aspect);
    *geo = geo.with_size(constrained);
}
