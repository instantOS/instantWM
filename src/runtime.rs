//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! These functions operate purely on [`Wm`] and are backend-agnostic.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction};

use crate::globals::LayoutWorkTargets;
use crate::wm::Wm;

// ── Event-loop tick helpers ─────────────────────────────────────────────

/// Backend-neutral scheduler options for a runtime tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickOptions {
    /// When true, defer non-urgent layout work while animations are active.
    pub defer_layout_while_animations_active: bool,
    /// Whether the backend currently has active window animations.
    pub animations_active: bool,
}

/// Result of a runtime tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickResult {
    pub ipc_handled: bool,
    pub monitor_config_applied: bool,
    pub layout_applied: bool,
    pub layout_deferred_for_animation: bool,
}

/// Shared per-tick housekeeping with backend-specific scheduler options.
///
/// Processing order is backend-independent and deterministic:
/// 1. IPC command dispatch
/// 2. monitor configuration work
/// 3. layout work
/// 4. backend-specific bar draw (X11 only)
pub fn event_loop_tick_with_options(
    wm: &mut Wm,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    options: TickOptions,
) -> TickResult {
    let status_handled = crate::bar::status::drain_internal_status_updates(wm);
    let ipc_handled = process_ipc_commands(ipc_server, wm);
    let work = process_pending_work(wm, options);

    draw_x11_bars_if_dirty(wm);
    TickResult {
        ipc_handled: ipc_handled || status_handled,
        monitor_config_applied: work.monitor_config_applied,
        layout_applied: work.layout_applied,
        layout_deferred_for_animation: work.layout_deferred_for_animation,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PendingWorkResult {
    pub monitor_config_applied: bool,
    pub layout_applied: bool,
    pub layout_deferred_for_animation: bool,
}

/// Apply all pending work in deterministic order.
pub fn process_pending_work(wm: &mut Wm, options: TickOptions) -> PendingWorkResult {
    let mut result = PendingWorkResult::default();

    if wm.g.pending.monitor_config {
        wm.g.pending.monitor_config = false;
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
        result.monitor_config_applied = true;
    }

    if !wm.g.pending.layout.is_pending() {
        return result;
    }

    if options.defer_layout_while_animations_active
        && options.animations_active
        && !wm.g.pending.layout.is_urgent()
    {
        result.layout_deferred_for_animation = true;
        return result;
    }

    let Some(targets) = wm.g.pending.layout.take_targets() else {
        return result;
    };
    result.layout_applied = apply_layout_targets(wm, targets);
    result
}

fn apply_layout_targets(wm: &mut Wm, targets: LayoutWorkTargets) -> bool {
    if wm.g.clients.is_empty() {
        return false;
    }

    match targets {
        LayoutWorkTargets::AllMonitors => {
            let mut ctx = wm.ctx();
            crate::layouts::arrange(&mut ctx, None);
            true
        }
        LayoutWorkTargets::Monitors(monitors) => {
            if monitors.is_empty() {
                return false;
            }
            for monitor_id in monitors {
                let mut ctx = wm.ctx();
                crate::layouts::arrange(&mut ctx, Some(monitor_id));
            }
            true
        }
    }
}

pub fn draw_x11_bars_if_dirty(wm: &mut Wm) {
    if !matches!(wm.backend, crate::backend::Backend::X11(_)) || !wm.bar.needs_redraw() {
        return;
    }

    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::X11(mut x11_ctx) = ctx {
        crate::bar::x11::draw_bars_x11(
            &mut x11_ctx.core,
            x11_ctx.x11_runtime,
            x11_ctx.systray.as_deref(),
        );
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

// ── Calloop source helpers ──────────────────────────────────────────────

/// Register an IPC listener fd as a calloop source.
///
/// The source simply wakes the event loop when a new connection arrives;
/// actual command processing is done by the caller via
/// [`process_ipc_commands`].
pub fn register_ipc_source<'loop_handle, T: 'static>(
    handle: &calloop::LoopHandle<'loop_handle, T>,
    ipc_server: &Option<crate::ipc::IpcServer>,
) {
    use std::os::unix::io::AsRawFd;
    if let Some(ref server) = *ipc_server {
        let ipc_fd = server.as_raw_fd();
        let ipc_source = Generic::new(
            unsafe { std::os::unix::io::BorrowedFd::borrow_raw(ipc_fd) },
            Interest::READ,
            Mode::Level,
        );
        handle
            .insert_source(ipc_source, |_, _, _| Ok(PostAction::Continue))
            .expect("failed to insert IPC fd source");
    }
}

/// On-demand animation timer guard shared by all backends.
///
/// Tracks whether a 16 ms animation timer is currently armed.  When the
/// timer fires and no animations remain it auto-drops; this flag is then
/// cleared so a new timer can be armed on the next animation start.
#[derive(Clone)]
pub struct AnimationTimerGuard {
    active: Rc<Cell<bool>>,
}

impl AnimationTimerGuard {
    pub fn new() -> Self {
        Self {
            active: Rc::new(Cell::new(false)),
        }
    }

    /// Arm the timer if animations are active and no timer is running.
    ///
    /// `has_animations` should reflect whether the backend currently has
    /// active window animations.  `on_tick` is called each time the timer
    /// fires (before the active-check) to let the backend mark outputs
    /// dirty, etc.
    pub fn ensure_armed<'loop_handle, T: 'static>(
        &self,
        has_animations: bool,
        handle: &calloop::LoopHandle<'loop_handle, T>,
        on_tick: impl Fn(&mut T) -> bool + 'static,
    ) {
        if !has_animations || self.active.get() {
            return;
        }
        self.active.set(true);
        let flag = Rc::clone(&self.active);
        let _ = handle.insert_source(
            Timer::from_duration(Duration::from_millis(16)),
            move |_, _, data| {
                let still_active = on_tick(data);
                if still_active {
                    TimeoutAction::ToDuration(Duration::from_millis(16))
                } else {
                    flag.set(false);
                    TimeoutAction::Drop
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{TickOptions, process_pending_work};
    use crate::backend::{Backend as WmBackend, wayland::WaylandBackend};
    use crate::types::MonitorId;
    use crate::wm::Wm;

    #[test]
    fn non_urgent_layout_can_be_deferred_for_animations() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.g.pending.layout.clear();
        wm.g.pending.layout.mark_monitor(MonitorId(0));

        let result = process_pending_work(
            &mut wm,
            TickOptions {
                defer_layout_while_animations_active: true,
                animations_active: true,
            },
        );

        assert!(result.layout_deferred_for_animation);
        assert!(wm.g.pending.layout.is_pending());
    }

    #[test]
    fn urgent_layout_bypasses_animation_defer() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.g.pending.layout.clear();
        wm.g.pending.layout.mark_monitor_urgent(MonitorId(0));

        let result = process_pending_work(
            &mut wm,
            TickOptions {
                defer_layout_while_animations_active: true,
                animations_active: true,
            },
        );

        assert!(!result.layout_deferred_for_animation);
        assert!(!wm.g.pending.layout.is_pending());
    }
}
