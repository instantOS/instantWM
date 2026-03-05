//! Wayland compositor startup and event loop.

use std::process::{exit, Stdio};
use std::sync::Arc;
use std::time::Duration;

use smithay::backend::input::InputEvent;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::wayland_server::Display;
use smithay::utils::Point;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::monitor::update_geom;
use crate::wm::Wm;

mod bar;
mod init;
mod input;
mod render;

use self::init::{
    apply_wayland_session_env, init_wayland_globals, sanitize_wayland_size,
    spawn_wayland_smoke_window,
};
use self::input::{
    handle_keyboard, handle_pointer_axis, handle_pointer_button, handle_pointer_motion,
    handle_resize,
};
use self::render::{render_frame, wayland_border_elements_shared as border_elements_shared_impl};
use super::autostart::run_autostart;

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

    let (backend_init, mut winit_loop) =
        winit::init::<GlesRenderer>().expect("failed to init winit backend");
    let mut backend = Box::new(backend_init);
    state.attach_renderer(backend.renderer());
    state.init_dmabuf_global(backend.renderer().dmabuf_formats().into_iter().collect());
    let output_size = backend.window_size();
    let (initial_w, initial_h) = sanitize_wayland_size(output_size.w, output_size.h);
    wm.g.cfg.screen_width = initial_w;
    wm.g.cfg.screen_height = initial_h;
    update_geom(&mut wm.ctx());

    let output = state.create_output("winit", initial_w, initial_h);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();
    apply_wayland_session_env(&socket_name);

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    match XWayland::spawn(
        &state.display_handle,
        None,
        std::iter::empty::<(String, String)>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    ) {
        Ok((xwayland, client)) => {
            std::env::set_var("DISPLAY", format!(":{}", xwayland.display_number()));
            let handle_for_wm = loop_handle.clone();
            if let Err(err) = loop_handle.insert_source(xwayland, move |event, _, data| match event
            {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    data.xdisplay = Some(display_number);
                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                    match X11Wm::start_wm(handle_for_wm.clone(), x11_socket, client.clone()) {
                        Ok(wm) => data.xwm = Some(wm),
                        Err(e) => log::error!("failed to start X11 WM for XWayland: {e}"),
                    }
                }
                XWaylandEvent::Error => {
                    log::error!("XWayland failed to start");
                }
            }) {
                log::error!("failed to insert XWayland source: {err}");
            }
        }
        Err(err) => {
            log::warn!("failed to spawn XWayland: {err}");
        }
    }

    run_autostart();
    spawn_wayland_smoke_window();
    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    let start_time = std::time::Instant::now();
    let mut pointer_location = Point::from((0.0, 0.0));

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
            state.sync_space_from_globals();

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

pub(crate) fn wayland_border_elements_shared(
    g: &crate::globals::Globals,
    state: &WaylandState,
) -> Vec<SolidColorRenderElement> {
    border_elements_shared_impl(g, state)
}
