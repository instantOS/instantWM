//! Shared event loop tick logic for all Wayland backends.
//!
//! Both the winit (nested) and DRM (standalone) runtimes perform the same
//! set of per-tick housekeeping steps: layout arrangement, IPC command
//! dispatch, monitor configuration, and compositor space synchronisation.
//!
//! This module extracts those shared steps so each backend's event loop
//! only contains the minimal backend-specific match arms.

use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;

/// Arrange client layout when the dirty flag is set and no window
/// animations are in progress.
///
/// Shared between both winit and DRM event loops.
pub fn arrange_layout_if_dirty(wm: &mut Wm, state: &WaylandState) {
    if !wm.g.dirty.layout {
        return;
    }
    let mut ctx = wm.ctx();
    if !ctx.core().globals().clients.is_empty() && !state.has_active_window_animations() {
        ctx.core_mut().globals_mut().dirty.layout = false;
        let selected_monitor_id = ctx.core().globals().selected_monitor_id();
        crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
    }
}

/// Process pending IPC commands.
///
/// Returns `true` when at least one command was handled, so that
/// backend-specific code can react (e.g. DRM marks all outputs dirty).
pub fn process_ipc_commands(ipc_server: &mut Option<crate::ipc::IpcServer>, wm: &mut Wm) -> bool {
    let Some(server) = ipc_server.as_mut() else {
        return false;
    };
    let handled = server.process_pending(wm);
    if handled {
        wm.g.dirty.layout = true;
    }
    handled
}

/// Apply monitor configuration when the dirty flag is set.
pub fn apply_monitor_config_if_dirty(wm: &mut Wm) {
    if wm.g.dirty.monitor_config {
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
    }
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
