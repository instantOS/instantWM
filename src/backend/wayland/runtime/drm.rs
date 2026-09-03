//! DRM/KMS bare-metal backend for running directly on hardware.

use smithay::backend::drm::DrmEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::libinput::LibinputSessionInterface;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::input::pointer::CursorIcon;
use smithay::reexports::calloop::{EventLoop, LoopHandle, LoopSignal};
use smithay::reexports::drm::control::crtc;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::utils::{Clock, Monotonic};
use smithay::wayland::presentation::Refresh;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::env;
use std::mem;
use std::process::exit;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::backend::output::{
    OutputId, OutputPlacement, OutputPowerError, OutputPowerMode, plan_automatic_output_positions,
};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::init::drm::init_gpu;
use crate::backend::wayland::input::apply_pending_warp;
use crate::backend::wayland::render::cursor::{ResolvedCursor, resolve_cursor};
use crate::backend::wayland::render::drm::{
    CursorManager, ManagedDrmOutputManager, OutputSurfaceEntry, RenderOutcome,
    add_new_output_surfaces, build_output_surfaces, create_output_manager, render_drm_output,
    usable_connector_handles,
};
use crate::backend::wayland::render::scene::{SceneCache, build_shared_scene_elements};
use crate::config::config_toml::CursorConfig;
use crate::wm::Wm;

mod output_config;
mod vrr;

use output_config::{process_output_configurations, process_output_power_requests};
use vrr::apply_output_vrr_policy;

#[derive(Debug)]
struct DrmLayoutState {
    layout: crate::types::Rect,
    output_hit_regions: Vec<OutputHitRegion>,
}

/// Render-scheduling geometry for one scanout. This deliberately lives in the
/// DRM runtime rather than the renderer: deciding which CRTC needs a frame is
/// event-loop policy, and the renderer already owns the authoritative output
/// geometry through [`OutputSurfaceEntry`].
#[derive(Debug, Clone, Copy)]
struct OutputHitRegion {
    crtc: crtc::Handle,
    rect: crate::types::Rect,
}

struct DrmLoopState {
    session_active: bool,
    render_flags: HashMap<crtc::Handle, bool>,
    taken_render_flags: HashMap<crtc::Handle, bool>,
    pending_crtcs: HashSet<crtc::Handle>,
    presentation_scheduler: super::engine::PresentationScheduler<crtc::Handle>,
    presentation_seq: HashMap<crtc::Handle, u64>,
    last_bar_update_seq: u64,
    scene_cache: SceneCache,
}

impl DrmLoopState {
    fn new(output_surfaces: &[OutputSurfaceEntry]) -> Self {
        let render_flags = output_surfaces
            .iter()
            .map(|entry| (entry.crtc, true))
            .collect();
        Self {
            session_active: true,
            render_flags,
            taken_render_flags: HashMap::new(),
            pending_crtcs: HashSet::new(),
            presentation_scheduler: super::engine::PresentationScheduler::default(),
            presentation_seq: output_surfaces
                .iter()
                .map(|entry| (entry.crtc, 0))
                .collect(),
            last_bar_update_seq: 0,
            scene_cache: SceneCache::default(),
        }
    }

    fn mark_all_dirty(&mut self) {
        for flag in self.render_flags.values_mut() {
            *flag = true;
        }
    }

    fn mark_dirty(&mut self, crtc: crtc::Handle) {
        if let Some(flag) = self.render_flags.get_mut(&crtc) {
            *flag = true;
        }
    }

    fn mark_pointer_output_dirty(&mut self, pointer: crate::types::Point, layout: &DrmLayoutState) {
        if let Some(crtc) = output_at_pointer(&layout.output_hit_regions, pointer) {
            self.mark_dirty(crtc);
        } else {
            // This can be observed briefly while a new output layout is being
            // projected. Redrawing all outputs is the safe recovery path.
            self.mark_all_dirty();
        }
    }

    fn take_render_flags(&mut self) -> HashMap<crtc::Handle, bool> {
        let mut taken = mem::take(&mut self.taken_render_flags);
        taken.clear();
        for (&crtc, flag) in &mut self.render_flags {
            taken.insert(crtc, *flag);
            *flag = false;
        }
        taken
    }

    fn has_renderable_dirty_outputs(&self) -> bool {
        self.render_flags
            .iter()
            .any(|(crtc, &dirty)| dirty && !self.pending_crtcs.contains(crtc))
    }

    fn remove_output(&mut self, crtc: crtc::Handle) {
        self.render_flags.remove(&crtc);
        self.taken_render_flags.remove(&crtc);
        self.pending_crtcs.remove(&crtc);
        self.presentation_seq.remove(&crtc);
        self.presentation_scheduler.remove_output(&crtc);
    }

    fn add_output(&mut self, crtc: crtc::Handle) {
        self.render_flags.insert(crtc, true);
        self.presentation_seq.entry(crtc).or_insert(0);
    }
}

fn output_at_pointer(
    regions: &[OutputHitRegion],
    pointer: crate::types::Point,
) -> Option<crtc::Handle> {
    regions
        .iter()
        .find(|region| region.rect.contains_point(pointer))
        .map(|region| region.crtc)
}

