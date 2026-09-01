//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! These functions operate purely on [`Wm`] and are backend-agnostic.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction};

use crate::backend::WindowOps;
use crate::core_state::LayoutWorkTargets;
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
    /// StatusNotifier tray content changed; bar redraw / render required.
    pub systray_updated: bool,
}

/// Shared per-tick housekeeping with backend-specific scheduler options.
///
/// Processing order is backend-independent and deterministic:
/// 1. StatusNotifier tray events (incl. the external instantMENU tray-menu
///    host, which reconciles against the drained session state)
/// 2. internal status updates
/// 3. IPC command dispatch
/// 4. monitor configuration work
/// 5. layout work
/// 6. dirty-bar redraw (backend-routed)
pub fn event_loop_tick_with_options(
    wm: &mut Wm,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    options: TickOptions,
) -> TickResult {
    let systray_updated = wm.poll_systray();
    if crate::systray::instantmenu::drive_instantmenu_menu(wm) {
        wm.bar.mark_dirty();
    }
    let status_handled = crate::bar::status::drain_internal_status_updates(wm);
    // A finished region selection may resize a window, so it drains before
    // pending work to let the same tick apply the resulting layout.
    let region_selection_applied = crate::mouse::slop::drain_region_selection(wm);
    let ipc_handled = process_ipc_commands(ipc_server, wm);
    let work = process_pending_work(wm, options);
    crate::bar::status::sync_visibility(wm);

    {
        let mut ctx = wm.ctx();
        ctx.redraw_bars_if_dirty();
    }
    TickResult {
        ipc_handled: ipc_handled || status_handled || region_selection_applied,
        monitor_config_applied: work.monitor_config_applied,
        layout_applied: work.layout_applied,
        systray_updated,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PendingWorkResult {
    pub monitor_config_applied: bool,
    pub layout_applied: bool,
}

/// Apply all pending work in deterministic order.
pub fn process_pending_work(wm: &mut Wm, options: TickOptions) -> PendingWorkResult {
    let mut result = PendingWorkResult::default();

    if wm.work.monitor_config {
        wm.work.monitor_config = false;
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
        result.monitor_config_applied = true;
    }

    // Edge scratchpads finish their slide-out through backend animation
    // bookkeeping; complete the deferred logical hide once it drained.
    let pending_hides = wm.work.pending_scratchpad_hide_windows();
    let finished_hides: Vec<crate::types::WindowId> = pending_hides
        .into_iter()
        .filter(|win| !wm.backend.window_animation_active(*win))
        .collect();
    for win in &finished_hides {
        wm.work.cancel_pending_scratchpad_hide(*win);
    }
    if !finished_hides.is_empty() {
        let mut ctx = wm.ctx();
        crate::floating::finish_scratchpad_hides(&mut ctx, &finished_hides);
    }

    if !wm.work.layout.is_pending() {
        return result;
    }

    if options.defer_layout_while_animations_active
        && options.animations_active
        && !wm.work.layout.is_urgent()
    {
        return result;
    }

    let Some(targets) = wm.work.layout.take_targets() else {
        return result;
    };
    result.layout_applied = apply_layout_targets(wm, targets);
    result
}

fn apply_layout_targets(wm: &mut Wm, targets: LayoutWorkTargets) -> bool {
    if wm.core.model.clients.is_empty() {
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

/// Spawn the configured status bar command, the auto-detected
/// `i3status-rs`, or the built-in default (in that order of
/// precedence).
pub fn spawn_status_bar(wm: &Wm) {
    crate::bar::status::sync_visibility(wm);
    if let Some(ref cmd) = wm.core.config.status_command {
        crate::bar::status::spawn_status_command(cmd);
    } else if crate::bar::status::is_i3status_rs_available() {
        crate::bar::status::spawn_status_command("i3status-rs");
    } else {
        crate::bar::status::spawn_default_status();
    }
}

/// Run autostart, user-defined `exec_once` and `exec` commands.
///
/// Called by each backend during startup. The Wayland backends call this
/// from [`autostart_ipc_status_ping`], while X11 calls it from
/// [`late_init_x11`].
pub fn run_startup_commands(wm: &Wm) {
    crate::startup::autostart::run_autostart();
    crate::startup::autostart::run_exec_commands(&wm.core.config.exec_once);
    crate::startup::autostart::run_exec_commands(&wm.core.config.exec);
}

/// X11 late startup sequence.
///
/// Binds the IPC socket first so startup commands — including `ins autostart`,
/// which applies the wallpaper through `instantwmctl wallpaper` — can reach
/// the compositor immediately, then runs them and spawns the status bar.
/// The StatusNotifier worker starts later, from the calloop event loop, so it
/// can receive a wake ping; see `backend::x11::events::run`.
pub fn late_init_x11(wm: &mut Wm) -> Option<crate::ipc::IpcServer> {
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    run_startup_commands(wm);
    spawn_status_bar(wm);
    ipc_server
}

// ── Calloop source helpers ──────────────────────────────────────────────

/// Register a no-op ping source and return its handle.
///
/// Cross-thread producers — currently the StatusNotifier worker — ping it to
/// wake an otherwise idle event loop so polled state is drained promptly.
pub fn make_wake_ping<T: 'static>(
    handle: &calloop::LoopHandle<'_, T>,
) -> Option<calloop::ping::Ping> {
    let (ping, source) = calloop::ping::make_ping().ok()?;
    handle.insert_source(source, |_, _, _| {}).ok()?;
    Some(ping)
}

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
/// Tracks whether an animation timer is currently armed. When the timer fires
/// and no animations remain it auto-drops; this flag is then cleared so a new
/// timer can be armed on the next animation start. Backends may select a frame
/// interval, while the default API retains the 16 ms fallback used by DRM.
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
        self.ensure_armed_with_interval(has_animations, Duration::from_millis(16), handle, on_tick);
    }

    /// Arm the timer at a backend-selected frame interval.
    ///
    /// X11 and nested winit use this to follow their active display's refresh
    /// rate. Native DRM intentionally keeps [`Self::ensure_armed`] as a
    /// fallback because page flips already drive its normal frame cadence.
    pub fn ensure_armed_with_interval<'loop_handle, T: 'static>(
        &self,
        has_animations: bool,
        interval: Duration,
        handle: &calloop::LoopHandle<'loop_handle, T>,
        on_tick: impl Fn(&mut T) -> bool + 'static,
    ) {
        if !has_animations || self.active.get() {
            return;
        }
        self.active.set(true);
        let flag = Rc::clone(&self.active);
        let _ = handle.insert_source(Timer::from_duration(interval), move |_, _, data| {
            let still_active = on_tick(data);
            if still_active {
                TimeoutAction::ToDuration(interval)
            } else {
                flag.set(false);
                TimeoutAction::Drop
            }
        });
    }
}

/// Convert a refresh rate expressed in millihertz to a timer interval.
/// Invalid or unavailable rates retain the historical 16 ms fallback.
pub fn animation_frame_interval(refresh_millihertz: Option<u32>) -> Duration {
    refresh_millihertz
        .filter(|rate| *rate > 0)
        .map(|rate| Duration::from_nanos(1_000_000_000_000u64 / u64::from(rate)))
        .filter(|interval| !interval.is_zero())
        .unwrap_or_else(|| Duration::from_millis(16))
}

#[cfg(test)]
mod tests {
    use super::{TickOptions, animation_frame_interval, process_pending_work};
    use crate::backend::{Backend as WmBackend, wayland::WaylandBackend};
    use crate::types::MonitorId;
    use crate::wm::Wm;
    use std::time::Duration;

    #[test]
    fn animation_interval_tracks_refresh_rate() {
        assert_eq!(
            animation_frame_interval(Some(60_000)),
            Duration::from_nanos(16_666_666)
        );
        assert_eq!(
            animation_frame_interval(Some(144_000)),
            Duration::from_nanos(6_944_444)
        );
    }

    #[test]
    fn animation_interval_falls_back_for_unknown_refresh_rate() {
        assert_eq!(animation_frame_interval(None), Duration::from_millis(16));
        assert_eq!(animation_frame_interval(Some(0)), Duration::from_millis(16));
    }

    #[test]
    fn non_urgent_layout_can_be_deferred_for_animations() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.work.layout.clear();
        wm.work.layout.mark_monitor(MonitorId::default());

        process_pending_work(
            &mut wm,
            TickOptions {
                defer_layout_while_animations_active: true,
                animations_active: true,
            },
        );

        assert!(wm.work.layout.is_pending());
    }

    #[test]
    fn urgent_layout_bypasses_animation_defer() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.work.layout.clear();
        wm.work.layout.mark_monitor_urgent(MonitorId::default());

        process_pending_work(
            &mut wm,
            TickOptions {
                defer_layout_while_animations_active: true,
                animations_active: true,
            },
        );

        assert!(!wm.work.layout.is_pending());
    }
}
