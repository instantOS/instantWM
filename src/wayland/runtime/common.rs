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
//! behaviour (animation guards, dirty-space propagation, etc.).

use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;

/// Arrange client layout when the dirty flag is set and no window
/// animations are in progress.
///
/// Delegates to the shared [`crate::runtime::arrange_layout_if_dirty`]
/// but additionally checks for active Wayland window animations.
pub fn arrange_layout_if_dirty(wm: &mut Wm, state: &WaylandState) {
    if wm.g.dirty.layout && !state.has_active_window_animations() {
        crate::runtime::arrange_layout_if_dirty(wm);
    }
}

/// Run the shared Wayland per-tick housekeeping.
///
/// Returns `true` when at least one IPC command was handled so the caller can
/// perform backend-specific invalidation.
pub fn event_loop_tick(
    wm: &mut Wm,
    state: &WaylandState,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) -> bool {
    arrange_layout_if_dirty(wm, state);
    let internal_status_handled = crate::bar::status::drain_internal_status_updates(wm);
    // IPC handlers own their own invalidation. Do not synthesize a generic
    // Wayland-specific dirtying policy here.
    let handled = crate::runtime::process_ipc_commands(ipc_server, wm);
    crate::runtime::apply_monitor_config_if_dirty(wm);
    handled || internal_status_handled
}

/// Synchronise the Smithay compositor space from WM globals when the
/// dirty flag is set.
///
/// Returns `true` when the space was dirty and got synchronised, so that
/// backend-specific code can react (e.g. DRM marks all outputs dirty).
pub fn sync_space_if_dirty(wm: &mut Wm, state: &mut WaylandState) -> bool {
    if wm.g.dirty.space {
        wm.g.dirty.space = false;
        state.sync_space_from_globals();
        true
    } else {
        false
    }
}

/// Run compositor-space sync and animation progression in one place.
///
/// Returns `true` when either the space was synchronized or at least one
/// animation tick was processed.
pub fn process_window_animations(wm: &mut Wm, state: &mut WaylandState) -> bool {
    let mut changed = sync_space_if_dirty(wm, state);
    if state.has_active_window_animations() {
        state.tick_window_animations();
        changed = true;
    }
    changed
}
