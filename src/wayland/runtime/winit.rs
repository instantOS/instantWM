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
    let mut display_handle = display.handle();

    let (backend_init, mut winit_loop) =
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
    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    let start_time = std::time::Instant::now();

    crate::runtime::spawn_status_bar(&state.wm);

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut state, move |mut state| {
            winit_loop.dispatch_new_events(|event| match event {
                WinitEvent::Resized { size, .. } => {
                    crate::wayland::input::handle_resize(&mut state.wm, &output, size.w, size.h);
                }
                WinitEvent::Input(event) => match event {
                    InputEvent::Keyboard { event } => {
                        handle_keyboard(&mut state, &keyboard_handle, event);
                    }
                    InputEvent::PointerMotionAbsolute { event } => {
                        let size = backend.window_size();
                        let motion_event = motion_event_from_winit(event, size);
                        handle_pointer_motion(
                            &mut state,
                            &pointer_handle,
                            &keyboard_handle,
                            motion_event,
                        );
                    }
                    InputEvent::PointerButton { event } => {
                        let pointer_location = state.pointer_location;
                        handle_pointer_button(
                            &mut state,
                            &pointer_handle,
                            &keyboard_handle,
                            event,
                            pointer_location,
                        );
                    }
                    InputEvent::PointerAxis { event } => {
                        let pointer_location = state.pointer_location;
                        handle_pointer_axis(
                            &mut state,
                            &pointer_handle,
                            &keyboard_handle,
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
            });

            super::common::arrange_layout_if_dirty(&mut state);
            super::common::process_ipc_commands(&mut ipc_server, &mut state);
            crate::runtime::apply_monitor_config_if_dirty(&mut state.wm);

            // Winit has no libinput devices to reconfigure, but clear the
            // flag so it doesn't stay dirty forever (scroll_factor is
            // already applied at the compositor level in handle_pointer_axis).
            state.wm.g.dirty.input_config = false;

            super::common::sync_space_if_dirty(&mut state);

            // Apply any compositor-side cursor warp requested during this tick
            // (e.g. from a warp-to-focus keybinding or IPC command).
            apply_pending_warp(&mut state, &pointer_handle);

            render_frame(
                &mut state,
                &mut backend,
                &output,
                &mut damage_tracker,
                start_time,
            );

            // Phase 1: drain the command queue
            let ops = if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
                data.backend.drain_ops()
            } else {
                Vec::new()
            };

            // Phase 2: execute queued commands
            for op in ops {
                state.execute_command(op);
            }

            // Phase 3: sync caches
            if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
                data.backend.sync_cache(state);
            }

            if display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("wayland event loop run");
    exit(0);
}
