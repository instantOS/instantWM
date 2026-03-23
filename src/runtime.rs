//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! These functions operate purely on [`Wm`] and are backend-agnostic.

use crate::wm::Wm;

// ── Event-loop tick helpers ─────────────────────────────────────────────

/// Arrange client layout when the dirty flag is set.
///
/// Used by the X11 event loop (which previously called `arrange()` directly
/// from event handlers) and by the Wayland event loop (which may add an
/// additional animation guard on top).
pub fn arrange_layout_if_dirty(wm: &mut Wm) {
    if !wm.g.dirty.layout {
        return;
    }
    if wm.g.clients.is_empty() {
        return;
    }
    let mut ctx = wm.ctx();
    let monitor_id = ctx.core().globals().selected_monitor_id();
    crate::layouts::arrange(&mut ctx, Some(monitor_id));
}

/// Apply monitor configuration when the dirty flag is set.
pub fn apply_monitor_config_if_dirty(wm: &mut Wm) {
    if wm.g.dirty.monitor_config {
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
    }
}

// ── Startup helpers ─────────────────────────────────────────────────────

/// Initialise the keyboard layout from the WM configuration.
pub fn init_keyboard_layout(wm: &mut Wm) {
    let mut ctx = wm.ctx();
    crate::keyboard_layout::init_keyboard_layout(&mut ctx);
}

/// Spawn the configured status bar command, or the built-in default.
pub fn spawn_status_bar(wm: &Wm) {
    if let Some(ref cmd) = wm.g.cfg.status_command {
        crate::bar::status::spawn_status_command(cmd);
    } else {
        crate::bar::status::spawn_default_status();
    }
}

/// Late startup sequence shared by all backends.
///
/// Runs autostart, binds the IPC socket, and spawns the status bar.
/// Each backend calls this before entering its event loop.
pub fn late_init(wm: &Wm) -> Option<crate::ipc::IpcServer> {
    crate::startup::autostart::run_autostart();
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    spawn_status_bar(wm);
    ipc_server
}
