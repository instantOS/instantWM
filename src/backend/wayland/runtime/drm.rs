//! DRM/KMS bare-metal backend for running directly on hardware.

use smithay::backend::drm::DrmEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::backend::libinput::LibinputSessionInterface;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
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
use std::time::Instant;

use crate::backend::BackendVrrSupport;
use crate::backend::output::{
    AdaptiveSyncPolicy, OutputHeadCapabilities, OutputHeadConfiguration, OutputHeadSnapshot,
    OutputMode as TransactionOutputMode, OutputSnapshot, OutputTransaction, OutputTransactionError,
    OutputTransactionKind,
};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::init::drm::init_gpu;
use crate::backend::wayland::input::apply_pending_warp;
use crate::backend::wayland::render::cursor::{CursorPresentation, resolve_cursor_presentation};
use crate::backend::wayland::render::drm::{
    CursorManager, ManagedDrmOutputManager, OutputHitRegion, OutputSurfaceEntry, RenderOutcome,
    build_output_surfaces, create_output_manager, render_drm_output,
};
use crate::backend::wayland::render::scene::{
    SceneCache, build_shared_scene_elements, poll_systray,
};
use crate::config::config_toml::CursorConfig;
use crate::config::config_toml::VrrMode;
use crate::wm::Wm;

#[derive(Debug)]
struct DrmLayoutState {
    total_size: crate::types::Size,
    output_hit_regions: Vec<OutputHitRegion>,
}

struct DrmLoopState {
    session_active: bool,
    render_flags: HashMap<crtc::Handle, bool>,
    taken_render_flags: HashMap<crtc::Handle, bool>,
    pending_crtcs: HashSet<crtc::Handle>,
    frame_callback_timers: super::engine::FrameCallbackTimerGuard<crtc::Handle>,
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
            frame_callback_timers: super::engine::FrameCallbackTimerGuard::default(),
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
}

#[derive(Debug, Clone, Copy)]
enum DrmRuntimeEvent {
    SessionPaused,
    SessionActivated,
    VBlank(crtc::Handle),
    PointerMoved { old_x: i32 },
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

    super::bootstrap::attach_backend_state(&mut wm, &mut state);

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

    let total_size = compute_total_dimensions(&output_surfaces);

    {
        use crate::monitor::refresh_monitor_layout;
        refresh_monitor_layout(&mut wm.ctx());
    }
    state.push_command(crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones);
    crate::monitor::apply_monitor_config(&mut wm.ctx());

    let mut layout_state = init_layout_state(&output_surfaces, total_size);
    // Calloop dispatches sources and the loop callback sequentially on this
    // thread. Libinput only needs the current dimensions, so share that small
    // copy without putting the complete layout behind an atomic lock.
    let input_dimensions = Rc::new(Cell::new(total_size));
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
            let dimensions = shared_input_dimensions.get();
            let total_w = dimensions.w;
            let total_h = dimensions.h;

