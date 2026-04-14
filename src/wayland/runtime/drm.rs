//! DRM/KMS bare-metal backend for running directly on hardware.

use smithay::backend::drm::DrmEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::libinput::LibinputSessionInterface;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::calloop::{EventLoop, LoopHandle, LoopSignal};
use smithay::reexports::drm::control::crtc;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{Clock, Monotonic};
use smithay::wayland::presentation::Refresh;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::process::exit;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::backend::Backend as WmBackend;
use crate::backend::BackendVrrSupport;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::CursorConfig;
use crate::config::config_toml::VrrMode;
use crate::startup::autostart::run_autostart;
use crate::wayland::common::{build_fixed_scene_elements, poll_wayland_systray};
use crate::wayland::common::{
    ensure_dbus_session, init_wayland_globals, send_frame_callbacks, setup_wayland_socket,
    spawn_wayland_smoke_window, spawn_xwayland,
};
use crate::wayland::init::drm::init_gpu;
use crate::wayland::input::apply_pending_warp;
use crate::wayland::render::drm::{
    CursorManager, ManagedDrmOutputManager, OutputHitRegion, OutputSurfaceEntry, RenderOutcome,
    build_output_surfaces, create_output_manager, render_drm_output,
};
use crate::wm::Wm;

#[derive(Debug)]
struct DrmLayoutState {
    total_width: i32,
    total_height: i32,
    output_hit_regions: Vec<OutputHitRegion>,
}

#[derive(Debug)]
struct DrmLoopState {
    session_active: bool,
    render_flags: HashMap<crtc::Handle, bool>,
    pending_crtcs: HashSet<crtc::Handle>,
    empty_frame_callback_crtcs: Rc<RefCell<HashSet<crtc::Handle>>>,
    presentation_seq: HashMap<crtc::Handle, u64>,
    last_bar_update_seq: u64,
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
            pending_crtcs: HashSet::new(),
            empty_frame_callback_crtcs: Rc::new(RefCell::new(HashSet::new())),
            presentation_seq: output_surfaces
                .iter()
                .map(|entry| (entry.crtc, 0))
                .collect(),
            last_bar_update_seq: 0,
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

    fn mark_pointer_output_dirty(&mut self, px: i32, layout: &DrmLayoutState) {
        for entry in &layout.output_hit_regions {
            if px >= entry.x_offset && px < entry.x_offset + entry.width {
                self.mark_dirty(entry.crtc);
                return;
            }
        }
        self.mark_all_dirty();
    }

    fn take_render_flags(&mut self) -> HashMap<crtc::Handle, bool> {
        let flags = self.render_flags.clone();
        for flag in self.render_flags.values_mut() {
            *flag = false;
        }
        flags
    }

    fn has_renderable_dirty_outputs(&self) -> bool {
        self.render_flags
            .iter()
            .any(|(crtc, &dirty)| dirty && !self.pending_crtcs.contains(crtc))
    }
}

#[derive(Debug, Clone, Copy)]
enum DrmRuntimeEvent {
    SessionPaused,
    SessionActivated,
    VBlank(crtc::Handle),
    PointerActivity(i32),
}

