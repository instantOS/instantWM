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

/// Process pending IPC commands (Wayland wrapper).
///
/// Delegates to the shared [`crate::runtime::process_ipc_commands`] and
/// additionally marks the layout dirty so the Wayland event loop re-arranges.
///
/// Returns `true` when at least one command was handled, so that
/// backend-specific code can react (e.g. DRM marks all outputs dirty).
pub fn process_ipc_commands(ipc_server: &mut Option<crate::ipc::IpcServer>, wm: &mut Wm) -> bool {
    let handled = crate::runtime::process_ipc_commands(ipc_server, wm);
    if handled {
        wm.g.dirty.layout = true;
    }
    handled
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
