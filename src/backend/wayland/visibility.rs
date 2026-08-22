//! Wayland client visibility: map/unmap-based concealment.
//!
//! X11 parks invisible windows mapped but off-screen so their content stays
//! alive; under Wayland the compositor owns surfaces, so visibility is
//! expressed directly through map/unmap. The shared policy lives in
//! [`crate::client::visibility`]; this module only projects it onto
//! compositor state.

use crate::backend::WindowOps;
use crate::contexts::WmCtxWayland;
use crate::types::WindowId;

pub(crate) fn apply_visibility(ctx: &mut WmCtxWayland<'_>) {
    let globals = ctx.core.state();
    let pending_spawns = &ctx.core.pending_work().spawn_animations;
    for entry in crate::client::visibility::visibility_plan(&globals.model) {
        // Newly spawned windows (pending their first layout) are intentionally
        // left unmapped here.  They are mapped at their layout-allocated rect
        // after arrange runs, so the client never appears at its initial
        // buffer size before the tiling layout resizes it.
        if entry.visible && !pending_spawns.contains(&entry.win) {
            ctx.wayland.map_window(entry.win);
        } else {
            ctx.wayland.unmap_window(entry.win);
        }
    }
}

pub(crate) fn hide(ctx: &mut WmCtxWayland<'_>, win: WindowId) {
    ctx.wayland.unmap_window(win);
    ctx.wayland.flush();
}