#[derive(Debug, Clone, Copy)]
enum DrmRuntimeEvent {
    SessionPaused,
    SessionActivated,
    VBlank(crtc::Handle),
    PointerMoved { old_location: crate::types::Point },
    OutputTopologyChanged,
}

// WARNING: This function is extremely fragile, do not refactor or mess with it without
// great care and patience for random ass segfaults. Yes, this is awful, leave it.
// Hours spent on this: ~3h
pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");
    let mut wm = super::bootstrap::create_wayland_wm_boxed();
    let (event_loop, mut state) = crate::backend::wayland::compositor::new_event_loop_and_state();
    let loop_handle = event_loop.handle();

    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    state.runtime.session = Some(session.clone());

    super::bootstrap::attach_backend_state(&mut wm, &mut state);

    crate::runtime::init_keyboard_layout(&mut wm);

    let (primary_gpu_path, drm_device, drm_notifier, drm_fd, gbm_device, egl_display, mut renderer) =
        init_gpu(&mut session, &seat_name);
    log::info!("Using GPU: {:?}", primary_gpu_path);

    state.init_drm_syncobj(drm_fd.clone());

    super::bootstrap::attach_gles_renderer_and_protocols(
        &mut state,
        &mut renderer,
        Some(&egl_display),
    );
    state.attach_wm(&mut wm);

    let mut cursor_manager = init_cursor_manager(&state.cursor_config);
    let output_manager = Arc::new(Mutex::new(create_output_manager(
        drm_device,
        &renderer,
        &gbm_device,
    )));

    let mut output_surfaces = {
        let mut manager = output_manager.lock().unwrap();
        build_output_surfaces(&mut manager, &mut renderer, &mut state)
    };
    for entry in &output_surfaces {
        state
            .space
            .map_output(&entry.output, (entry.rect.x, entry.rect.y));
    }

    // Register all DRM outputs with wlr-output-management.
    state
        .output_management_state
        .add_heads::<crate::backend::wayland::compositor::WaylandState>(
            output_surfaces.iter().map(|e| &e.output),
        );

    let layout = compute_total_dimensions(&output_surfaces);

    {
        use crate::monitor::refresh_monitor_layout;
        refresh_monitor_layout(&mut wm.ctx());
    }
    state.push_command(crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones);
    crate::monitor::apply_monitor_config(&mut wm.ctx());

    let mut layout_state = init_layout_state(&output_surfaces, layout);
    // Calloop dispatches sources and the loop callback sequentially on this
    // thread. Libinput only needs the current layout bounds, so share that
    // small copy without putting the complete layout behind an atomic lock.
    let input_dimensions = Rc::new(Cell::new(layout));
    let mut loop_state = DrmLoopState::new(&output_surfaces);
    let (runtime_event_tx, runtime_event_rx) = mpsc::channel();

    super::bootstrap::setup_listen_socket(&loop_handle, &state, &mut wm);

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");

    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());
    let shared_input_dimensions = Rc::clone(&input_dimensions);
    let runtime_event_tx_input = runtime_event_tx.clone();
    loop_handle
        .insert_source(libinput_backend, move |event, _, state| {
            let layout = shared_input_dimensions.get();

            // SAFETY: calloop source callback runs synchronously within
            // event_loop.dispatch(); the &mut Wm borrow in the main body
            // has not yet resumed.
            let old_pointer_location = crate::types::Point::new(
                state.runtime.pointer_location.x as i32,
                state.runtime.pointer_location.y as i32,
            );
            let outcome = if let Some(wm_ptr) = unsafe { state.wm_mut_ptr() } {
                let wm = unsafe { &mut *wm_ptr };
                crate::backend::wayland::input::drm::dispatch_libinput_event(
                    event, state, wm, layout,
                )
            } else {
                crate::backend::wayland::input::drm::LibinputEventOutcome::Ignored
            };

            use crate::backend::wayland::input::drm::LibinputEventOutcome;
            match outcome {
                LibinputEventOutcome::Ignored => {}
                LibinputEventOutcome::Activity => state.notify_activity(),
                LibinputEventOutcome::PointerMoved => {
                    state.notify_activity();
                    let _ = runtime_event_tx_input.send(DrmRuntimeEvent::PointerMoved {
                        old_location: old_pointer_location,
                    });
                }
            }
        })
        .expect("failed to insert libinput source");

    setup_session_handlers(
        &loop_handle,
        notifier,
        &mut libinput_context,
        Arc::clone(&output_manager),
        runtime_event_tx.clone(),
    );

    setup_drm_vblank_handler(&loop_handle, drm_notifier, runtime_event_tx.clone());
    setup_udev_hotplug_handler(&loop_handle, &seat_name, runtime_event_tx.clone());

    let mut ipc_server = super::bootstrap::autostart_ipc_status_ping(&loop_handle, &wm);

    // One-shot wakeup for the initial frame. Later render failures use a
    // bounded timer instead of an immediate self-ping loop.
    let (initial_render_ping, initial_render_ping_source) =
        calloop::ping::make_ping().expect("ping");
    event_loop
        .handle()
        .insert_source(initial_render_ping_source, |_, _, _| {})
        .expect("ping source");

    // Compositor redraw pings preserve a target set installed by
    // `request_output_render`.  A bare ping (for example, completion of the
    // asynchronous bar worker) falls back to invalidating all outputs.
    let (render_ping, render_ping_source) = calloop::ping::make_ping().expect("render ping");
    event_loop
        .handle()
        .insert_source(render_ping_source, |_, _, state| {
            if matches!(
                state.runtime.render_targets,
                crate::backend::wayland::compositor::PendingRenderTargets::None
            ) {
                state.runtime.render_targets =
                    crate::backend::wayland::compositor::PendingRenderTargets::All;
            }
        })
        .expect("render ping source");
    state.runtime.render_ping = Some(render_ping);
    initial_render_ping.ping();

    let start_time = Instant::now();
    let mut render_failures: HashMap<crtc::Handle, u32> = HashMap::new();

    crate::runtime::spawn_status_bar(&wm);

    let (led_state_tx, led_state_rx) = mpsc::channel();
    state.runtime.led_state_tx = Some(led_state_tx);

    run_event_loop(
        event_loop,
        &mut wm,
        &mut state,
        &mut layout_state,
        &input_dimensions,
        &mut loop_state,
        &mut output_surfaces,
        &output_manager,
        &mut renderer,
        &mut cursor_manager,
        &mut ipc_server,
        &mut render_failures,
        start_time,
        led_state_rx,
        runtime_event_rx,
    );

    crate::startup::autostart::shutdown_autostart();
    crate::backend::wayland::session::stop_graphical_session_target();
    exit(0);
}

