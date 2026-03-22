//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! These functions operate purely on [`Wm`] and are backend-agnostic.

use crate::wm::Wm;

/// Apply monitor configuration when the dirty flag is set.
pub fn apply_monitor_config_if_dirty(wm: &mut Wm) {
    if wm.g.dirty.monitor_config {
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
    }
}

/// Process pending IPC commands.
///
/// Returns `true` when at least one command was handled.
pub fn process_ipc_commands(ipc_server: &mut Option<crate::ipc::IpcServer>, wm: &mut Wm) -> bool {
    let Some(server) = ipc_server.as_mut() else {
        return false;
    };
    server.process_pending(wm)
}

/// Spawn the configured status bar command, or the built-in default.
pub fn spawn_status_bar(wm: &Wm) {
    if let Some(ref cmd) = wm.g.cfg.status_command {
        crate::bar::status::spawn_status_command(cmd);
    } else {
        crate::bar::status::spawn_default_status();
    }
}
