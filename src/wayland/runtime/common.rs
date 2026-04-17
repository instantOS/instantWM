//! Shared event loop tick logic for all Wayland backends.
//!
//! Both the winit (nested) and DRM (standalone) runtimes perform the same
//! set of per-tick housekeeping steps: layout arrangement, IPC command
//! dispatch, monitor configuration, and compositor space synchronisation.
//!
//! This module extracts those shared steps so each backend's event loop
//! only contains the minimal backend-specific match arms.
//!
//! Most helpers delegate to [`crate::runtime`] and add Wayland-specific
//! behaviour (animation guards, compositor-space sync, etc.).

use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;

/// Run the shared Wayland per-tick housekeeping and return detailed outcome.
pub fn event_loop_tick(
    wm: &mut Wm,
    state: &WaylandState,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) -> crate::runtime::TickResult {
    crate::runtime::event_loop_tick_with_options(
        wm,
        ipc_server,
        crate::runtime::TickOptions {
            defer_layout_while_animations_active: true,
            animations_active: state.has_active_window_animations(),
        },
    )
}

/// Run compositor-space sync and animation progression in one place.
///
/// Returns `true` when either the space was synchronized or at least one
/// animation tick was processed.
pub fn process_window_animations(state: &mut WaylandState) -> bool {
    let mut changed = false;
    if state.take_space_sync_pending() {
        state.sync_space_from_globals();
        changed = true;
    }
    if state.has_active_window_animations() {
        state.tick_window_animations();
        changed = true;
    }
    changed
}