            // SAFETY: calloop source callback runs synchronously within
            // event_loop.dispatch(); the &mut Wm borrow in the main body
            // has not yet resumed.
            let old_pointer_x = state.runtime.pointer_location.x as i32;
            let outcome = if let Some(wm_ptr) = unsafe { state.wm_mut_ptr() } {
                let wm = unsafe { &mut *wm_ptr };
                crate::backend::wayland::input::drm::dispatch_libinput_event(
                    event, state, wm, total_w, total_h,
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
                        old_x: old_pointer_x,
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

/// Compute total screen dimensions from output surfaces.
fn compute_total_dimensions(output_surfaces: &[OutputSurfaceEntry]) -> crate::types::Size {
    let total_width = output_surfaces
        .iter()
        .map(|surface| surface.rect.x + surface.rect.w)
        .max()
        .unwrap_or(crate::backend::wayland::render::drm::DEFAULT_SCREEN_WIDTH);
    let total_height = output_surfaces
        .iter()
        .map(|surface| surface.rect.h)
        .max()
        .unwrap_or(crate::backend::wayland::render::drm::DEFAULT_SCREEN_HEIGHT);
    crate::types::Size::new(total_width, total_height)
}

fn init_layout_state(
    output_surfaces: &[OutputSurfaceEntry],
    total_size: crate::types::Size,
) -> DrmLayoutState {
    DrmLayoutState {
        total_size,
        output_hit_regions: output_surfaces
            .iter()
            .map(|entry| OutputHitRegion {
                crtc: entry.crtc,
                x_offset: entry.rect.x,
                width: entry.rect.w,
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
    let total_size = crate::types::Size::new(
        active
            .iter()
            .map(|entry| entry.rect.x + entry.rect.w)
            .max()
            .unwrap_or(1)
            .max(1),
        active
            .iter()
            .map(|entry| entry.rect.y + entry.rect.h)
            .max()
            .unwrap_or(1)
            .max(1),
    );
    *layout_state = DrmLayoutState {
        total_size,
        output_hit_regions: active
            .iter()
            .map(|entry| OutputHitRegion {
                crtc: entry.crtc,
                x_offset: entry.rect.x,
                width: entry.rect.w,
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

/// Extract a `CursorIcon` from a resolved cursor presentation for
/// animation-timer scheduling.  `Surface` cursors are client-owned and
/// cannot be introspected, so they return `None`.
fn cursor_presentation_icon(p: &CursorPresentation) -> Option<CursorIcon> {
    match p {
        CursorPresentation::Hidden | CursorPresentation::Surface { .. } => None,
        CursorPresentation::Named(icon) => Some(*icon),
        CursorPresentation::DndIcon { cursor, .. } => cursor_presentation_icon(cursor),
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
    input_dimensions: &Rc<Cell<crate::types::Size>>,
    loop_state: &mut DrmLoopState,
    output_surfaces: &mut [OutputSurfaceEntry],
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
    let anim_guard = crate::runtime::AnimationTimerGuard::new();
    let render_retry_guard = crate::runtime::AnimationTimerGuard::new();
    let shared_input_dimensions = Rc::clone(input_dimensions);
    let monotonic_clock = Clock::<Monotonic>::new();

    event_loop
        .run(None, state, move |state| {
            let pointer_moved = process_runtime_events(
                &runtime_event_rx,
                loop_state,
                layout_state,
                output_surfaces,
                &monotonic_clock,
            );
            process_frame_callback_requests(
                state,
                &loop_handle,
                loop_state,
                output_surfaces,
                start_time,
            );
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
                shared_input_dimensions.set(layout_state.total_size);
            }
            if pointer_moved {
                loop_state.mark_pointer_output_dirty(
                    state.runtime.pointer_location.x as i32,
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

            // Resolve cursor animation state so the on-demand timer keeps
            // animated cursors (e.g. the spinning "wait" cursor) alive
            // even when the system is otherwise idle.
            let animated = {
                let presentation = resolve_cursor_presentation(
                    &state.cursor_image_status,
                    state.cursor_icon_override,
                    state.runtime.dnd_icon.as_ref(),
                    state.runtime.cursor_hidden_by_touch,
                );
                cursor_presentation_icon(&presentation)
                    .is_some_and(|icon| cursor_manager.is_animated(icon, 1))
            };
            state.runtime.cursor_is_animated = animated;
            if animated {
                loop_state.mark_pointer_output_dirty(
                    state.runtime.pointer_location.x as i32,
                    layout_state,
                );
            }

            // Arm an on-demand animation timer when animations are active.
            let has_anim = state.has_active_animations() || animated;
            anim_guard.ensure_armed(has_anim, &loop_handle, move |state| {
                state.has_active_animations() || state.runtime.cursor_is_animated
            });

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
) -> bool {
    let mut pointer_moved = false;
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
            DrmRuntimeEvent::PointerMoved { old_x } => {
                loop_state.mark_pointer_output_dirty(old_x, layout_state);
                pointer_moved = true;
            }
        }
    }
    pointer_moved
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
            loop_state.frame_callback_timers.arm(
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

fn requested_mode(
    entry: &OutputSurfaceEntry,
    config: &OutputHeadConfiguration,
) -> Option<smithay::reexports::drm::control::Mode> {
    let requested = config.mode?;
    entry.modes.iter().find_map(|(output_mode, drm_mode)| {
        (output_mode.size.w == requested.width
            && output_mode.size.h == requested.height
            && output_mode.refresh == requested.refresh_millihertz)
            .then_some(*drm_mode)
    })
}

fn transaction_snapshot(
    transaction: &OutputTransaction,
    output_surfaces: &[OutputSurfaceEntry],
) -> OutputSnapshot {
    OutputSnapshot {
        heads: transaction
            .heads
            .iter()
            .map(|configuration| {
                let entry = output_surfaces
                    .iter()
                    .find(|entry| entry.output.name() == configuration.id.0);
                let modes = entry
                    .map(|entry| {
                        entry
                            .modes
                            .iter()
                            .map(|(mode, _)| TransactionOutputMode {
                                width: mode.size.w,
                                height: mode.size.h,
                                refresh_millihertz: mode.refresh,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let adaptive_sync_policy = configuration.adaptive_sync.unwrap_or_else(|| {
                    entry.map_or(AdaptiveSyncPolicy::Disabled, |entry| {
                        match entry.configured_vrr_mode {
                            VrrMode::Off => AdaptiveSyncPolicy::Disabled,
                            VrrMode::On => AdaptiveSyncPolicy::Enabled,
                            VrrMode::Auto => AdaptiveSyncPolicy::Automatic,
                        }
                    })
                });
                OutputHeadSnapshot {
                    configuration: configuration.clone(),
                    modes,
                    adaptive_sync_policy,
                    adaptive_sync_enabled: entry.is_some_and(|entry| entry.vrr_enabled),
                }
            })
            .collect(),
    }
}

fn output_capabilities(output_surfaces: &[OutputSurfaceEntry]) -> Vec<OutputHeadCapabilities> {
    output_surfaces
        .iter()
        .map(|entry| OutputHeadCapabilities {
            id: entry.output.name().as_str().into(),
            modes: entry
                .modes
                .iter()
                .map(|(mode, _)| TransactionOutputMode {
                    width: mode.size.w,
                    height: mode.size.h,
                    refresh_millihertz: mode.refresh,
                })
                .collect(),
            adaptive_sync: !matches!(entry.vrr_support, BackendVrrSupport::Unsupported),
        })
        .collect()
}

fn requested_vrr(
    entry: &OutputSurfaceEntry,
    configuration: &OutputHeadConfiguration,
) -> (VrrMode, bool) {
    let policy = configuration
        .adaptive_sync
        .unwrap_or(match entry.configured_vrr_mode {
            VrrMode::Off => AdaptiveSyncPolicy::Disabled,
            VrrMode::On => AdaptiveSyncPolicy::Enabled,
            VrrMode::Auto => AdaptiveSyncPolicy::Automatic,
        });
    match policy {
        AdaptiveSyncPolicy::Disabled => (VrrMode::Off, false),
        AdaptiveSyncPolicy::Enabled => (VrrMode::On, true),
        AdaptiveSyncPolicy::Automatic => (VrrMode::Auto, entry.vrr_enabled),
    }
}

fn process_output_configurations(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    output_manager: &Arc<Mutex<ManagedDrmOutputManager>>,
    renderer: &mut GlesRenderer,
    loop_state: &mut DrmLoopState,
) {
    if !state.runtime.output_transactions.has_pending() {
        return;
    }

    let render_elements = smithay::backend::drm::output::DrmOutputRenderElements::<
        GlesRenderer,
        crate::backend::wayland::render::drm::DrmExtras,
    >::default();
    let capabilities = output_capabilities(output_surfaces);

    while let Some(pending) = state.runtime.output_transactions.take_next_pending() {
        if let Err(error) = pending.transaction.validate(&capabilities) {
            state
                .runtime
                .output_transactions
                .complete(pending, Err(error));
            continue;
        }
        if pending.kind == OutputTransactionKind::Test {
            let snapshot = transaction_snapshot(&pending.transaction, output_surfaces);
            state
                .runtime
                .output_transactions
                .complete(pending, Ok(snapshot));
            continue;
        }

        let requested: Vec<_> = pending
            .transaction
            .heads
            .iter()
            .map(|config| {
                let index = output_surfaces
                    .iter()
                    .position(|entry| entry.output.name() == config.id.0)
                    .expect("validated output disappeared before application");
                (
                    index,
                    config,
                    requested_mode(&output_surfaces[index], config),
                )
            })
            .collect();
        if requested.iter().any(|(index, _, _)| {
            loop_state
                .pending_crtcs
                .contains(&output_surfaces[*index].crtc)
        }) {
            state.runtime.output_transactions.requeue(pending);
            break;
        }

        let mut newly_enabled = Vec::new();
        let mut changed_modes = Vec::new();
        let mut changed_vrr = Vec::new();
        let mut applied = true;

        for (index, config, mode) in &requested {
            if !config.enabled {
                continue;
            }
            let entry = &mut output_surfaces[*index];
            let mode = mode.expect("enabled configurations were prevalidated");
            let was_enabled = entry.surface.is_some();
            if let Some(surface) = entry.surface.as_mut() {
                let current_mode = surface.with_compositor(|compositor| compositor.current_mode());
                if current_mode != mode {
                    if surface.use_mode(mode, renderer, &render_elements).is_err() {
                        applied = false;
                        break;
                    }
                    changed_modes.push((*index, current_mode));
                }
            } else {
                let mut manager = output_manager.lock().unwrap();
                match manager.lock().initialize_output(
                    entry.crtc,
                    mode,
                    &[entry.connector],
                    &entry.output,
                    None,
                    renderer,
                    &render_elements,
                ) {
                    Ok(surface) => {
                        entry.surface = Some(surface);
                        newly_enabled.push(*index);
                    }
                    Err(error) => {
                        log::warn!("failed to enable output {}: {error:?}", entry.output.name());
                        applied = false;
                        break;
                    }
                }
            }
            let (_, adaptive_sync) = requested_vrr(entry, config);
            let current_vrr = entry
                .surface
                .as_ref()
                .expect("output was enabled above")
                .with_compositor(|compositor| compositor.vrr_enabled());
            if current_vrr != adaptive_sync {
                if entry
                    .surface
                    .as_ref()
                    .expect("output was enabled above")
                    .with_compositor(|compositor| compositor.use_vrr(adaptive_sync))
                    .is_err()
                {
                    applied = false;
                    break;
                }
                if was_enabled {
                    changed_vrr.push((*index, current_vrr));
                }
            }
        }

        if !applied {
            for index in newly_enabled {
                output_surfaces[index].surface.take();
            }
            for (index, old_mode) in changed_modes {
                if let Some(surface) = output_surfaces[index].surface.as_mut() {
                    let _ = surface.use_mode(old_mode, renderer, &render_elements);
                }
            }
            for (index, old_vrr) in changed_vrr {
                if let Some(surface) = output_surfaces[index].surface.as_ref() {
                    let _ = surface.with_compositor(|compositor| compositor.use_vrr(old_vrr));
                }
            }
            state.runtime.output_transactions.complete(
                pending,
                Err(OutputTransactionError::Backend(
                    "DRM could not commit the requested output state".to_string(),
                )),
            );
            continue;
        }

        for (index, config, _) in &requested {
            let entry = &mut output_surfaces[*index];
            if !config.enabled {
                entry.surface.take();
                entry.enabled = false;
                entry.vrr_enabled = false;
            } else {
                entry.enabled = true;
                let (mode, enabled) = requested_vrr(entry, config);
                entry.vrr_enabled = enabled;
                entry.configured_vrr_mode = mode;
            }
        }
        let snapshot = transaction_snapshot(&pending.transaction, output_surfaces);
        state
            .runtime
            .output_transactions
            .complete(pending, Ok(snapshot));
        loop_state.mark_all_dirty();
        // Project the authoritative state before attempting another apply.
        break;
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
    let Some(mon) = wm
        .core
        .model
        .monitors_iter_all()
        .find(|m| m.name == output_name)
    else {
        return false;
    };
    if matches!(
        wm.core.behavior.current_mode,
        crate::core_state::ActiveWmMode::Overview
    ) && wm.core.model.selected_monitor_id() == mon.id()
    {
        return false;
    }

    let selected = mon.selected_tags();
    let mut visible_clients = mon
        .iter_clients(&wm.core.model.clients)
        .filter(|(_, client)| client.is_visible(selected) && !client.is_scratchpad());

    let Some((_, first_client)) = visible_clients.next() else {
        return false;
    };

    if visible_clients.next().is_some() {
        return false;
    }

    first_client.mode().is_true_fullscreen()
}

fn compute_output_vrr_target(wm: &Wm, state: &WaylandState, entry: &OutputSurfaceEntry) -> bool {
    let output_name = entry.output.name();

    match entry.vrr_support {
        BackendVrrSupport::Unsupported => false,
        BackendVrrSupport::RequiresModeset => matches!(entry.configured_vrr_mode, VrrMode::On),
        BackendVrrSupport::Supported => {
            let hard_blocked = state.is_locked()
                || state.has_window_animations_on_output(&entry.output)
                || state.has_active_layout_preview_animation()
                || has_pending_screencopy_for_output(state, &output_name)
                || !state.overlay_windows_for_render(&entry.output).is_empty()
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
        .as_mut()
        .expect("enabled DRM output has a surface")
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
            poll_systray(wm);
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
            if !needs_render || !entry.enabled {
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
                    loop_state.frame_callback_timers.disarm(&entry.crtc);
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
                    loop_state.frame_callback_timers.arm(
                        entry.crtc,
                        loop_handle,
                        &entry.output,
                        start_time,
                    );
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
