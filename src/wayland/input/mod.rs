//! Input event handlers for the Wayland compositor backends.
//!
//! The keyboard, pointer-button, and pointer-axis handlers are generic over
//! the Smithay `InputBackend` type so that they can be shared between the
//! nested (winit) backend and the standalone DRM/libinput backend.
//!
//! Pointer motion uses a unified handler that accepts absolute coordinates,
//! relative deltas, or direct position updates (for warps).

pub mod bar;
pub mod drm;

pub mod keyboard;
pub mod pointer;

// Re-export public APIs
pub use keyboard::handle_keyboard;
pub use pointer::{
    handle_pointer_axis, handle_pointer_button, handle_pointer_motion,
    motion_event_from_libinput_absolute, motion_event_from_libinput_relative,
    motion_event_from_winit,
};

use crate::monitor::update_geom;
use crate::wm::Wm;
use smithay::desktop::layer_map_for_output;
use smithay::output::{Mode as OutputMode, Output};
use smithay::utils::{SERIAL_COUNTER, Transform};

// ─────────────────────────────────────────────────────────────────────────────
// Pending warp — compositor-side cursor teleport
// ─────────────────────────────────────────────────────────────────────────────

/// Consume any pending warp stored in `WaylandState` and synthesise a full
/// Smithay pointer-motion event so that:
///
/// 1. The pointer location in WaylandState is updated to the new position.
/// 2. `pointer_handle.motion()` is called, which sends `wl_pointer::enter`
///    / `wl_pointer::motion` / `wl_pointer::leave` to the right clients and
///    updates the internal Smithay focus.
/// 3. `pointer_handle.frame()` closes the event batch.
///
/// Call this once per event-loop tick, *before* rendering, so the rendered
/// cursor position matches the pointer protocol state.
///
/// Returns `true` when a warp was applied (callers may wish to mark output
/// dirty so the new cursor position is painted immediately).
pub fn apply_pending_warp(
    state: &mut crate::backend::wayland::compositor::WaylandState,
    pointer_handle: &smithay::input::pointer::PointerHandle<
        crate::backend::wayland::compositor::WaylandState,
    >,
) -> bool {
    use crate::backend::wayland::compositor::PointerFocusTarget;
    use smithay::utils::{Clock, Monotonic};

    let Some(target) = state.take_pending_warp() else {
        return false;
    };

    state.runtime.pointer_location = target;

    let focus = state
        .layer_surface_under_pointer(target)
        .or_else(|| state.surface_under_pointer(target))
        .map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));

    let serial = SERIAL_COUNTER.next_serial();
    let time_msec = Clock::<Monotonic>::new().now().as_millis();
    let motion = smithay::input::pointer::MotionEvent {
        location: target,
        serial,
        time: time_msec,
    };

    pointer_handle.motion(state, focus, &motion);
    pointer_handle.frame(state);

    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Resize helper (winit-only — output size comes from the backend window)
// ─────────────────────────────────────────────────────────────────────────────

pub fn handle_resize(
    wm: &mut Wm,
    state: &mut crate::backend::wayland::compositor::WaylandState,
    output: &Output,
    w: i32,
    h: i32,
) {
    let (safe_w, safe_h) = crate::wayland::common::sanitize_wayland_size(w, h);
    let mode = OutputMode {
        size: (safe_w, safe_h).into(),
        refresh: 60_000,
    };
    // Transform::Flipped180 is REQUIRED for the winit (nested) backend.
    //
    // Smithay's winit backend renders into an OpenGL framebuffer whose
    // Y-axis points upward (OpenGL convention), but the host Wayland
    // compositor expects the top-left origin (Wayland convention).  The
    // result is that every frame arrives at the host upside-down unless
    // we tell Smithay's output machinery to compensate with a 180° flip.
    //
    // DO NOT replace this with Transform::Normal — the entire compositor
    // output will be rendered upside-down inside the host window.
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    let output_loc = state
        .space
        .output_geometry(output)
        .map(|geo| geo.loc)
        .unwrap_or_default();
    state.space.map_output(output, output_loc);
    layer_map_for_output(output).arrange();

    wm.g.cfg.screen_width = safe_w;
    wm.g.cfg.screen_height = safe_h;
    update_geom(&mut wm.ctx());
    wm.g.queue_layout_for_all_monitors_urgent();
    state.request_space_sync();
}
