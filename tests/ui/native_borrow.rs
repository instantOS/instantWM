//! Deliberately invalid programs compiled by tests/borrow_contract.py.
use crate::backend::WindowOps;
use crate::backend::wayland::{WaylandBackend, compositor::WaylandState};

fn state_cannot_outlive_exclusive_capability(state: &mut WaylandState) {
    let mut backend = WaylandBackend::new(state);
    state.native.request_render();
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

fn protocol_dispatch_cannot_overlap_model_access(state: &mut WaylandState) {
    let wm = &mut state.wm;
    state.dispatch_pending_commits();
    wm.core.quit();
}

fn renderer_cannot_be_borrowed_twice(
    graphics: &mut crate::backend::wayland::compositor::graphics::Graphics,
) {
    let first = &mut *graphics;
    graphics.with_renderer(|_| ());
    first.with_renderer(|_| ());
}

// Valid field splits must compile alongside the rejected programs.
fn model_and_scene_borrows_are_disjoint(state: &mut WaylandState) {
    state.native.tick_animations(&state.wm.core.state);
}

fn renderer_and_scene_borrows_are_disjoint(state: &mut WaylandState) {
    if let Some(graphics) = state.graphics.as_mut() {
        graphics.with_renderer(|_| state.native.space.refresh());
    }
}

// Explicit field access must preserve the same owner borrow boundaries.
fn model_field_cannot_overlap_native_effects(ctx: &mut crate::contexts::WmCtx<'_>) {
    let model = &ctx.core().state.model;
    ctx.raise_client(crate::types::WindowId(1));
    model.selected_win();
}

fn shared_conversion_cannot_overlap_typed_backend(ctx: &mut crate::contexts::WmCtxX11<'_>) {
    let mut shared = ctx.wm_ctx();
    ctx.core.quit();
    shared.raise_client(crate::types::WindowId(1));
}

// Pure queries can run without any compositor or backend capability.
fn listings_only_need_policy_data(
    model: &crate::model::WmModel,
    config: &crate::core_state::EffectiveConfig,
) {
    crate::ipc::layout::list_layouts(model);
    crate::ipc::theme::get_theme(config);
    crate::ipc::keybinds::list_keybinds(&config.bindings);
}
