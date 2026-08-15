//! Frame callbacks and primary-scanout bookkeeping.

use std::time::Duration;

use smithay::backend::renderer::element::{
    RenderElementStates, default_primary_scanout_output_compare,
};
use smithay::desktop::utils::{
    send_frames_surface_tree, surface_primary_scanout_output,
    update_surface_primary_scanout_output, with_surfaces_surface_tree,
};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::fractional_scale::with_fractional_scale;

use crate::backend::wayland::compositor::WaylandState;

// ─────────────────────────────────────────────────────────────────────────────
// Frame callbacks
// ─────────────────────────────────────────────────────────────────────────────

/// Send `wl_surface.frame` callbacks for windows visible on `output`.
///
/// Must be called once per rendered frame, after the buffer has been submitted
/// for scanout, so that clients know when to draw the next frame.
///
/// `Window::send_frame` owns surface-tree and popup traversal. Window/output
/// selection is done from current geometry rather than `Space`'s cached output
/// membership: commits can arrive before the next `Space::refresh`, especially
/// for short-lived Xwayland override-redirect windows.
pub fn send_frame_callbacks(state: &WaylandState, output: &Output, elapsed: Duration) {
    let throttle = output.current_mode().and_then(|mode| {
        let refresh = u64::try_from(mode.refresh).ok()?;
        (refresh > 0).then(|| Duration::from_nanos(1_000_000_000_000u64 / refresh))
    });

    if state.is_locked() {
        let output_name = output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            send_frames_surface_tree(
                lock_surface.wl_surface(),
                output,
                elapsed,
                throttle,
                surface_primary_scanout_output,
            );
        }
        send_auxiliary_surface_frame_callbacks(state, output, elapsed, throttle);
        return;
    }

    for window in state
        .space
        .elements()
        .filter(|window| window_overlaps_output(state, window, output))
    {
        window.send_frame(output, elapsed, throttle, surface_primary_scanout_output);
    }

    // Layer surfaces for this output only.
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.send_frame(output, elapsed, throttle, surface_primary_scanout_output);
    }

    send_auxiliary_surface_frame_callbacks(state, output, elapsed, throttle);
}

fn send_auxiliary_surface_frame_callbacks(
    state: &WaylandState,
    output: &Output,
    elapsed: Duration,
    throttle: Option<Duration>,
) {
    for_each_auxiliary_surface(state, |surface| {
        send_frames_surface_tree(
            surface,
            output,
            elapsed,
            throttle,
            surface_primary_scanout_output,
        );
    });
}

/// Update Smithay's primary-scanout bookkeeping for all surfaces visible on `output`.
///
/// `send_frames_surface_tree` and presentation feedback use this state to decide
/// which output should drive a surface's callbacks. If we never update it,
/// frame callbacks are throttled as if every surface were off-screen, which can
/// stall clients that rely on `wl_surface.frame`.
pub fn update_primary_scanout_output(
    state: &WaylandState,
    output: &Output,
    render_states: &RenderElementStates,
) {
    if state.is_locked() {
        let output_name = output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            with_surfaces_surface_tree(lock_surface.wl_surface(), |surface, data| {
                let _ = update_surface_primary_scanout_output(
                    surface,
                    output,
                    data,
                    None,
                    render_states,
                    default_primary_scanout_output_compare,
                );
                update_preferred_fractional_scale(surface, data);
            });
        }
        update_auxiliary_surface_primary_scanout(state, output, render_states);
        return;
    }

    for window in state
        .space
        .elements()
        .filter(|window| window_overlaps_output(state, window, output))
    {
        window.with_surfaces(|surface, data| {
            let _ = update_surface_primary_scanout_output(
                surface,
                output,
                data,
                None,
                render_states,
                default_primary_scanout_output_compare,
            );
            update_preferred_fractional_scale(surface, data);
        });
    }

    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.with_surfaces(|surface, data| {
            let _ = update_surface_primary_scanout_output(
                surface,
                output,
                data,
                None,
                render_states,
                default_primary_scanout_output_compare,
            );
            update_preferred_fractional_scale(surface, data);
        });
    }

    update_auxiliary_surface_primary_scanout(state, output, render_states);
}

fn update_auxiliary_surface_primary_scanout(
    state: &WaylandState,
    output: &Output,
    render_states: &RenderElementStates,
) {
    for_each_auxiliary_surface(state, |root| {
        with_surfaces_surface_tree(root, |surface, data| {
            let _ = update_surface_primary_scanout_output(
                surface,
                output,
                data,
                None,
                render_states,
                default_primary_scanout_output_compare,
            );
            update_preferred_fractional_scale(surface, data);
        });
    });
}

/// Visit compositor-rendered surfaces which are not part of a window or layer
/// tree. They still need the same frame and scanout servicing as ordinary
/// visible content or clients can stall while updating them.
fn for_each_auxiliary_surface(state: &WaylandState, mut visit: impl FnMut(&WlSurface)) {
    if let CursorImageStatus::Surface(surface) = &state.cursor_image_status {
        visit(surface);
    }
    if let Some(surface) = state.runtime.dnd_icon.as_ref() {
        visit(surface);
    }
}

fn update_preferred_fractional_scale(
    surface: &WlSurface,
    states: &smithay::wayland::compositor::SurfaceData,
) {
    let Some(output) = surface_primary_scanout_output(surface, states) else {
        return;
    };
    with_fractional_scale(states, |fractional_scale| {
        fractional_scale.set_preferred_scale(output.current_scale().fractional_scale());
    });
}

/// Test current compositor geometry instead of Smithay's lazily refreshed
/// element/output membership cache.
pub(crate) fn window_overlaps_output(
    state: &WaylandState,
    window: &smithay::desktop::Window,
    output: &Output,
) -> bool {
    let Some(output_rect) = state.space.output_geometry(output) else {
        return false;
    };
    let Some(location) = state.space.element_location(window) else {
        return false;
    };
    let mut window_rect = window.bbox_with_popups();
    window_rect.loc += location - window.geometry().loc;
    output_rect.overlaps(window_rect)
}
