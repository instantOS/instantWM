//! Wayland compositor startup and event loop (nested / winit backend).
//!
//! The standalone DRM/KMS backend lives in `super::drm`.  Everything shared
//! between the two backends (globals init, session env, XWayland, socket
//! setup, bar elements, frame callbacks) lives in `super::common_wayland`.

use std::process::exit;
use std::time::Duration;

use smithay::backend::input::InputEvent;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::wayland_server::Display;
use smithay::utils::Point;

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::monitor::update_geom;
use crate::wm::Wm;

mod bar;
pub mod cursor;
mod init;
pub mod input;
pub mod render;

use self::init::sanitize_wayland_size;
use self::input::{
    apply_pending_warp, handle_keyboard, handle_pointer_axis, handle_pointer_button,
    handle_pointer_motion, handle_resize,
};
use self::render::render_frame;
use super::autostart::run_autostart;
use crate::startup::common_wayland::{
    init_wayland_globals, setup_wayland_socket, spawn_wayland_smoke_window, spawn_xwayland,
};

pub fn run() -> ! {
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
    let (initial_w, initial_h) = sanitize_wayland_size(output_size.w, output_size.h);
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
    wm.wayland_systray_runtime = crate::wayland_systray::WaylandSystrayRuntime::start();

    run_autostart();
    spawn_wayland_smoke_window();
    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    let start_time = std::time::Instant::now();
    let mut pointer_location = Point::from((0.0, 0.0));

    if let Some(ref cmd) = wm.g.cfg.status_command {
        crate::bar::status::spawn_status_command(cmd);
    } else {
        crate::bar::status::spawn_default_status();
    }

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            state.attach_globals(&mut wm.g);

            winit_loop.dispatch_new_events(|event| match event {
                WinitEvent::Resized { size, .. } => {
                    handle_resize(&mut wm, &output, size.w, size.h);
                }
                WinitEvent::Input(event) => match event {
                    InputEvent::Keyboard { event } => {
                        handle_keyboard(&mut wm, state, &keyboard_handle, event);
                    }
                    InputEvent::PointerMotionAbsolute { event } => {
                        handle_pointer_motion(
                            &mut wm,
                            state,
                            &pointer_handle,
                            &keyboard_handle,
                            &backend,
                            event,
                            &mut pointer_location,
                        );
                    }
                    InputEvent::PointerButton { event } => {
                        handle_pointer_button(
                            &mut wm,
                            state,
                            &pointer_handle,
                            &keyboard_handle,
                            event,
                            pointer_location,
                        );
                    }
                    InputEvent::PointerAxis { event } => {
                        handle_pointer_axis(
                            &mut wm,
                            state,
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

            {
                let mut ctx = wm.ctx();
                if !ctx.g.clients.is_empty() && !state.has_active_window_animations() {
                    let selected_monitor_id = ctx.g.selected_monitor_id();
                    crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
                }
            }
            if let Some(server) = ipc_server.as_mut() {
                server.process_pending(&mut wm);
            }

            if wm.g.monitor_config_dirty {
                let mut ctx = wm.ctx();
                crate::monitor::apply_monitor_config(&mut ctx);
            }

            // Winit has no libinput devices to reconfigure, but clear the
            // flag so it doesn't stay dirty forever (scroll_factor is
            // already applied at the compositor level in handle_pointer_axis).
            wm.g.input_config_dirty = false;

            state.sync_space_from_globals();

            // Apply any compositor-side cursor warp requested during this tick
            // (e.g. from a warp-to-focus keybinding or IPC command).
            apply_pending_warp(state, &pointer_handle, &mut pointer_location);

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
