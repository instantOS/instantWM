//! Wayland compositor runtime for the winit (nested) backend.
//!
//! The winit backend runs as a nested compositor inside an existing
//! Wayland or X11 session.

use std::process::exit;

use smithay::backend::input::InputEvent;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
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
    let mut state = WaylandState::new(display, &loop_handle);
    state.attach_wm(&mut wm);
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.backend.attach_state(&mut state);
    }

    crate::runtime::init_keyboard_layout(&mut wm);

    let (backend_init, winit_loop) =
        winit::init::<GlesRenderer>().expect("failed to init winit backend");
    let mut backend = Box::new(backend_init);
    state.attach_renderer(backend.renderer());
    state.init_dmabuf_global(
        backend.renderer().dmabuf_formats().into_iter().collect(),
        Some(backend.renderer().egl_context().display()),
    );
    state.init_screencopy_manager();

    let output_size = backend.window_size();
    let (initial_w, initial_h) =
        crate::wayland::common::sanitize_wayland_size(output_size.w, output_size.h);
    wm.g.cfg.screen_width = initial_w;
    wm.g.cfg.screen_height = initial_h;
    update_geom(&mut wm.ctx());

    // Store initial window size for the calloop source callback.
    state.winit_window_size = output_size;

    let output = state.create_output("winit", initial_w, initial_h);
    let mut damage_tracker =
        smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);

    // Initialize Wayland systray runtime - only applicable for Wayland backend
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();
    }

    run_autostart();
    spawn_wayland_smoke_window();
    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    crate::runtime::register_ipc_source(&loop_handle, &ipc_server);

    // ── Winit event source ──────────────────────────────────────────────
    // Insert the winit event loop as a calloop source so host window
    // events (input, resize, close) wake the event loop immediately
    // instead of requiring periodic polling.
    let kb = keyboard_handle.clone();
    let ptr = pointer_handle.clone();
    loop_handle
        .insert_source(winit_loop, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                state.winit_window_size = size;
                state.pending_winit_resize = Some((size.w, size.h));
            }
            WinitEvent::Input(event) => {
                dispatch_winit_input(state, &kb, &ptr, event);
            }
            WinitEvent::CloseRequested => {
                state.winit_close_requested = true;
            }
            WinitEvent::Redraw | WinitEvent::Focus(_) => {}
        })
        .expect("failed to insert winit source");

    let start_time = std::time::Instant::now();

    crate::runtime::spawn_status_bar(&wm);

    // ── Animation timer (on-demand) ─────────────────────────────────────
    let anim_guard = crate::runtime::AnimationTimerGuard::new();
    let loop_handle_for_timer = event_loop.handle();

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(None, &mut state, move |state| {
            // ── 1. Process buffered winit resize/close ──────────────────
            if let Some((w, h)) = state.pending_winit_resize.take() {
                crate::wayland::input::handle_resize(&mut wm, state, &output, w, h);
            }
            if state.winit_close_requested {
                loop_signal.stop();
                return;
            }

            // ── 2. Shared tick: layout, IPC, monitor config ─────────────
            super::common::event_loop_tick(&mut wm, state, &mut ipc_server);

            // Winit has no libinput devices to reconfigure, but clear the
            // flag so it doesn't stay dirty forever (scroll_factor is
            // already applied at the compositor level in handle_pointer_axis).
            wm.g.dirty.input_config = false;

            super::common::sync_space_if_dirty(&mut wm, state);

            // ── 3. Arm animation timer if needed ────────────────────────
            anim_guard.ensure_armed(
                state.has_active_window_animations(),
                &loop_handle_for_timer,
                |_state| {
                    // Timer wakes the loop; animation ticking + render
                    // happen in the main body on the next iteration.
                    _state.has_active_window_animations()
                },
            );

            // Apply any compositor-side cursor warp requested during this tick
            // (e.g. from a warp-to-focus keybinding or IPC command).
            apply_pending_warp(state, &pointer_handle);

            render_frame(
                &mut wm,
                state,
                &mut backend,
                &output,
                &mut damage_tracker,
                start_time,
            );

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("wayland event loop run");
    exit(0);
}

/// Dispatch a winit input event using the WM back-reference in WaylandState.
fn dispatch_winit_input(
    state: &mut WaylandState,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    event: InputEvent<smithay::backend::winit::WinitInput>,
) {
    state.with_wm_mut_unified(|wm, state| match event {
        InputEvent::Keyboard { event } => {
            handle_keyboard(wm, state, keyboard_handle, event);
        }
        InputEvent::PointerMotionAbsolute { event: motion } => {
            let size = state.winit_window_size;
            let motion_event = motion_event_from_winit(motion, size);
            handle_pointer_motion(wm, state, pointer_handle, keyboard_handle, motion_event);
        }
        InputEvent::PointerButton { event: btn } => {
            let loc = state.pointer_location;
            handle_pointer_button(wm, state, pointer_handle, keyboard_handle, btn, loc);
        }
        InputEvent::PointerAxis { event: axis } => {
            let loc = state.pointer_location;
            handle_pointer_axis(wm, state, pointer_handle, keyboard_handle, axis, loc);
        }
        _ => {}
    });
}
