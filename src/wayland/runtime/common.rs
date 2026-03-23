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

/// Arrange client layout when the dirty flag is set and no window
/// animations are in progress.
///
/// Delegates to the shared [`crate::runtime::arrange_layout_if_dirty`]
/// but additionally checks for active Wayland window animations.
pub fn arrange_layout_if_dirty(state: &mut WaylandState) {
    if state.wm.g.dirty.layout && !state.has_active_window_animations() {
        crate::runtime::arrange_layout_if_dirty(&mut state.wm);
    }
}

/// Synchronise the Smithay compositor space from WM globals when the
/// dirty flag is set.
///
/// Returns `true` when the space was dirty and got synchronised, so that
/// backend-specific code can react (e.g. DRM marks all outputs dirty).
pub fn sync_space_if_dirty(state: &mut WaylandState) -> bool {
    if state.wm.g.dirty.space {
        state.wm.g.dirty.space = false;
        state.sync_space_from_globals();
        true
    } else {
        false
    }
}

/// Drain the WM command queue and execute each command on WaylandState.
///
/// Commands are queued by WM logic (e.g. `show_hide_wayland` pushes
/// MapWindow/UnmapWindow).  They must be executed before building render
/// elements so the Space reflects the correct visibility state.
pub fn drain_and_execute_ops(state: &mut WaylandState) {
    let ops = if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
        data.backend.drain_ops()
    } else {
        Vec::new()
    };
    for op in ops {
        state.execute_command(op);
    }
}
