//! Wayland compositor runtime for the winit (nested) backend.
//!
//! The winit backend runs as a nested compositor inside an existing
//! Wayland or X11 session.

use std::process::exit;
use std::time::Duration;

use smithay::backend::input::InputEvent;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::monitor::update_geom;
use crate::startup::autostart::run_autostart;
use crate::wayland::common::{
    ensure_dbus_session, init_wayland_globals, setup_wayland_socket, spawn_wayland_smoke_window,
    spawn_xwayland,
};
use crate::wayland::input::{
    apply_pending_warp, handle_keyboard, handle_pointer_axis, handle_pointer_button,
    handle_pointer_motion, motion_event_from_winit,
};
use crate::wayland::render::winit::render_frame;
use crate::wm::Wm;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::output::Output;
use std::time::Instant;

/// Handle winit events (input, resize, close).
fn handle_winit_event(
    event: WinitEvent,
    state: &mut WaylandState,
    output: &Output,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    pointer_handle: &PointerHandle<WaylandState>,
    loop_signal: &smithay::reexports::calloop::LoopSignal,
) {
    match event {
        WinitEvent::Resized { size, .. } => {
            crate::wayland::input::handle_resize(&mut state.wm, output, size.w, size.h);
        }
        WinitEvent::Input(event) => match event {
            InputEvent::Keyboard { event } => {
                handle_keyboard(state, keyboard_handle, event);
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let size = (state.wm.g.cfg.screen_width, state.wm.g.cfg.screen_height);
                let motion_event = motion_event_from_winit(event, size.into());
                handle_pointer_motion(state, pointer_handle, keyboard_handle, motion_event);
            }
            InputEvent::PointerButton { event } => {
                let pointer_location = state.pointer_location;
                handle_pointer_button(
                    state,
                    pointer_handle,
                    keyboard_handle,
                    event,
                    pointer_location,
                );
            }
            InputEvent::PointerAxis { event } => {
                let pointer_location = state.pointer_location;
                handle_pointer_axis(
                    state,
                    pointer_handle,
                    keyboard_handle,
                    event,
                    pointer_location,
                );
            }
            _ => {}
        },
        WinitEvent::CloseRequested => {
            loop_signal.stop();
        }
        _ => {}
    }
}

/// Run one iteration of the main event loop.
fn event_loop_tick(
    state: &mut WaylandState,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut smithay::backend::renderer::damage::OutputDamageTracker,
    pointer_handle: &PointerHandle<WaylandState>,
    display_handle: &mut smithay::reexports::wayland_server::DisplayHandle,
    loop_signal: &smithay::reexports::calloop::LoopSignal,
    start_time: Instant,
) {
    let mut needs_render = false;

    if state.wm.g.dirty.layout {
        needs_render = true;
    }
    super::common::arrange_layout_if_dirty(state);
    crate::runtime::apply_monitor_config_if_dirty(&mut state.wm);

    // Winit has no libinput devices to reconfigure, but clear the
    // flag so it doesn't stay dirty forever (scroll_factor is
    // already applied at the compositor level in handle_pointer_axis).
    state.wm.g.dirty.input_config = false;

    state.popups.cleanup();
    state.refresh_popup_grab();

    // Drain and execute commands BEFORE sync/render so that
    // map/unmap from show_hide_wayland takes effect in the Space
    // before we build render elements.
    super::common::drain_and_execute_ops(state);

    if super::common::sync_space_if_dirty(state) {
        needs_render = true;
    }

    if apply_pending_warp(state, pointer_handle) {
        needs_render = true;
    }

    if state.has_active_window_animations() {
        needs_render = true;
    }

    // Surface commits from client windows set content_dirty_pending.
    if state.content_dirty_pending {
        state.content_dirty_pending = false;
        needs_render = true;
    }

    if needs_render {
        render_frame(state, backend, output, damage_tracker, start_time);
    }

    // Second drain pass for any commands queued during execute_command
    // or render (e.g. surface lifecycle callbacks).
    super::common::drain_and_execute_ops(state);

    // Sync caches
    if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
        data.backend.sync_cache(state);
    }

    if display_handle.flush_clients().is_err() {
        loop_signal.stop();
    }
}

/// Run the winit (nested) Wayland compositor.
pub fn run() -> ! {
    ensure_dbus_session();
    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        init_wayland_globals(&mut wm.g, wayland);
    }

    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();

    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut display_handle = display.handle();

    let (backend_init, winit_loop) =
        winit::init::<GlesRenderer>().expect("failed to init winit backend");
    let mut backend = Box::new(backend_init);
    let dmabuf_formats: Vec<_> = backend.renderer().dmabuf_formats().into_iter().collect();
    let egl_display = backend.renderer().egl_context().display().clone();
    let mut state = WaylandState::new(display, &loop_handle, *wm, None);

    crate::runtime::init_keyboard_layout(&mut state.wm);

    state.init_dmabuf_global(dmabuf_formats, Some(&egl_display));
    state.bind_egl_to_display(backend.renderer());
    state.init_screencopy_manager();

    let output_size = backend.window_size();
    let (initial_w, initial_h) =
        crate::wayland::common::sanitize_wayland_size(output_size.w, output_size.h);
    state.wm.g.cfg.screen_width = initial_w;
    state.wm.g.cfg.screen_height = initial_h;
    update_geom(&mut state.wm.ctx());

    let output = state.create_output("winit", initial_w, initial_h);
    let mut damage_tracker =
        smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);

    // Clone handles to avoid borrow conflicts when passing to handlers
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);

    // Initialize Wayland systray runtime - only applicable for Wayland backend
    if let WmBackend::Wayland(data) = &mut state.wm.backend {
        data.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();
    }

    run_autostart();
    spawn_wayland_smoke_window();
    let ipc_server = crate::ipc::IpcServer::bind().ok();

    // Setup IPC as event-driven calloop source
    if let Some(ipc) = ipc_server {
        super::calloop_helpers::setup_ipc_source(loop_handle.clone(), ipc, move |ipc, state| {
            if ipc.process_pending(&mut state.wm) {
                state.wm.g.dirty.layout = true;
                crate::runtime::apply_monitor_config_if_dirty(&mut state.wm);
                state.wm.g.dirty.space = true;
            }
        });
    }

    // Setup animation timer
    super::calloop_helpers::setup_animation_timer(
        loop_handle.clone(),
        |state| state.tick_window_animations(),
        |state| state.has_active_window_animations(),
    );

    // Register winit as a proper calloop event source
    let loop_signal = event_loop.get_signal();
    let loop_signal_for_winit = loop_signal.clone();
    let output_for_winit = output.clone();
    let keyboard_handle_for_winit = keyboard_handle.clone();
    let pointer_handle_for_winit = pointer_handle.clone();
    loop_handle
        .insert_source(winit_loop, move |event, (), state| {
            handle_winit_event(
                event,
                state,
                &output_for_winit,
                &keyboard_handle_for_winit,
                &pointer_handle_for_winit,
                &loop_signal_for_winit,
            );
        })
        .expect("failed to insert winit event source");

    let start_time = Instant::now();
    crate::runtime::spawn_status_bar(&state.wm);

    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            event_loop_tick(
                state,
                &mut backend,
                &output,
                &mut damage_tracker,
                &pointer_handle,
                &mut display_handle,
                &loop_signal,
                start_time,
            );
        })
        .expect("wayland event loop run");
    exit(0);
}