// WARNING: This function is extremely fragile, do not refactor or mess with it without
// great care and patience for random ass segfaults. Yes, this is awful, leave it.
// Hours spent on this: ~3h
pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");
    ensure_dbus_session();

    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        init_wayland_globals(&mut wm.g, wayland);
    }

    let event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut state = WaylandState::new(display, &loop_handle);
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.backend.attach_state(&mut state);
    }

    crate::runtime::init_keyboard_layout(&mut wm);

    let (
        primary_gpu_path,
        drm_device,
        drm_notifier,
        _drm_fd,
        gbm_device,
        egl_display,
        mut renderer,
    ) = init_gpu(&mut session, &seat_name);
    log::info!("Using GPU: {:?}", primary_gpu_path);

    state.attach_renderer(&mut renderer);
    state.init_dmabuf_global(
        renderer.dmabuf_formats().into_iter().collect(),
        Some(&egl_display),
    );
    state.init_screencopy_manager();
    state.attach_wm(&mut wm);

    let cursor_manager = init_cursor_manager(&state.cursor_config);
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
        state.space.map_output(&entry.output, (entry.x_offset, 0));
    }

    let (total_width, total_height) = compute_total_dimensions(&output_surfaces);

    {
        use crate::monitor::update_geom;
        update_geom(&mut wm.ctx());
    }
    crate::monitor::apply_monitor_config(&mut wm.ctx());

    let layout_state = Arc::new(init_layout_state(
        &output_surfaces,
        total_width,
        total_height,
    ));
    let mut loop_state = DrmLoopState::new(&output_surfaces);
    let (runtime_event_tx, runtime_event_rx) = mpsc::channel();

    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);

    // Initialize Wayland systray runtime - only applicable for Wayland backend
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();
    }

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");

    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());
    let shared_layout = Arc::clone(&layout_state);
    let runtime_event_tx_input = runtime_event_tx.clone();
    loop_handle
        .insert_source(libinput_backend, move |event, _, state| {
            let total_w = shared_layout.total_width;
            let total_h = shared_layout.total_height;

            let any_input = state
                .with_wm_mut_unified(|wm, state| {
                    crate::wayland::input::drm::dispatch_libinput_event(
                        event, state, wm, total_w, total_h,
                    )
                })
                .unwrap_or(false);

            if any_input {
                let _ = runtime_event_tx_input.send(DrmRuntimeEvent::PointerActivity(
                    state.runtime.pointer_location.x as i32,
                ));
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

    run_autostart();
    spawn_wayland_smoke_window();

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    // Register IPC listener fd so the event loop wakes on incoming commands.
    crate::runtime::register_ipc_source(&event_loop.handle(), &ipc_server);

    let (status_ping, status_ping_source) = calloop::ping::make_ping().expect("status ping");
    crate::bar::status::set_internal_status_ping(status_ping);
    event_loop
        .handle()
        .insert_source(status_ping_source, |_, _, _| {})
        .expect("status ping source");

    // Ping source for initial frame kick, explicit redraw requests and render-failure retries.
    let (retry_ping, retry_ping_source) = calloop::ping::make_ping().expect("ping");
    event_loop
        .handle()
        .insert_source(retry_ping_source, |_, _, state| {
            state.runtime.render_dirty = true;
        })
        .expect("ping source");
    state.runtime.render_ping = Some(retry_ping.clone());
    retry_ping.ping(); // Wake loop once to render the initial frame

    let start_time = std::time::Instant::now();
    let mut render_failures: HashMap<crtc::Handle, u32> = HashMap::new();

    crate::runtime::spawn_status_bar(&wm);

    let (led_state_tx, led_state_rx) = std::sync::mpsc::channel();
    state.runtime.led_state_tx = Some(led_state_tx);

    run_event_loop(
        event_loop,
        &mut wm,
        &mut state,
        &layout_state,
        &mut loop_state,
        &mut output_surfaces,
        &mut renderer,
        &cursor_manager,
        &mut ipc_server,
        &mut render_failures,
        start_time,
        led_state_rx,
        runtime_event_rx,
        retry_ping,
    );

    exit(0);
}

/// Initialize cursor manager from environment or defaults.
fn init_cursor_manager(config: &CursorConfig) -> CursorManager {
    let cursor_theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| config.theme.clone());
    let cursor_size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(config.size);
    CursorManager::new(&cursor_theme, cursor_size as u8)
}

/// Compute total screen dimensions from output surfaces.
fn compute_total_dimensions(output_surfaces: &[OutputSurfaceEntry]) -> (i32, i32) {
    let total_width = output_surfaces
        .iter()
        .map(|s| s.x_offset + s.width)
        .max()
        .unwrap_or(crate::wayland::render::drm::DEFAULT_SCREEN_WIDTH);
    let total_height = output_surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(crate::wayland::render::drm::DEFAULT_SCREEN_HEIGHT);
    (total_width, total_height)
}

fn init_layout_state(
    output_surfaces: &[OutputSurfaceEntry],
    total_width: i32,
    total_height: i32,
) -> DrmLayoutState {
    DrmLayoutState {
        total_width,
        total_height,
        output_hit_regions: output_surfaces
            .iter()
            .map(|entry| OutputHitRegion {
                crtc: entry.crtc,
                x_offset: entry.x_offset,
                width: entry.width,
            })
            .collect(),
    }
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
        .insert_source(notifier, move |event, _, _data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away) - suspending rendering");
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
    layout_state: &Arc<DrmLayoutState>,
    loop_state: &mut DrmLoopState,
    output_surfaces: &mut [OutputSurfaceEntry],
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    render_failures: &mut HashMap<crtc::Handle, u32>,
    start_time: std::time::Instant,
    led_state_rx: std::sync::mpsc::Receiver<smithay::input::keyboard::LedState>,
    runtime_event_rx: mpsc::Receiver<DrmRuntimeEvent>,
    retry_ping: calloop::ping::Ping,
) {
    let loop_signal: LoopSignal = event_loop.get_signal();
    let loop_handle = event_loop.handle();
    let pointer_handle = state.pointer.clone();
    let anim_guard = crate::runtime::AnimationTimerGuard::new();
    let shared_layout = Arc::clone(layout_state);
    let monotonic_clock = Clock::<Monotonic>::new();

    event_loop
        .run(None, state, move |state| {
            process_runtime_events(
                &runtime_event_rx,
                loop_state,
                &shared_layout,
                output_surfaces,
                &monotonic_clock,
            );
            process_commit_redraws(state, loop_state);
            process_common_tick(ipc_server, wm, state, loop_state);
            sync_output_vrr_modes_from_state(state, output_surfaces, loop_state);

            let bar_update_seq = wm.bar.update_seq();
            if loop_state.last_bar_update_seq != bar_update_seq {
                loop_state.last_bar_update_seq = bar_update_seq;
                loop_state.mark_all_dirty();
            }

            if wm.g.pending.input_config {
                wm.g.pending.input_config = false;
                crate::wayland::input::drm::reconfigure_all_devices(
                    &mut state.runtime.tracked_devices,
                    &wm.g.cfg.input,
                );
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

            process_animations(state, loop_state);

            // Arm an on-demand animation timer when animations are active.
            anim_guard.ensure_armed(
                state.has_active_window_animations(),
                &loop_handle,
                move |state| state.has_active_window_animations(),
            );

            process_cursor_warp(state, &pointer_handle, loop_state);

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

            // If an output can be rendered immediately after a failure, ping
            // the loop to retry. Dirty outputs with a page flip in flight are
            // woken by the DRM vblank source instead; self-pinging there spins.
            if loop_state.has_renderable_dirty_outputs() {
                retry_ping.ping();
            }

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
) {
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
                if let Some(entry) = output_surfaces.iter_mut().find(|entry| entry.crtc == crtc) {
                    match entry.surface.frame_submitted() {
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
            DrmRuntimeEvent::PointerActivity(px) => {
                loop_state.mark_pointer_output_dirty(px, layout_state);
            }
        }
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

fn output_frame_callback_delay(entry: &OutputSurfaceEntry) -> Duration {
    entry
        .output
        .current_mode()
        .and_then(|mode| {
            let refresh = u64::try_from(mode.refresh).ok()?;
            (refresh > 0).then(|| Duration::from_nanos(1_000_000_000_000u64 / refresh))
        })
        .unwrap_or_else(|| Duration::from_millis(16))
}

fn arm_empty_frame_callback_timer(
    loop_handle: &LoopHandle<'_, WaylandState>,
    loop_state: &DrmLoopState,
    entry: &OutputSurfaceEntry,
    start_time: std::time::Instant,
) {
    let crtc = entry.crtc;
    let armed = Rc::clone(&loop_state.empty_frame_callback_crtcs);
    if !armed.borrow_mut().insert(crtc) {
        return;
    }

    let output = entry.output.clone();
    let delay = output_frame_callback_delay(entry);
    let armed_for_timer = Rc::clone(&armed);
    if let Err(err) = loop_handle.insert_source(
        calloop::timer::Timer::from_duration(delay),
        move |_, _, state| {
            if armed_for_timer.borrow_mut().remove(&crtc) {
                send_frame_callbacks(state, &output, start_time.elapsed());
            }
            calloop::timer::TimeoutAction::Drop
        },
    ) {
        armed.borrow_mut().remove(&crtc);
        log::warn!(
            "failed to arm empty-frame callback timer for {:?}: {err}",
            crtc
        );
    }
}

/// Promote compositor-side redraw requests into DRM output dirties.
fn process_commit_redraws(state: &mut WaylandState, loop_state: &mut DrmLoopState) {
    if state.take_render_dirty() {
        loop_state.mark_all_dirty();
    }
}

/// Run the shared Wayland tick, then apply DRM-specific invalidation.
fn process_common_tick(
    ipc_server: &mut Option<crate::ipc::IpcServer>,
    wm: &mut Wm,
    state: &WaylandState,
    loop_state: &mut DrmLoopState,
) {
    let tick = super::common::event_loop_tick(wm, state, ipc_server);
    if tick.ipc_handled || tick.monitor_config_applied || tick.layout_applied {
        loop_state.mark_all_dirty();
    }
}

/// Process window animations and pending compositor-space sync.
fn process_animations(state: &mut WaylandState, loop_state: &mut DrmLoopState) {
    if super::common::process_window_animations(state) {
        // DRM-specific: mark all outputs dirty after space sync
        loop_state.mark_all_dirty();
    }
}

/// Apply compositor-side cursor warp.
fn process_cursor_warp(
    state: &mut WaylandState,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    loop_state: &mut DrmLoopState,
) {
    if apply_pending_warp(state, pointer_handle) {
        loop_state.mark_all_dirty();
    }
}

fn sync_output_vrr_modes_from_state(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    loop_state: &mut DrmLoopState,
) {
    let mut changed = false;
    for entry in output_surfaces.iter_mut() {
        let output_name = entry.output.name();
        if let Some(metadata) = state.output_vrr_metadata(&output_name)
            && entry.configured_vrr_mode != metadata.vrr_mode
        {
            entry.configured_vrr_mode = metadata.vrr_mode;
            log::info!(
                "Output {}: VRR mode set to {:?} (support: {:?})",
                output_name,
                entry.configured_vrr_mode,
                entry.vrr_support
            );
            changed = true;
        }
    }
    if changed {
        loop_state.mark_all_dirty();
    }
}

fn has_pending_screencopy_for_output(state: &WaylandState, output_name: &str) -> bool {
    state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output.name() == output_name)
}

fn auto_vrr_content_is_suitable(wm: &Wm, output_name: &str) -> bool {
    let Some(mon) = wm.g.monitors_iter_all().find(|m| m.name == output_name) else {
        return false;
    };
    if mon.current_layout().is_overview() {
        return false;
    }

    let selected = mon.selected_tags();
    let mut visible_clients = mon
        .iter_clients(wm.g.clients.map())
        .filter(|(_, client)| client.is_visible(selected))
        .filter(|(_, client)| !client.is_scratchpad())
        .collect::<Vec<_>>();

    if visible_clients.len() != 1 {
        return false;
    }

    visible_clients
        .pop()
        .is_some_and(|(_, client)| client.is_true_fullscreen())
}

fn compute_output_vrr_target(wm: &Wm, state: &WaylandState, entry: &OutputSurfaceEntry) -> bool {
    let output_name = entry.output.name();

    match entry.vrr_support {
        BackendVrrSupport::Unsupported => false,
        BackendVrrSupport::RequiresModeset => matches!(entry.configured_vrr_mode, VrrMode::On),
        BackendVrrSupport::Supported => {
            let hard_blocked = state.is_locked()
                || state.has_active_window_animations()
                || has_pending_screencopy_for_output(state, &output_name)
                || !state.overlay_windows_for_render(entry.x_offset).is_empty()
                || !matches!(
                    state.cursor_image_status,
                    smithay::input::pointer::CursorImageStatus::Named(_)
                        | smithay::input::pointer::CursorImageStatus::Hidden
                )
                || state.runtime.dnd_icon.is_some();

            if hard_blocked {
                return false;
            }

            match entry.configured_vrr_mode {
                VrrMode::Off => false,
                VrrMode::On => true,
                VrrMode::Auto => auto_vrr_content_is_suitable(wm, &output_name),
            }
        }
    }
}

fn apply_output_vrr_policy(wm: &Wm, state: &mut WaylandState, entry: &mut OutputSurfaceEntry) {
    let target = compute_output_vrr_target(wm, state, entry);
    if entry.vrr_enabled == target {
        state.set_output_vrr_enabled(&entry.output.name(), entry.vrr_enabled);
        return;
    }

    match entry
        .surface
        .with_compositor(|compositor| compositor.use_vrr(target))
    {
        Ok(()) => {
            entry.vrr_enabled = target;
            state.set_output_vrr_enabled(&entry.output.name(), target);
            log::info!(
                "Output {}: VRR {} (mode: {:?}, support: {:?})",
                entry.output.name(),
                if target { "enabled" } else { "disabled" },
                entry.configured_vrr_mode,
                entry.vrr_support
            );
        }
        Err(err) => {
            state.set_output_vrr_enabled(&entry.output.name(), entry.vrr_enabled);
            log::warn!(
                "Output {}: failed to set VRR {}: {:?}",
                entry.output.name(),
                if target { "on" } else { "off" },
                err
            );
        }
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
    start_time: std::time::Instant,
) {
    let render_flags = loop_state.take_render_flags();
    let session_active = loop_state.session_active;
    let pending_crtcs = loop_state.pending_crtcs.clone();

    let pointer_location = state.runtime.pointer_location;

    if session_active {
        let needs_any_render = output_surfaces
            .iter()
            .any(|entry| render_flags.get(&entry.crtc).copied().unwrap_or(false));
        let fixed_scene = if needs_any_render && !state.is_locked() {
            poll_wayland_systray(wm);
            Some(build_fixed_scene_elements(wm, state))
        } else {
            None
        };

        for entry in output_surfaces.iter_mut() {
            let needs_render = render_flags.get(&entry.crtc).copied().unwrap_or(false);
            if !needs_render {
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
            let rendered = render_drm_output(
                state,
                renderer,
                entry,
                cursor_manager,
                pointer_location,
                start_time,
                fixed_scene.clone(),
            );

            match rendered {
                RenderOutcome::Submitted => {
                    loop_state
                        .empty_frame_callback_crtcs
                        .borrow_mut()
                        .remove(&entry.crtc);
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
                    arm_empty_frame_callback_timer(loop_handle, loop_state, entry, start_time);
                    render_failures.remove(&entry.crtc);
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

                    loop_state.mark_dirty(entry.crtc);
                }
            }
        }
    }
}
