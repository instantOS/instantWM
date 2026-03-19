//! Wayland compositor runtime for the winit (nested) backend.
//!
//! The winit backend runs as a nested compositor inside an existing
//! Wayland or X11 session.

use std::process::exit;
use std::time::Duration;

use smithay::backend::input::InputEvent;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::wayland_server::Display;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
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
    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_wayland_globals(&mut wm);

    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();

    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut display_handle = display.handle();
    let mut state = WaylandState::new(display, &loop_handle);
    state.attach_globals(&mut wm.g);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut state);
    }

    // Apply the initial keyboard layout if configured.
    {
        let mut ctx = wm.ctx();
        crate::keyboard_layout::init_keyboard_layout(&mut ctx);
    }

    let (backend_init, mut winit_loop) =
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

    let output = state.create_output("winit", initial_w, initial_h);
    let mut damage_tracker =
        smithay::backend::renderer::damage::OutputDamageTracker::from_output(&output);

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    setup_wayland_socket(&loop_handle, &state);
    spawn_xwayland(&state, &loop_handle);
    wm.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();

    run_autostart();
    spawn_wayland_smoke_window();
    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    let start_time = std::time::Instant::now();

    if let Some(ref cmd) = wm.g.cfg.status_command {
        crate::bar::status::spawn_status_command(cmd);
    } else {
        crate::bar::status::spawn_default_status();
    }

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            winit_loop.dispatch_new_events(|event| match event {
                WinitEvent::Resized { size, .. } => {
                    crate::wayland::input::handle_resize(&mut wm, &output, size.w, size.h);
                }
                WinitEvent::Input(event) => match event {
                    InputEvent::Keyboard { event } => {
                        handle_keyboard(&mut wm, state, &keyboard_handle, event);
                    }
                    InputEvent::PointerMotionAbsolute { event } => {
                        let size = backend.window_size();
                        let motion_event = motion_event_from_winit(event, size);
                        handle_pointer_motion(
                            &mut wm,
                            state,
                            &pointer_handle,
                            &keyboard_handle,
                            motion_event,
                        );
                    }
                    InputEvent::PointerButton { event } => {
                        handle_pointer_button(
                            &mut wm,
                            state,
                            &pointer_handle,
                            &keyboard_handle,
                            event,
                            state.pointer_location,
                        );
                    }
                    InputEvent::PointerAxis { event } => {
                        handle_pointer_axis(
                            &mut wm,
                            state,
                            &pointer_handle,
                            &keyboard_handle,
                            event,
                            state.pointer_location,
                        );
                    }
                    _ => {}
                },
                WinitEvent::CloseRequested => {
                    loop_signal.stop();
                }
                _ => {}
            });

            super::common::arrange_layout_if_dirty(&mut wm, state);
            super::common::process_ipc_commands(&mut ipc_server, &mut wm);
            super::common::apply_monitor_config_if_dirty(&mut wm);

            // Winit has no libinput devices to reconfigure, but clear the
            // flag so it doesn't stay dirty forever (scroll_factor is
            // already applied at the compositor level in handle_pointer_axis).
            wm.g.dirty.input_config = false;

            super::common::sync_space_if_dirty(&mut wm, state);

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

            if display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("wayland event loop run");
    exit(0);
}
