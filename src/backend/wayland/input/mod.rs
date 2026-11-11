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
mod modifiers;
pub mod pointer;
pub mod touch;

// Re-export public APIs
pub use keyboard::{handle_keyboard, handle_virtual_keyboard_event};
pub use modifiers::modifiers_to_x11_mask;
pub use pointer::handle_virtual_pointer_event;

use crate::backend::wayland::output::clamp_output_size;
use crate::monitor::refresh_monitor_layout;
use crate::types::Size;
use smithay::output::{Mode as OutputMode, Output};
use smithay::utils::Transform;

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
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<
        crate::backend::wayland::compositor::WaylandState,
    >,
) -> bool {
    let Some(target) = state.native.take_pending_warp() else {
        return false;
    };

    crate::backend::wayland::input::pointer::motion::process_pointer_motion_command(
        state,
        pointer_handle,
        keyboard_handle,
        crate::backend::wayland::commands::PointerMotionCommand::Warp {
            x: target.x,
            y: target.y,
            time: smithay::backend::input::InputTime::now(),
        },
    );

    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Resize helper (winit-only — output size comes from the backend window)
// ─────────────────────────────────────────────────────────────────────────────

/// Apply a new output mode for the nested host window.
///
/// Callers screen degenerate sizes before getting here (a minimized host
/// window reports 0x0 and keeps its previous mode), so the floor is only a
/// last-resort invariant: Smithay must never see a zero-sized mode.
pub fn handle_resize(
    state: &mut crate::backend::wayland::compositor::WaylandState,
    output: &Output,
    size: Size,
) {
    let safe_size = clamp_output_size(size);
    let mode = OutputMode {
        size: (safe_size.w, safe_size.h).into(),
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
        .native
        .space
        .output_geometry(output)
        .map(|geo| geo.loc)
        .unwrap_or_default();
    state.native.space.map_output(output, output_loc);
    state
        .native
        .output_management_state
        .update_heads::<crate::backend::wayland::compositor::WaylandState>(std::iter::once(output));
    state.native.configure_output_surfaces(output);

    refresh_monitor_layout(&mut state.ctx());
    // `refresh_monitor_layout` resets each monitor's `available_rect` back to
    // its full output rect, so re-apply the layer-shell exclusive zones.
    let _ = crate::backend::wayland::compositor::layer_shell::apply_available_rects(state);
    state.wm.core.work.layout.mark_all_urgent();
    state.native.request_space_sync();
    recover_pointer_after_output_change(state);
}

/// A topology change can leave the pointer on an output that no longer exists.
/// Recover immediately, including for explicit/client-managed layouts with gaps.
pub(crate) fn recover_pointer_after_output_change(
    state: &mut crate::backend::wayland::compositor::WaylandState,
) {
    if state
        .native
        .space
        .output_under(state.native.runtime.pointer_location)
        .next()
        .is_some()
    {
        return;
    }
    let target = state
        .wm
        .core
        .state
        .model
        .selected_monitor()
        .and_then(|monitor| {
            state
                .native
                .space
                .outputs()
                .find(|output| output.name() == monitor.name)
        })
        .or_else(|| state.native.space.outputs().next());
    let center = target
        .and_then(|output| state.native.space.output_geometry(output))
        .map(|geometry| {
            (
                geometry.loc.x as f64 + geometry.size.w as f64 / 2.0,
                geometry.loc.y as f64 + geometry.size.h as f64 / 2.0,
            )
        });
    if let Some((x, y)) = center {
        state.native.request_warp(x, y);
    }
}