/// Initialize cursor manager from environment or defaults.
fn init_cursor_manager(config: &CursorConfig) -> CursorManager {
    let cursor_theme = env::var("XCURSOR_THEME").unwrap_or_else(|_| config.theme.clone());
    let configured_size = env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(config.size);
    CursorManager::new(&cursor_theme, cursor_size(configured_size))
}

fn cursor_size(size: u32) -> u8 {
    size.clamp(1, u8::MAX as u32) as u8
}

/// Compute total screen bounds from output surfaces, origin included.
///
/// Absolute-device mapping needs both the size and the origin: a layout with
/// a negative position (output placed above/left) maps `0..size` onto
/// `origin..origin+size`.
fn compute_total_dimensions(output_surfaces: &[OutputSurfaceEntry]) -> crate::types::Rect {
    output_layout_bounds(
        output_surfaces.iter().map(|surface| surface.rect),
        crate::types::Size::new(
            crate::backend::wayland::render::drm::DEFAULT_SCREEN_WIDTH,
            crate::backend::wayland::render::drm::DEFAULT_SCREEN_HEIGHT,
        ),
    )
}

fn output_layout_bounds(
    rects: impl IntoIterator<Item = crate::types::Rect>,
    fallback: crate::types::Size,
) -> crate::types::Rect {
    let mut min_x = None::<i32>;
    let mut min_y = None::<i32>;
    let mut max_x = None::<i32>;
    let mut max_y = None::<i32>;
    for rect in rects {
        let right = rect.x.saturating_add(rect.w);
        let bottom = rect.y.saturating_add(rect.h);
        min_x = Some(min_x.map_or(rect.x, |v| v.min(rect.x)));
        min_y = Some(min_y.map_or(rect.y, |v| v.min(rect.y)));
        max_x = Some(max_x.map_or(right, |v| v.max(right)));
        max_y = Some(max_y.map_or(bottom, |v| v.max(bottom)));
    }
    match (min_x, min_y, max_x, max_y) {
        (Some(min_x), Some(min_y), Some(max_x), Some(max_y)) => crate::types::Rect::new(
            min_x,
            min_y,
            max_x.saturating_sub(min_x).max(1),
            max_y.saturating_sub(min_y).max(1),
        ),
        _ => crate::types::Rect::new(0, 0, fallback.w.max(1), fallback.h.max(1)),
    }
}

fn output_layout_size(
    rects: impl IntoIterator<Item = crate::types::Rect>,
    fallback: crate::types::Size,
) -> crate::types::Size {
    output_layout_bounds(rects, fallback).size()
}

fn init_layout_state(
    output_surfaces: &[OutputSurfaceEntry],
    layout: crate::types::Rect,
) -> DrmLayoutState {
    DrmLayoutState {
        layout,
        output_hit_regions: output_surfaces
            .iter()
            .map(|entry| OutputHitRegion {
                crtc: entry.crtc,
                rect: entry.rect,
            })
            .collect(),
    }
}

fn refresh_drm_layout_state(
    state: &WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    layout_state: &mut DrmLayoutState,
) {
    for entry in output_surfaces.iter_mut().filter(|entry| entry.enabled) {
        if let Some(geometry) = state.space.output_geometry(&entry.output) {
            entry.rect = crate::types::Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            );
        }
    }
    let active: Vec<_> = output_surfaces
        .iter()
        .filter(|entry| entry.enabled)
        .collect();
    let layout = output_layout_bounds(
        active.iter().map(|entry| entry.rect),
        crate::types::Size::new(1, 1),
    );
    *layout_state = DrmLayoutState {
        layout,
        output_hit_regions: active
            .iter()
            .map(|entry| OutputHitRegion {
                crtc: entry.crtc,
                rect: entry.rect,
            })
            .collect(),
    };
}

