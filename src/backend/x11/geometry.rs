//! X11-specific geometry helpers.

use crate::backend::x11::X11BackendRef;
use crate::client::geometry::ClientSizeConstraints;
use crate::model::WmModel;
use crate::types::{Rect, WindowId};

/// Apply ICCCM size hints for an X11 client.
///
/// The caller passes the constraints it already resolved from the model. Only
/// a stale hint set forces a round-trip to the X server, and the refreshed
/// constraints are re-read once after that update.
pub fn apply_icccm_size_hints(
    model: &mut WmModel,
    x11: &X11BackendRef,
    win: WindowId,
    hints: &ClientSizeConstraints,
    geo: &mut Rect,
) {
    if hints.hints_valid {
        *geo = hints.constrain(*geo);
        return;
    }

    let _ = crate::backend::x11::client::update_size_hints(model, x11, win);
    if let Some(client) = model.client(win) {
        *geo = ClientSizeConstraints::of(client).constrain(*geo);
    }
}
