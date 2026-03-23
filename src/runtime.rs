//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! These functions operate on [`WmCtx`] and are backend-agnostic.

use crate::contexts::{CoreCtx, WmCtx};
use crate::wm::Wm;

// ── Event-loop tick helpers ─────────────────────────────────────────────

/// Arrange client layout when the dirty flag is set.
///
/// Used by the X11 event loop (which previously called `arrange()` directly
/// from event handlers) and by the Wayland event loop (which may add an
/// additional animation guard on top).
pub fn arrange_layout_if_dirty(ctx: &mut WmCtx) {
    if !ctx.core().globals().dirty.layout {
        return;
    }
    if ctx.core().globals().clients.is_empty() {
        return;
    }
    let monitor_id = ctx.core().globals().selected_monitor_id();
    crate::layouts::arrange(ctx, Some(monitor_id));
}

/// Apply monitor configuration when the dirty flag is set.
pub fn apply_monitor_config_if_dirty(ctx: &mut WmCtx) {
    if ctx.core().globals().dirty.monitor_config {
        crate::monitor::apply_monitor_config(ctx);
    }
}

// ── Startup helpers ─────────────────────────────────────────────────────

/// Spawn the configured status bar command, or the built-in default.
pub fn spawn_status_bar(core: &CoreCtx) {
    if let Some(ref cmd) = core.globals().cfg.status_command {
        crate::bar::status::spawn_status_command(cmd.as_str());
    } else {
        crate::bar::status::spawn_default_status();
    }
}

/// Late startup sequence shared by all backends.
///
/// Runs autostart, binds the IPC socket, and spawns the status bar.
/// Each backend calls this before entering its event loop.
pub fn late_init(wm: &mut Wm) -> Option<crate::ipc::IpcServer> {
    crate::startup::autostart::run_autostart();
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    let ctx = wm.ctx();
    let core = ctx.core();
    spawn_status_bar(&core);
    ipc_server
}