/// Setup session pause/activate handlers for VT switching.
fn setup_session_handlers(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    notifier: smithay::backend::session::libseat::LibSeatSessionNotifier,
    libinput_context: &mut Libinput,
    output_manager: Arc<Mutex<ManagedDrmOutputManager>>,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
) {
    let mut session_libinput = libinput_context.clone();
    let session_output_manager = Arc::clone(&output_manager);

    loop_handle
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) - suspending rendering");
                if let Some(wm_ptr) = unsafe { state.wm_mut_ptr() } {
                    let wm = unsafe { &mut *wm_ptr };
                    crate::backend::wayland::input::touch::handle_touch_cancel(wm, state);
                }
                session_libinput.suspend();
                session_output_manager.lock().unwrap().pause();
                let _ = runtime_event_tx.send(DrmRuntimeEvent::SessionPaused);
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated (VT switch back) - resuming rendering");
                if let Err(err) = session_libinput.resume() {
                    log::error!("failed to resume libinput context: {:?}", err);
                }
                if let Err(err) = session_output_manager
                    .lock()
                    .unwrap()
                    .lock()
                    .activate(false)
                {
                    log::error!("failed to reactivate DRM device: {err}");
                }
                let _ = runtime_event_tx.send(DrmRuntimeEvent::SessionActivated);
            }
        })
        .expect("session source");
}

