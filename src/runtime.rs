//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! These functions operate purely on [`Wm`] and are backend-agnostic.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction};

use crate::wm::Wm;

// ── Event-loop tick helpers ─────────────────────────────────────────────

/// Shared per-tick housekeeping: process IPC, apply monitor config, arrange
/// layout.  Returns `true` when at least one IPC command was handled.
///
/// Backend-specific work (rendering, space sync, event draining, flushing)
/// should be done by the caller before/after this function.
pub fn event_loop_tick(wm: &mut Wm, ipc_server: &mut Option<crate::ipc::IpcServer>) -> bool {
    if wm.bar.poll_async_status(&wm.g.bar_runtime.status_text) {
        wm.bar.mark_dirty();
    }

    let handled = process_ipc_commands(ipc_server, wm);
    apply_monitor_config_if_dirty(wm);
    arrange_layout_if_dirty(wm);
    draw_x11_bars_if_dirty(wm);
    handled
}

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
