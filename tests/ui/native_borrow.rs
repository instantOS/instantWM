//! Deliberately invalid programs compiled by tests/borrow_contract.py.
use crate::backend::WindowOps;
use crate::backend::wayland::{WaylandBackend, compositor::WaylandState};

fn state_cannot_outlive_exclusive_capability(state: &mut WaylandState) {
    let mut backend = WaylandBackend::new(state);
    state.request_render();
    backend.request_render();
}

fn effects_cannot_reenter_capability(state: &mut WaylandState) {
    let mut backend = WaylandBackend::new(state);
    let second = &mut backend;
    backend.request_render();
    second.request_render();
}

fn queries_cannot_overlap_effects(state: &mut WaylandState) {
    let mut backend = WaylandBackend::new(state);
    let query = &backend;
    backend.request_render();
    query.window_protocol(crate::types::WindowId(1));
}

fn backend_identity_cannot_be_mismatched(wm: &mut crate::wm::WaylandWm) {
    wm.x11_ctx();
}

fn shared_context_effects_require_mutable_access(ctx: &crate::contexts::WmCtx<'_>) {
    ctx.resize_window(
        crate::types::WindowId(1),
        crate::types::Rect::new(0, 0, 10, 10),
    );
}