/// Setup DRM vblank handler for render synchronization.
fn setup_drm_vblank_handler(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    drm_notifier: smithay::backend::drm::DrmDeviceNotifier,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
) {
    loop_handle
        .insert_source(drm_notifier, move |event, _metadata, _data| match event {
            DrmEvent::VBlank(crtc) => {
                let _ = runtime_event_tx.send(DrmRuntimeEvent::VBlank(crtc));
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .expect("drm notifier source");
}

/// Wake the DRM runtime whenever udev reports a graphics-device topology
/// change. Connector probing and mutation remain on the compositor thread.
fn setup_udev_hotplug_handler(
    loop_handle: &calloop::LoopHandle<WaylandState>,
    seat_name: &str,
    runtime_event_tx: mpsc::Sender<DrmRuntimeEvent>,
) {
    let backend = match UdevBackend::new(seat_name) {
        Ok(backend) => backend,
        Err(error) => {
            log::error!("failed to monitor DRM hot-plug events: {error}");
            return;
        }
    };
    let retry_handle = loop_handle.clone();
    loop_handle
        .insert_source(backend, move |event, _, _| match event {
            UdevEvent::Added { .. } | UdevEvent::Changed { .. } | UdevEvent::Removed { .. } => {
                let _ = runtime_event_tx.send(DrmRuntimeEvent::OutputTopologyChanged);
                // Thunderbolt and DisplayPort MST branches enumerate in
                // stages. Udev does not guarantee that the final connector
                // state has settled when the first DRM change arrives, nor
                // that every later link-state transition produces a distinct
                // event useful to this compositor. Reprobe at bounded settling
                // points so a connector or mode that appears late is not lost
                // until the next physical hotplug.
                for delay in [
                    Duration::from_millis(100),
                    Duration::from_millis(350),
                    Duration::from_secs(1),
                ] {
                    let retry_tx = runtime_event_tx.clone();
                    if let Err(error) = retry_handle.insert_source(
                        smithay::reexports::calloop::timer::Timer::from_duration(delay),
                        move |_, _, _| {
                            let _ = retry_tx.send(DrmRuntimeEvent::OutputTopologyChanged);
                            smithay::reexports::calloop::timer::TimeoutAction::Drop
                        },
                    ) {
                        log::warn!("failed to schedule DRM hot-plug reprobe: {error}");
                    }
                }
            }
        })
        .expect("failed to insert udev hot-plug source");
}

/// Extract a `CursorIcon` from a resolved cursor presentation for
/// animation-timer scheduling.  `Surface` cursors are client-owned and
/// cannot be introspected, so they return `None`.
fn resolved_cursor_icon(p: &ResolvedCursor) -> Option<CursorIcon> {
    match p {
        ResolvedCursor::Hidden | ResolvedCursor::Surface { .. } => None,
        ResolvedCursor::Named(icon) => Some(*icon),
        ResolvedCursor::DndIcon { cursor, .. } => resolved_cursor_icon(cursor),
    }
}

/// Run the main event loop.
///
/// This is the heart of the DRM backend. It handles:
/// - Frame submission tracking
/// - Libinput event dispatch
/// - Layout arrangement
/// - IPC command processing
/// - Window animations
/// - Cursor warp
/// - DRM rendering
#[allow(clippy::too_many_arguments)]
fn run_event_loop(
    mut event_loop: EventLoop<WaylandState>,
    wm: &mut Wm,
    state: &mut WaylandState,
    layout_state: &mut DrmLayoutState,
    input_dimensions: &Rc<Cell<crate::types::Rect>>,
    loop_state: &mut DrmLoopState,
    output_surfaces: &mut Vec<OutputSurfaceEntry>,
    output_manager: &Arc<Mutex<ManagedDrmOutputManager>>,
    renderer: &mut GlesRenderer,
    cursor_manager: &mut CursorManager,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: Instant,
    led_state_rx: mpsc::Receiver<smithay::input::keyboard::LedState>,
    runtime_event_rx: mpsc::Receiver<DrmRuntimeEvent>,
) {
    let loop_signal: LoopSignal = event_loop.get_signal();
    let loop_handle = event_loop.handle();
    let pointer_handle = state.pointer.clone();
    let render_retry_guard = crate::runtime::AnimationTimerGuard::new();
    let shared_input_dimensions = Rc::clone(input_dimensions);
    let monotonic_clock = Clock::<Monotonic>::new();

    event_loop
        .run(None, state, move |state| {
            let (pointer_moved, topology_changed) = process_runtime_events(
                &runtime_event_rx,
                loop_state,
                layout_state,
                output_surfaces,
                &monotonic_clock,
            );
            if topology_changed
                && reconcile_drm_outputs(
                    wm,
                    state,
                    output_surfaces,
                    output_manager,
                    renderer,
                    loop_state,
                    layout_state,
                    &shared_input_dimensions,
                )
            {
                loop_state.mark_all_dirty();
            }
            process_frame_callback_requests(
                state,
                &loop_handle,
                loop_state,
                output_surfaces,
                start_time,
            );
            for entry in output_surfaces
                .iter()
                .filter(|entry| entry.enabled && entry.powered)
            {
                loop_state.presentation_scheduler.schedule_commit_timing(
                    entry.crtc,
                    &loop_handle,
                    state,
                    &entry.output,
                );
            }
            super::engine::event_loop_tick_and_request_render(wm, state, ipc_server);
            process_output_configurations(
                state,
                output_surfaces,
                output_manager,
                renderer,
                loop_state,
            );
            let outputs_changed = state.project_completed_output_transactions();
            if outputs_changed {
                crate::monitor::refresh_monitor_layout(&mut wm.ctx());
                state.push_command(
                    crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones,
                );
                refresh_drm_layout_state(state, output_surfaces, layout_state);
                shared_input_dimensions.set(layout_state.layout);
            }
            state.project_completed_output_power_requests();
            process_output_power_requests(state, output_surfaces, loop_state);
            state.project_completed_output_power_requests();
            if pointer_moved {
                loop_state.mark_pointer_output_dirty(
                    crate::types::Point::new(
                        state.runtime.pointer_location.x as i32,
                        state.runtime.pointer_location.y as i32,
                    ),
                    layout_state,
                );
            }
            super::engine::process_animations_and_request_render(state);
            process_commit_redraws(state, loop_state, output_surfaces);
            let bar_update_seq = wm.bar.update_seq();
            if loop_state.last_bar_update_seq != bar_update_seq {
                loop_state.last_bar_update_seq = bar_update_seq;
                loop_state.mark_all_dirty();
            }

            if wm.work.input_config {
                wm.work.input_config = false;
                crate::backend::wayland::input::drm::reconfigure_all_devices(
                    &mut state.runtime.tracked_devices,
                    &wm.core.config.input,
                );
            }

            if wm.work.cursor_config {
                wm.work.cursor_config = false;
                let cursor = &wm.core.config.cursor;
                cursor_manager.reload(&cursor.theme, cursor_size(cursor.size));
                state.cursor_config = cursor.clone();
                loop_state.mark_all_dirty();
            }

            while let Ok(led_state) = led_state_rx.try_recv() {
                let leds = smithay::reexports::input::Led::from(led_state);
                for device in state.runtime.tracked_devices.iter_mut() {
                    use smithay::reexports::input::DeviceCapability;
                    if device.has_capability(DeviceCapability::Keyboard) {
                        device.led_update(leds);
                    }
                }
            }

            // Resolve cursor animation state. Its first dirty frame starts a
            // page-flip chain; subsequent vblanks advance it without an
            // independent compositor timer drifting against scanout.
            let animated = {
                let presentation = resolve_cursor(
                    &state.cursor_image_status,
                    state.cursor_icon_override,
                    state.runtime.dnd_icon.as_ref(),
                    state.runtime.cursor_hidden_by_touch,
                );
                resolved_cursor_icon(&presentation)
                    .is_some_and(|icon| cursor_manager.is_animated(icon, 1))
            };
            state.runtime.cursor_is_animated = animated;
            if animated {
                loop_state.mark_pointer_output_dirty(
                    crate::types::Point::new(
                        state.runtime.pointer_location.x as i32,
                        state.runtime.pointer_location.y as i32,
                    ),
                    layout_state,
                );
            }

            if let Some(keyboard_handle) = state.seat.get_keyboard() {
                process_cursor_warp(wm, state, &pointer_handle, &keyboard_handle, loop_state);
            }

            render_outputs(
                wm,
                state,
                renderer,
                output_surfaces,
                cursor_manager,
                &loop_handle,
                loop_state,
                render_failures,
                start_time,
            );

            // Retry persistent render failures at a bounded cadence. Dirty
            // outputs with a page flip in flight are woken by DRM vblank.
            render_retry_guard.ensure_armed(
                loop_state.has_renderable_dirty_outputs(),
                &loop_handle,
                |_| false,
            );

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");
}

fn process_runtime_events(
    runtime_event_rx: &mpsc::Receiver<DrmRuntimeEvent>,
    loop_state: &mut DrmLoopState,
    layout_state: &DrmLayoutState,
    output_surfaces: &mut [OutputSurfaceEntry],
    monotonic_clock: &Clock<Monotonic>,
) -> (bool, bool) {
    let mut pointer_moved = false;
    let mut topology_changed = false;
    while let Ok(event) = runtime_event_rx.try_recv() {
        match event {
            DrmRuntimeEvent::SessionPaused => {
                loop_state.session_active = false;
            }
            DrmRuntimeEvent::SessionActivated => {
                loop_state.session_active = true;
                loop_state.mark_all_dirty();
            }
            DrmRuntimeEvent::VBlank(crtc) => {
                loop_state
                    .presentation_scheduler
                    .presentation_completed(crtc, Instant::now());
                if let Some(entry) = output_surfaces.iter_mut().find(|entry| entry.crtc == crtc) {
                    let Some(surface) = entry.surface.as_mut() else {
                        loop_state.pending_crtcs.remove(&crtc);
                        continue;
                    };
                    match surface.frame_submitted() {
                        Ok(Some(mut metadata)) => {
                            let seq = loop_state.presentation_seq.entry(crtc).or_insert(0);
                            *seq += 1;
                            metadata.presentation_feedback.presented(
                                monotonic_clock.now(),
                                output_refresh(entry),
                                *seq,
                                wp_presentation_feedback::Kind::Vsync,
                            );
                        }
                        Ok(None) => {}
                        Err(err) => {
                            log::warn!("frame_submitted failed for {:?}: {err}", crtc);
                        }
                    }
                }
                loop_state.pending_crtcs.remove(&crtc);
            }
            DrmRuntimeEvent::PointerMoved { old_location } => {
                loop_state.mark_pointer_output_dirty(old_location, layout_state);
                pointer_moved = true;
            }
            DrmRuntimeEvent::OutputTopologyChanged => topology_changed = true,
        }
    }
    (pointer_moved, topology_changed)
}

#[allow(clippy::too_many_arguments)]
fn reconcile_drm_outputs(
    wm: &mut Wm,
    state: &mut WaylandState,
    output_surfaces: &mut Vec<OutputSurfaceEntry>,
    output_manager: &Arc<Mutex<ManagedDrmOutputManager>>,
    renderer: &mut GlesRenderer,
    loop_state: &mut DrmLoopState,
    layout_state: &mut DrmLayoutState,
    input_dimensions: &Rc<Cell<crate::types::Rect>>,
) -> bool {
    let usable = {
        let manager = output_manager.lock().unwrap();
        match usable_connector_handles(&manager) {
            Ok(connectors) => connectors,
            Err(error) => {
                log::warn!("failed to probe DRM connectors after hot-plug: {error}");
                return false;
            }
        }
    };

    let old_connectors: HashSet<_> = output_surfaces
        .iter()
        .map(|entry| entry.connector)
        .collect();
    let mut retained = Vec::with_capacity(output_surfaces.len());
    let mut removed = Vec::new();
    for entry in output_surfaces.drain(..) {
        if usable.contains(&entry.connector) {
            retained.push(entry);
        } else {
            removed.push(entry);
        }
    }
    *output_surfaces = retained;
    let retained_connectors: HashSet<_> = output_surfaces
        .iter()
        .map(|entry| entry.connector)
        .collect();

    if !removed.is_empty() {
        state
            .output_management_state
            .remove_heads::<WaylandState>(removed.iter().map(|entry| &entry.output));
        for mut entry in removed {
            let name = entry.output.name();
            log::info!("Output {name}: disconnected");
            if let Some(id) = entry.pending_power_on.take() {
                state.runtime.output_power.complete_by_id(
                    id,
                    OutputId(name.clone()),
                    Err(OutputPowerError::Unavailable(name.clone())),
                );
            }
            entry.surface.take();
            loop_state.remove_output(entry.crtc);
            state.space.unmap_output(&entry.output);
            state.set_output_global_enabled(&entry.output, false);
            state.fail_pending_captures_for_output(&entry.output);
            state.runtime.output_power_modes.remove(&name);
            state.runtime.output_metadata.remove(&name);
            let cancelled = state.output_power_state.fail_output(&name);
            state.runtime.output_power.cancel(&cancelled);
        }
    }

    {
        let mut manager = output_manager.lock().unwrap();
        add_new_output_surfaces(&mut manager, renderer, state, output_surfaces);
    }

    let added: Vec<_> = output_surfaces
        .iter()
        .filter(|entry| !retained_connectors.contains(&entry.connector))
        .collect();
    for entry in &added {
        state
            .space
            .map_output(&entry.output, (entry.rect.x, entry.rect.y));
        loop_state.add_output(entry.crtc);
    }
    state
        .output_management_state
        .add_heads::<WaylandState>(added.iter().map(|entry| &entry.output));

    let connector_set_unchanged = added.is_empty()
        && old_connectors.len() == output_surfaces.len()
        && output_surfaces
            .iter()
            .all(|entry| old_connectors.contains(&entry.connector));
    if connector_set_unchanged {
        return false;
    }

    compact_drm_automatic_layout(state, output_surfaces);
    refresh_drm_layout_state(state, output_surfaces, layout_state);
    input_dimensions.set(layout_state.layout);
    crate::monitor::refresh_monitor_layout(&mut wm.ctx());
    state.push_command(crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones);
    crate::monitor::apply_monitor_config(&mut wm.ctx());
    true
}

fn compact_drm_automatic_layout(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
) {
    let mut placements: Vec<_> = output_surfaces
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| OutputPlacement {
            id: entry.output.name(),
            rect: entry.rect,
            source: entry.position_source,
        })
        .collect();
    for (name, position) in plan_automatic_output_positions(&mut placements) {
        let Some(entry) = output_surfaces
            .iter_mut()
            .find(|entry| entry.output.name() == name)
        else {
            continue;
        };
        entry.rect.x = position.x;
        entry.rect.y = position.y;
        entry
            .output
            .change_current_state(None, None, None, Some((position.x, position.y).into()));
        state
            .space
            .map_output(&entry.output, (position.x, position.y));
    }
}

fn output_refresh(entry: &OutputSurfaceEntry) -> Refresh {
    let period = entry.output.current_mode().and_then(|mode| {
        let refresh = u64::try_from(mode.refresh).ok()?;
        (refresh > 0).then(|| std::time::Duration::from_nanos(1_000_000_000_000u64 / refresh))
    });

    match (entry.vrr_enabled, period) {
        (true, Some(period)) => Refresh::variable(period),
        (false, Some(period)) => Refresh::fixed(period),
        (_, None) => Refresh::Unknown,
    }
}

/// Promote compositor-side redraw requests into DRM output dirties.
fn process_commit_redraws(
    state: &mut WaylandState,
    loop_state: &mut DrmLoopState,
    output_surfaces: &[OutputSurfaceEntry],
) {
    use crate::backend::wayland::compositor::PendingRenderTargets;

    match state.take_render_targets() {
        PendingRenderTargets::None => {}
        PendingRenderTargets::All => loop_state.mark_all_dirty(),
        PendingRenderTargets::Outputs(outputs) => {
            for entry in output_surfaces {
                if outputs.contains(&entry.output.name()) {
                    loop_state.mark_dirty(entry.crtc);
                }
            }
        }
    }
}

/// Service frame-callback-only commits without forcing a DRM render.
fn process_frame_callback_requests(
    state: &mut WaylandState,
    loop_handle: &LoopHandle<'_, WaylandState>,
    loop_state: &DrmLoopState,
    output_surfaces: &[OutputSurfaceEntry],
    start_time: Instant,
) {
    use crate::backend::wayland::compositor::PendingRenderTargets;

    let targets = state.take_frame_callback_targets();
    for entry in output_surfaces.iter().filter(|entry| entry.enabled) {
        let targeted = match &targets {
            PendingRenderTargets::None => false,
            PendingRenderTargets::All => true,
            PendingRenderTargets::Outputs(outputs) => outputs.contains(&entry.output.name()),
        };
        if targeted {
            loop_state.presentation_scheduler.arm_callbacks(
                entry.crtc,
                loop_handle,
                &entry.output,
                start_time,
            );
        }
    }
}

/// Apply compositor-side cursor warp.
fn process_cursor_warp(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    loop_state: &mut DrmLoopState,
) {
    if apply_pending_warp(wm, state, pointer_handle, keyboard_handle) {
        loop_state.mark_all_dirty();
    }
}

/// Render all outputs that need it.
#[allow(clippy::too_many_arguments)]
fn render_outputs(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    output_surfaces: &mut [OutputSurfaceEntry],
    cursor_manager: &CursorManager,
    loop_handle: &LoopHandle<'_, WaylandState>,
    loop_state: &mut DrmLoopState,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: Instant,
) {
    let render_flags = loop_state.take_render_flags();
    let session_active = loop_state.session_active;
    let pending_crtcs = loop_state.pending_crtcs.clone();

    let pointer_location = state.runtime.pointer_location;

    if session_active {
        let needs_any_render = output_surfaces
            .iter()
            .any(|entry| render_flags.get(&entry.crtc).copied().unwrap_or(false));
        let shared_scene = if needs_any_render && !state.is_locked() {
            Some(build_shared_scene_elements(
                wm,
                state,
                &mut loop_state.scene_cache,
            ))
        } else {
            None
        };

        for entry in output_surfaces.iter_mut() {
            let needs_render = render_flags.get(&entry.crtc).copied().unwrap_or(false);
            if !needs_render || !entry.enabled || !entry.powered {
                continue;
            }
            // Don't render if a page flip is already in flight — queue_buffer
            // would fail with EBUSY and leak a swapchain slot.
            if pending_crtcs.contains(&entry.crtc) {
                // Re-mark as dirty so we render after the VBlank arrives.
                loop_state.mark_dirty(entry.crtc);
                continue;
            }
            apply_output_vrr_policy(wm, state, entry);
            let suppress_upper_layers =
                crate::backend::wayland::render::scene::output_has_real_fullscreen(
                    wm,
                    &entry.output,
                );
            let rendered = render_drm_output(
                state,
                renderer,
                entry,
                cursor_manager,
                pointer_location,
                start_time,
                shared_scene.clone(),
                suppress_upper_layers,
            );

            match rendered {
                RenderOutcome::Submitted => {
                    if let Some(id) = entry.pending_power_on.take() {
                        let output = OutputId(entry.output.name());
                        state
                            .runtime
                            .output_power_modes
                            .insert(output.0.clone(), OutputPowerMode::On);
                        state.runtime.output_power.complete_by_id(
                            id,
                            output,
                            Ok(OutputPowerMode::On),
                        );
                    }
                    loop_state
                        .presentation_scheduler
                        .presentation_submitted(&entry.crtc);
                    loop_state.pending_crtcs.insert(entry.crtc);
                    if let Some(failed_frames) = render_failures.remove(&entry.crtc)
                        && failed_frames >= 3
                    {
                        log::info!(
                            "DRM render recovered on {:?} after {failed_frames} failed frames",
                            entry.crtc
                        );
                    }
                }
                RenderOutcome::EmptyFrame => {
                    loop_state.presentation_scheduler.arm_callbacks(
                        entry.crtc,
                        loop_handle,
                        &entry.output,
                        start_time,
                    );
                    render_failures.remove(&entry.crtc);
                    if entry.pending_power_on.is_some() {
                        loop_state.mark_dirty(entry.crtc);
                    }
                }
                RenderOutcome::Failed => {
                    let failed_frames = render_failures.entry(entry.crtc).or_insert(0);
                    *failed_frames += 1;

                    if *failed_frames == 1 || (*failed_frames).is_multiple_of(60) {
                        log::warn!(
                            "DRM render failed on {:?} (consecutive failures: {})",
                            entry.crtc,
                            *failed_frames
                        );
                    }

                    if *failed_frames >= 3
                        && let Some(id) = entry.pending_power_on.take()
                    {
                        entry.powered = false;
                        let output = OutputId(entry.output.name());
                        state.runtime.output_power.complete_by_id(
                            id,
                            output,
                            Err(OutputPowerError::Backend(
                                "failed to queue a frame while powering on".to_string(),
                            )),
                        );
                        if let Some(surface) = entry.surface.as_ref() {
                            let _ = surface.with_compositor(|compositor| compositor.clear());
                        }
                    } else {
                        loop_state.mark_dirty(entry.crtc);
                    }
                }
            }
        }
    }
    loop_state.taken_render_flags = render_flags;
}

#[cfg(test)]
mod cursor_config_tests {
    use super::cursor_size;

    #[test]
    fn cursor_size_stays_in_xcursor_range() {
        assert_eq!(cursor_size(0), 1);
        assert_eq!(cursor_size(24), 24);
        assert_eq!(cursor_size(512), u8::MAX);
    }
}

#[cfg(test)]
mod output_layout_tests {
    use smithay::reexports::drm::control::{crtc, from_u32};

    use super::{OutputHitRegion, output_at_pointer, output_layout_bounds, output_layout_size};
    use crate::types::{Point, Rect, Size};

    fn crtc(raw: u32) -> crtc::Handle {
        from_u32(raw).expect("test CRTC handles are non-zero")
    }

    #[test]
    fn stacked_outputs_with_overlapping_x_ranges_route_by_both_axes() {
        let top_left = crtc(1);
        let top_right = crtc(2);
        let bottom = crtc(3);
        let regions = [
            OutputHitRegion {
                crtc: top_left,
                rect: Rect::new(0, 0, 1920, 1080),
            },
            OutputHitRegion {
                crtc: top_right,
                rect: Rect::new(1920, 0, 1920, 1080),
            },
            OutputHitRegion {
                crtc: bottom,
                rect: Rect::new(960, 1080, 1920, 1080),
            },
        ];

        assert_eq!(
            output_at_pointer(&regions, Point::new(1200, 500)),
            Some(top_left)
        );
        assert_eq!(
            output_at_pointer(&regions, Point::new(2200, 500)),
            Some(top_right)
        );
        assert_eq!(
            output_at_pointer(&regions, Point::new(1200, 1500)),
            Some(bottom)
        );
        assert_eq!(output_at_pointer(&regions, Point::new(4000, 1500)), None);
    }

    #[test]
    fn layout_size_includes_vertical_offsets() {
        let outputs = [
            Rect::new(0, 0, 3840, 1080),
            Rect::new(960, 1080, 1920, 1080),
        ];

        assert_eq!(
            output_layout_size(outputs, Size::new(1, 1)),
            Size::new(3840, 2160)
        );
    }

    #[test]
    fn empty_layout_uses_its_callers_fallback() {
        assert_eq!(
            output_layout_size([], Size::new(1280, 800)),
            Size::new(1280, 800)
        );
    }

    #[test]
    fn layout_bounds_keep_negative_origin_for_absolute_mapping() {
        let outputs = [
            Rect::new(0, -1080, 1920, 1080),
            Rect::new(0, 0, 1920, 1080),
        ];

        assert_eq!(
            output_layout_bounds(outputs, Size::new(1, 1)),
            Rect::new(0, -1080, 1920, 2160)
        );
        assert_eq!(
            output_layout_size(outputs, Size::new(1, 1)),
            Size::new(1920, 2160)
        );
    }
}
