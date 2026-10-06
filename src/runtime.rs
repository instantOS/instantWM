//! Shared event-loop tick helpers used by both X11 and Wayland backends.
//!
//! Shared operations receive [`WmCtx`], whose capabilities are
//! borrowed from the running backend. Policy remains backend-independent.

use crate::contexts::WmCtx;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction};

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
    ctx: &mut WmCtx<'_>,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    options: TickOptions,
) -> TickResult {
    let systray_updated = ctx.core_mut().poll_systray();
    if crate::systray::instantmenu::drive_instantmenu_menu(ctx.core_mut()) {
        ctx.core_mut().bar.mark_dirty();
    }
    let status_handled = ctx.core_mut().bar.drain_status_updates();
    // A finished region selection may resize a window, so it drains before
    // pending work to let the same tick apply the resulting layout.
    let region_selection_applied = crate::mouse::slop::drain_region_selection(ctx);
    let ipc_handled = process_ipc_commands(ipc_server, ctx);
    let work = process_pending_work(ctx, options);
    crate::bar::status::sync_visibility(ctx.core_mut());

    {
        let _ = crate::mouse::interaction::reconcile_capture(ctx);
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
pub fn process_pending_work(ctx: &mut WmCtx<'_>, options: TickOptions) -> PendingWorkResult {
    let mut result = PendingWorkResult::default();

    if std::mem::take(&mut ctx.core_mut().work.monitor_config) {
        crate::monitor::apply_monitor_config(ctx);
        result.monitor_config_applied = true;
    }

    crate::hooks::run_monitor_hooks(ctx);

    // Edge scratchpads finish their slide-out through backend animation
    // bookkeeping; complete the deferred logical hide once it drained.
    let pending_hides = ctx.core().work.pending_scratchpad_hide_windows();
    let finished_hides: Vec<crate::types::WindowId> = pending_hides
        .into_iter()
        .filter(|win| !ctx.window_animation_active(*win))
        .collect();
    {
        let work = &mut ctx.core_mut().work;
        for win in &finished_hides {
            work.cancel_pending_scratchpad_hide(*win);
        }
    }
    if !finished_hides.is_empty() {
        crate::floating::finish_scratchpad_hides(ctx, &finished_hides);
    }

    let Some(targets) = take_ready_layout(&mut ctx.core_mut().work.layout, options) else {
        return result;
    };
    result.layout_applied = apply_layout_targets(ctx, targets);
    result
}

/// Resolve scheduler policy without borrowing native capabilities. The borrow
/// ends before applying layout, which can synchronously dispatch native effects.
fn take_ready_layout(
    layout: &mut crate::core_state::PendingLayoutWork,
    options: TickOptions,
) -> Option<LayoutWorkTargets> {
    if !layout.is_pending()
        || (options.defer_layout_while_animations_active
            && options.animations_active
            && !layout.is_urgent())
    {
        return None;
    }
    layout.take_targets()
}

fn apply_layout_targets(ctx: &mut WmCtx<'_>, targets: LayoutWorkTargets) -> bool {
    if ctx.core().state.model.client_count() == 0 {
        return false;
    }

    match targets {
        LayoutWorkTargets::AllMonitors => {
            crate::layouts::arrange(ctx, None);
            true
        }
        LayoutWorkTargets::Monitors(monitors) => {
            if monitors.is_empty() {
                return false;
            }
            for monitor_id in monitors {
                crate::layouts::arrange(ctx, Some(monitor_id));
            }
            true
        }
    }
}

/// Process pending IPC commands.
///
/// Returns `true` when at least one command was handled.
pub fn process_ipc_commands(
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    ctx: &mut WmCtx<'_>,
) -> bool {
    let Some(server) = ipc_server.as_mut() else {
        return false;
    };
    server.process_pending(ctx)
}

// ── Startup helpers ─────────────────────────────────────────────────────

/// Initialise the keyboard layout from the WM configuration.
pub fn init_keyboard_layout(ctx: &mut WmCtx<'_>) {
    crate::keyboard_layout::init_keyboard_layout(ctx);
}

/// Spawn the configured status bar command, the auto-detected
/// `i3status-rs`, or the built-in default (in that order of
/// precedence).
pub fn spawn_status_bar<B: crate::backend::BackendState>(wm: &mut Wm<B>) {
    crate::bar::status::sync_visibility(&mut wm.core);
    wm.core
        .bar
        .status_sources
        .start(wm.core.state.config.status_command.as_deref());
}

/// Run autostart, user-defined `exec_once` and `exec` commands.
///
/// Called by each backend during startup. The Wayland backends call this
/// from [`autostart_ipc_status_ping`], while X11 calls it from
/// [`late_init_x11`].
pub fn run_startup_commands<B: crate::backend::BackendState>(wm: &Wm<B>) {
    crate::startup::autostart::run_autostart();
    crate::startup::autostart::run_exec_commands(&wm.core.state.config.exec_once);
    crate::startup::autostart::run_exec_commands(&wm.core.state.config.exec);
}

/// X11 late startup sequence.
///
/// Binds the IPC socket first so startup commands — including `ins autostart`,
/// which applies the wallpaper through `instantwmctl wallpaper` — can reach
/// the compositor immediately, then runs them and spawns the status bar.
/// The StatusNotifier worker starts later, from the calloop event loop, so it
/// can receive a wake ping; see `backend::x11::events::run`.
pub fn late_init_x11(wm: &mut crate::wm::X11Wm) -> Option<crate::ipc::IpcServer> {
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
    use crate::test_support::TestWm as Wm;
    use crate::types::MonitorId;

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
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.work.layout.clear();
        wm.core.work.layout.mark_monitor(MonitorId::default());

        wm.with_ctx(|wm| {
            process_pending_work(
                wm,
                TickOptions {
                    defer_layout_while_animations_active: true,
                    animations_active: true,
                },
            )
        });

        assert!(wm.core.work.layout.is_pending());
    }

    #[test]
    fn urgent_layout_bypasses_animation_defer() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.work.layout.clear();
        wm.core
            .work
            .layout
            .mark_monitor_urgent(MonitorId::default());

        wm.with_ctx(|wm| {
            process_pending_work(
                wm,
                TickOptions {
                    defer_layout_while_animations_active: true,
                    animations_active: true,
                },
            )
        });

        assert!(!wm.core.work.layout.is_pending());
    }
    /// Exercise the shared scheduler against a real X11 connection: an edge
    /// scratchpad must remain mapped until the X11 animation map drains.
    #[test]
    #[ignore = "requires a dedicated Xvfb display"]
    fn x11_scratchpad_hide_waits_for_native_animation() {
        use crate::test_support::MonitorBuilder;
        use crate::types::{Client, Rect, WindowId};
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConnectionExt, CreateWindowAux, MapState, WindowClass};

        let (conn, screen) = x11rb::connect(None).expect("test requires Xvfb");
        let root = conn.setup().roots[screen].root;
        let xid = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            xid,
            root,
            0,
            0,
            640,
            360,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap()
        .check()
        .unwrap();
        conn.map_window(xid).unwrap().check().unwrap();
        let mut wm = crate::wm::X11Wm::new(crate::backend::X11BackendData::new(conn, screen));
        wm.backend.x11_runtime.root = root;
        let monitor = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(0, 0, 1920, 1080))
                .bar(0, false)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor);
        let win = WindowId::from(xid);
        let mut client = Client {
            win,
            geo: Rect::new(0, 0, 640, 360),
            ..Client::default()
        };
        client
            .promote_to_scratchpad(
                monitor,
                "edge",
                Some(crate::types::EdgeDirection::Top),
                1920,
                1080,
            )
            .unwrap();
        wm.core.state.model.add_client(monitor, client);
        wm.core.state.config.animations.enabled = true;
        crate::floating::scratchpad::hide_scratchpad_window(&mut wm.x11_ctx(), win);
        assert!(wm.x11_ctx().window_animation_active(win));

        process_pending_work(&mut wm.x11_ctx(), TickOptions::default());
        assert!(wm.core.work.has_pending_scratchpad_hide(win));
        assert!(
            wm.core
                .state
                .model
                .client(win)
                .unwrap()
                .is_scratchpad_visible()
        );
        assert_eq!(
            wm.backend
                .conn
                .get_window_attributes(xid)
                .unwrap()
                .reply()
                .unwrap()
                .map_state,
            MapState::VIEWABLE
        );

        wm.backend.x11_runtime.window_animations.remove(&win);
        process_pending_work(&mut wm.x11_ctx(), TickOptions::default());
        assert!(!wm.core.work.has_pending_scratchpad_hide(win));
        assert!(
            !wm.core
                .state
                .model
                .client(win)
                .unwrap()
                .is_scratchpad_visible()
        );
        assert_eq!(
            wm.backend
                .conn
                .get_window_attributes(xid)
                .unwrap()
                .reply()
                .unwrap()
                .map_state,
            MapState::UNMAPPED
        );
    }
}
