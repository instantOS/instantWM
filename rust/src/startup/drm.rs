//! DRM/KMS bare-metal backend for running directly on hardware.
//!
//! Uses libseat for session management, udev for GPU discovery, libinput
//! for input, and DRM/GBM/EGL for rendering.  This backend is vblank-driven:
//! each page-flip completion triggers the next frame.

use std::collections::HashMap;
use std::os::fd::FromRawFd;
use std::os::unix::io::AsRawFd;
use std::process::{exit, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, DrmEventMetadata, DrmNode, DrmSurface,
    GbmBufferedSurface, NodeType,
};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::{
    AbsolutePositionEvent, InputEvent, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Bind, ImportDma, Renderer};
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{self, UdevBackend, UdevEvent};
use smithay::desktop::space::render_output;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::desktop::PopupManager;
use smithay::input::keyboard::{FilterResult, KeyboardHandle};
use smithay::input::pointer::PointerHandle;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, LoopHandle, LoopSignal};
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{DeviceFd, Physical, Point, Rectangle, Transform, SERIAL_COUNTER};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::{
    KeyboardFocusTarget, PointerFocusTarget, WaylandClientState, WaylandState,
};
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::bar::color::rgba_from_hex;
use crate::bar::{bar_position_at_x, bar_position_to_gesture};
use crate::client::resize;
use crate::config::init_config;
use crate::monitor;
use crate::mouse::{set_cursor_default, set_cursor_move, set_cursor_resize};
use crate::types::*;
use crate::wm::Wm;

use super::autostart::run_autostart;

use drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags};
use rustix::fs::OFlags;

// Re-use the same render element enum from the winit backend.
render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-output state
// ═══════════════════════════════════════════════════════════════════════════

struct OutputSurface {
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    output: Output,
    damage_tracker: OutputDamageTracker,
    /// The instantWM monitor index mapped to this output.
    mon_idx: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════

pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");

    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_drm_globals(&mut wm);

    let mut event_loop: EventLoop<DrmData> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    // ── Session ──────────────────────────────────────────────────────
    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    // ── Wayland display ──────────────────────────────────────────────
    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut display_handle = display.handle();
    let mut wayland_state = WaylandState::new(display, &loop_handle);
    wayland_state.attach_globals(&mut wm.g);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut wayland_state);
    }

    // ── GPU discovery ────────────────────────────────────────────────
    let primary_gpu_path = udev::primary_gpu(&seat_name)
        .ok()
        .flatten()
        .or_else(|| {
            udev::all_gpus(&seat_name)
                .ok()
                .and_then(|gpus| gpus.into_iter().next())
        })
        .expect("no GPU found");
    let drm_node = DrmNode::from_path(&primary_gpu_path)
        .expect("DrmNode from path");
    log::info!("Using GPU: {:?} (node {:?})", primary_gpu_path, drm_node);

    // ── Open DRM device via session ──────────────────────────────────
    let fd = session
        .open(
            &primary_gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .expect("session open DRM device");
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (mut drm_device, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), true).expect("DrmDevice::new");

    // ── GBM + EGL + GLES renderer ───────────────────────────────────
    let gbm_device = GbmDevice::new(drm_fd.clone()).expect("GbmDevice::new");
    let egl_display =
        unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let mut renderer =
        unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    wayland_state.attach_renderer(&mut renderer);
    wayland_state
        .init_dmabuf_global(renderer.dmabuf_formats().into_iter().collect());

    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    // ── Scan connectors and create outputs ───────────────────────────
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut surfaces: HashMap<crtc::Handle, OutputSurface> = HashMap::new();
    let mut output_x_offset: i32 = 0;
    let mut mon_idx_counter: usize = 0;

    let res = drm_device
        .resource_handles()
        .expect("drm resource_handles");

    // Build a list of active (connected) connectors with assigned CRTCs.
    let mut crtc_assignments: Vec<(connector::Handle, crtc::Handle, drm::control::Mode)> =
        Vec::new();
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();
    let available_crtcs: Vec<crtc::Handle> = res.crtcs().to_vec();

    for &conn_handle in res.connectors() {
        let Ok(conn_info) = drm_device.get_connector(conn_handle, false) else {
            continue;
        };
        if conn_info.state() != connector::State::Connected {
            continue;
        }
        let modes = conn_info.modes();
        if modes.is_empty() {
            continue;
        }
        let mode = modes
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .copied()
            .unwrap_or(modes[0]);

        // Find an available CRTC for this connector.
        let encoder_crtcs: Vec<crtc::Handle> = conn_info
            .encoders()
            .iter()
            .filter_map(|&enc_h| drm_device.get_encoder(enc_h).ok())
            .flat_map(|enc| {
                available_crtcs
                    .iter()
                    .enumerate()
                    .filter(move |(i, _)| (1u32 << i) & enc.possible_crtcs() != 0)
                    .map(|(_, &c)| c)
            })
            .collect();
        if let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) {
            used_crtcs.push(picked_crtc);
            crtc_assignments.push((conn_handle, picked_crtc, mode));
        }
    }

    for (conn_handle, crtc_handle, mode) in &crtc_assignments {
        let drm_surface = drm_device
            .create_surface(*crtc_handle, *mode, &[*conn_handle])
            .expect("create_surface");
        let gbm_surface = GbmBufferedSurface::new(
            drm_surface,
            gbm_allocator.clone(),
            color_formats,
            renderer_formats.iter().cloned(),
        )
        .expect("GbmBufferedSurface::new");

        let (mode_w, mode_h) = mode.size();
        let (mode_w, mode_h) = (mode_w as i32, mode_h as i32);
        let conn_info = drm_device.get_connector(*conn_handle, false).unwrap();
        let output_name = format!(
            "{}-{}",
            connector_type_name(conn_info.interface()),
            conn_info.interface_id()
        );
        log::info!(
            "Output {output_name}: {mode_w}x{mode_h}@{}Hz on CRTC {:?}",
            mode.vrefresh(),
            crtc_handle
        );

        let output = Output::new(
            output_name,
            PhysicalProperties {
                size: physical_size_from_connector(&conn_info),
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
            },
        );
        let out_mode = OutputMode {
            size: (mode_w, mode_h).into(),
            refresh: (mode.vrefresh() as i32) * 1000,
        };
        output.change_current_state(
            Some(out_mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((output_x_offset, 0).into()),
        );
        output.set_preferred(out_mode);
        let _global = output.create_global::<WaylandState>(&wayland_state.display_handle);
        wayland_state
            .space
            .map_output(&output, (output_x_offset, 0));

        let damage_tracker = OutputDamageTracker::from_output(&output);
        let mon_idx = mon_idx_counter;
        mon_idx_counter += 1;

        surfaces.insert(
            *crtc_handle,
            OutputSurface {
                surface: gbm_surface,
                output,
                damage_tracker,
                mon_idx,
            },
        );
        output_x_offset += mode_w;
    }

    // Sync instantWM monitor state.
    sync_monitors_from_outputs(&mut wm, &surfaces);

    // ── Wayland socket ───────────────────────────────────────────────
    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();
    apply_drm_session_env(&socket_name);

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .wayland_state
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    // ── XWayland ─────────────────────────────────────────────────────
    match XWayland::spawn(
        &wayland_state.display_handle,
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
            if let Err(err) =
                loop_handle.insert_source(xwayland, move |event, _, data| match event {
                    XWaylandEvent::Ready {
                        x11_socket,
                        display_number,
                    } => {
                        data.wayland_state.xdisplay = Some(display_number);
                        std::env::set_var("DISPLAY", format!(":{display_number}"));
                        match X11Wm::start_wm(
                            handle_for_wm.clone(),
                            x11_socket,
                            client.clone(),
                        ) {
                            Ok(wm) => data.wayland_state.xwm = Some(wm),
                            Err(e) => log::error!("failed to start X11 WM: {e}"),
                        }
                    }
                    XWaylandEvent::Error => log::error!("XWayland failed"),
                })
            {
                log::error!("failed to insert XWayland source: {err}");
            }
        }
        Err(err) => log::warn!("failed to spawn XWayland: {err}"),
    }

    // ── libinput ─────────────────────────────────────────────────────
    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .expect("libinput assign seat");
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    loop_handle
        .insert_source(libinput_backend, |event, _, data| {
            data.process_input_event(event);
        })
        .expect("libinput source");

    // ── Session events ───────────────────────────────────────────────
    loop_handle
        .insert_source(notifier, move |event, _, data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused");
                data.drm_device.pause();
                data.libinput_context.suspend();
                data.session_active = false;
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated");
                if let Err(e) = data.drm_device.activate(false) {
                    log::error!("DRM activate failed: {e}");
                }
                if let Err(e) = data.libinput_context.resume() {
                    log::error!("libinput resume failed: {e:?}");
                }
                data.session_active = true;
                // Kick a full redraw on all outputs.
                for surface in data.surfaces.values_mut() {
                    surface.surface.reset_buffers();
                }
                data.needs_redraw = true;
            }
        })
        .expect("session source");

    // ── DRM vblank events ────────────────────────────────────────────
    loop_handle
        .insert_source(drm_notifier, |event, metadata, data| match event {
            DrmEvent::VBlank(crtc) => {
                data.frame_finish(crtc);
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .expect("drm notifier source");

    run_autostart();

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();

    // Assemble the DRM data struct that lives in the calloop.
    let mut drm_data = DrmData {
        wm,
        wayland_state,
        renderer,
        drm_device,
        surfaces,
        session,
        libinput_context,
        session_active: true,
        needs_redraw: true,
        pointer_location: Point::from((0.0, 0.0)),
        start_time,
        total_width: output_x_offset,
        total_height: crtc_assignments
            .iter()
            .map(|(_, _, m)| m.size().1 as i32)
            .max()
            .unwrap_or(800),
    };

    // Kick the first frame render.
    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut drm_data, move |data| {
            data.wayland_state.attach_globals(&mut data.wm.g);

            // Layout pass.
            {
                let mut ctx = data.wm.ctx();
                if !ctx.g.clients.is_empty() {
                    let selmon = ctx.g.selmon_id();
                    crate::layouts::arrange(&mut ctx, Some(selmon));
                }
            }
            if let Some(server) = ipc_server.as_mut() {
                server.process_pending(&mut data.wm);
            }
            data.wayland_state.sync_space_from_globals();

            // Render all outputs if damage is pending.
            if data.needs_redraw && data.session_active {
                data.render_all_outputs();
            }

            if data.wayland_state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");

    exit(0);
}

// ═══════════════════════════════════════════════════════════════════════════
// DRM data — the calloop state
// ═══════════════════════════════════════════════════════════════════════════

struct DrmData {
    wm: Wm,
    wayland_state: WaylandState,
    renderer: GlesRenderer,
    drm_device: DrmDevice,
    surfaces: HashMap<crtc::Handle, OutputSurface>,
    session: LibSeatSession,
    libinput_context: Libinput,
    session_active: bool,
    needs_redraw: bool,
    pointer_location: Point<f64, smithay::utils::Logical>,
    start_time: std::time::Instant,
    total_width: i32,
    total_height: i32,
}

impl DrmData {
    // ── Rendering ────────────────────────────────────────────────────

    fn render_all_outputs(&mut self) {
        if !self.session_active {
            return;
        }
        let crtcs: Vec<crtc::Handle> = self.surfaces.keys().copied().collect();
        for crtc in crtcs {
            self.render_output(crtc);
        }
        self.needs_redraw = false;
    }

    fn render_output(&mut self, crtc: crtc::Handle) {
        let Some(surface) = self.surfaces.get_mut(&crtc) else {
            return;
        };

        // Acquire the next buffer from the swapchain.
        let (dmabuf, age) = match surface.surface.next_buffer() {
            Ok(buf) => buf,
            Err(e) => {
                log::warn!("next_buffer failed for {:?}: {e}", crtc);
                return;
            }
        };

        // Bind the dmabuf to the renderer.
        let mut dmabuf_clone = dmabuf.clone();
        let Ok(mut target) = self.renderer.bind(&mut dmabuf_clone) else {
            log::warn!("renderer bind failed for {:?}", crtc);
            return;
        };

        // Build custom elements (bar + borders).
        let mut custom_elements: Vec<DrmExtras> = Vec::new();

        // Bar.
        if self.wm.g.cfg.showbar {
            let mut ctx = self.wm.ctx();
            let bar_buffers =
                crate::bar::wayland::render_bar_buffers(&mut ctx, Scale::from(1.0));
            for (buffer, x, y) in bar_buffers {
                match MemoryRenderBufferRenderElement::from_buffer(
                    &mut self.renderer,
                    (x as f64, y as f64),
                    &buffer,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    Ok(elem) => custom_elements.push(DrmExtras::Memory(elem)),
                    Err(e) => log::warn!("bar buffer upload failed: {:?}", e),
                }
            }
        }

        // Window borders.
        for elem in wayland_border_elements(&self.wm.g, &self.wayland_state) {
            custom_elements.push(DrmExtras::Solid(elem));
        }

        // Render.
        let surface = self.surfaces.get_mut(&crtc).unwrap();
        let render_result = render_output(
            &surface.output,
            &mut self.renderer,
            &mut target,
            1.0,
            age as usize,
            [&self.wayland_state.space],
            &custom_elements,
            &mut surface.damage_tracker,
            [0.05, 0.05, 0.07, 1.0],
        );
        drop(target);

        match render_result {
            Ok(result) => {
                let damage: Option<Vec<Rectangle<i32, Physical>>> =
                    result.damage.cloned();
                // Queue the buffer for page-flip.
                if let Err(e) = surface.surface.queue_buffer(
                    None,
                    damage,
                    (),
                ) {
                    log::warn!("queue_buffer failed: {e}");
                }
            }
            Err(e) => {
                log::warn!("render_output failed: {:?}", e);
            }
        }

        // Send frame callbacks.
        let time = self.start_time.elapsed();
        let output = &self.surfaces.get(&crtc).unwrap().output;
        for window in self.wayland_state.space.elements() {
            if let Some(wl_surface) = window.wl_surface() {
                send_frames_surface_tree(
                    &wl_surface,
                    output,
                    time,
                    Some(Duration::from_millis(16)),
                    surface_primary_scanout_output,
                );
                if let Some(toplevel) = window.toplevel() {
                    for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                        send_frames_surface_tree(
                            popup.wl_surface(),
                            output,
                            time,
                            Some(Duration::from_millis(16)),
                            surface_primary_scanout_output,
                        );
                    }
                }
            }
        }
    }

    fn frame_finish(&mut self, crtc: crtc::Handle) {
        if let Some(surface) = self.surfaces.get_mut(&crtc) {
            match surface.surface.frame_submitted() {
                Ok(_) => {}
                Err(e) => log::warn!("frame_submitted error: {e}"),
            }
        }
        // After a completed page-flip, schedule the next frame.
        self.needs_redraw = true;
    }

    // ── Input ────────────────────────────────────────────────────────

    fn process_input_event(&mut self, event: InputEvent<LibinputInputBackend>) {
        match event {
            InputEvent::Keyboard { event } => self.handle_keyboard(event),
            InputEvent::PointerMotion { event } => self.handle_pointer_motion_relative(event),
            InputEvent::PointerMotionAbsolute { event } => {
                self.handle_pointer_motion_absolute(event)
            }
            InputEvent::PointerButton { event } => self.handle_pointer_button(event),
            InputEvent::PointerAxis { event } => self.handle_pointer_axis(event),
            _ => {}
        }
    }

    fn handle_keyboard(&mut self, event: impl KeyboardKeyEvent<LibinputInputBackend>) {
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard_handle = self.wayland_state.keyboard.clone();

        if matches!(
            keyboard_handle.current_focus(),
            None | Some(KeyboardFocusTarget::Window(_))
        ) {
            if let Some(layer_surface) = self.wayland_state.keyboard_focus_layer_surface() {
                keyboard_handle.set_focus(
                    &mut self.wayland_state,
                    Some(KeyboardFocusTarget::WlSurface(layer_surface)),
                    serial,
                );
            }
        }

        let wm_shortcuts_allowed = matches!(
            keyboard_handle.current_focus(),
            None | Some(KeyboardFocusTarget::Window(_))
        );

        let wm = &mut self.wm;
        keyboard_handle.input(
            &mut self.wayland_state,
            event.key_code(),
            event.state(),
            serial,
            event.time() as u32,
            |_data, modifiers, keysym| {
                if wm_shortcuts_allowed
                    && event.state() == smithay::backend::input::KeyState::Pressed
                {
                    let mod_mask = modifiers_to_x11_mask(modifiers);
                    let mut ctx = wm.ctx();
                    if crate::keyboard::handle_keysym(
                        &mut ctx,
                        u32::from(keysym.modified_sym()),
                        mod_mask,
                    ) {
                        return FilterResult::Intercept(());
                    }
                }
                FilterResult::Forward
            },
        );
    }

    fn handle_pointer_motion_relative(
        &mut self,
        event: impl PointerMotionEvent<LibinputInputBackend>,
    ) {
        let dx = event.delta().x;
        let dy = event.delta().y;
        let mut new_x = self.pointer_location.x + dx;
        let mut new_y = self.pointer_location.y + dy;
        // Clamp to total output bounds.
        new_x = new_x.clamp(0.0, self.total_width as f64 - 1.0);
        new_y = new_y.clamp(0.0, self.total_height as f64 - 1.0);
        self.pointer_location = Point::from((new_x, new_y));

        self.dispatch_pointer_motion(event.time() as u32);
    }

    fn handle_pointer_motion_absolute(
        &mut self,
        event: impl AbsolutePositionEvent<LibinputInputBackend>,
    ) {
        let x = event.x_transformed(self.total_width);
        let y = event.y_transformed(self.total_height);
        self.pointer_location = Point::from((x, y));

        self.dispatch_pointer_motion(event.time() as u32);
    }

    fn dispatch_pointer_motion(&mut self, time: u32) {
        // Hover focus.
        let hovered_win = find_hovered_window(&self.wm, &self.wayland_state, self.pointer_location);
        {
            let mut ctx = self.wm.ctx();
            crate::focus::hover_focus_target(&mut ctx, hovered_win, false);
        }

        // Bar hit-testing.
        let root_x = self.pointer_location.x.round() as i32;
        let root_y = self.pointer_location.y.round() as i32;

        if wayland_hover_resize_drag_motion(&mut self.wm, root_x, root_y) {
            self.needs_redraw = true;
        }

        if wayland_active_drag_window(&self.wm).is_none() {
            let mut ctx = self.wm.ctx();
            let _ = crate::mouse::handle_floating_resize_hover(&mut ctx, root_x, root_y, false);
        }

        let _ = update_wayland_bar_hit_state(&mut self.wm, root_x, root_y, false);

        // Forward to Smithay.
        let focus = self
            .wayland_state
            .layer_surface_under_pointer(self.pointer_location)
            .or_else(|| self.wayland_state.surface_under_pointer(self.pointer_location))
            .map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));

        let serial = SERIAL_COUNTER.next_serial();
        let pointer_handle = self.wayland_state.pointer.clone();
        let motion = smithay::input::pointer::MotionEvent {
            location: self.pointer_location,
            serial,
            time,
        };
        pointer_handle.motion(&mut self.wayland_state, focus, &motion);
        pointer_handle.frame(&mut self.wayland_state);
        self.needs_redraw = true;
    }

    fn handle_pointer_button(
        &mut self,
        event: impl PointerButtonEvent<LibinputInputBackend>,
    ) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer_handle = self.wayland_state.pointer.clone();
        let keyboard_handle = self.wayland_state.keyboard.clone();

        let root_x = self.pointer_location.x.round() as i32;
        let root_y = self.pointer_location.y.round() as i32;

        let button_code = event.button_code();
        let state = event.state();

        if state == smithay::backend::input::ButtonState::Pressed {
            // Bar click handling.
            if let Some(pos) = update_wayland_bar_hit_state(&mut self.wm, root_x, root_y, true) {
                dispatch_wayland_bar_click(
                    &mut self.wm,
                    pos,
                    button_code,
                    root_x,
                    root_y,
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                );
                self.needs_redraw = true;
                return;
            }

            // Hover-resize drag begin.
            if wayland_hover_resize_drag_begin(&mut self.wm, root_x, root_y, button_to_mouse_button(button_code)) {
                self.needs_redraw = true;
                return;
            }

            // Focus the clicked window.
            let hovered =
                find_hovered_window(&self.wm, &self.wayland_state, self.pointer_location);
            if let Some(win) = hovered {
                let mut ctx = self.wm.ctx();
                crate::focus::focus_soft(&mut ctx, Some(win));
            }

            // Set keyboard focus to a layer surface or the clicked window's surface.
            let focus = self
                .wayland_state
                .layer_surface_under_pointer(self.pointer_location)
                .map(|(s, _)| KeyboardFocusTarget::WlSurface(s))
                .or_else(|| {
                    self.wayland_state
                        .surface_under_pointer(self.pointer_location)
                        .map(|(s, _)| KeyboardFocusTarget::WlSurface(s))
                });
            keyboard_handle.set_focus(&mut self.wayland_state, focus, serial);
        } else {
            // Release.
            if wayland_hover_resize_drag_finish(&mut self.wm, button_to_mouse_button(button_code)) {
                self.needs_redraw = true;
            }
        }

        // Forward to Smithay pointer.
        let pointer_event = smithay::input::pointer::ButtonEvent {
            button: button_code,
            state: match state {
                smithay::backend::input::ButtonState::Pressed => {
                    smithay::backend::input::ButtonState::Pressed
                }
                smithay::backend::input::ButtonState::Released => {
                    smithay::backend::input::ButtonState::Released
                }
            },
            serial,
            time: event.time() as u32,
        };
        pointer_handle.button(&mut self.wayland_state, &pointer_event);
        pointer_handle.frame(&mut self.wayland_state);
        self.needs_redraw = true;
    }

    fn handle_pointer_axis(
        &mut self,
        event: impl PointerAxisEvent<LibinputInputBackend>,
    ) {
        let root_x = self.pointer_location.x.round() as i32;
        let root_y = self.pointer_location.y.round() as i32;
        let keyboard_handle = self.wayland_state.keyboard.clone();

        // Bar scroll handling.
        if let Some(pos) = update_wayland_bar_hit_state(&mut self.wm, root_x, root_y, false) {
            if let Some(amount) = event.amount(smithay::backend::input::Axis::Vertical) {
                if amount.abs() > 0.1 {
                    dispatch_wayland_bar_scroll(
                        &mut self.wm,
                        pos,
                        amount,
                        root_x,
                        root_y,
                        modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    );
                    self.needs_redraw = true;
                    return;
                }
            }
        }

        // Forward axis to Smithay.
        let pointer_handle = self.wayland_state.pointer.clone();
        let mut frame = smithay::input::pointer::AxisFrame::new(event.time() as u32);
        if let Some(amount) = event.amount(smithay::backend::input::Axis::Horizontal) {
            frame = frame.value(smithay::backend::input::Axis::Horizontal, amount);
        }
        if let Some(amount) = event.amount(smithay::backend::input::Axis::Vertical) {
            frame = frame.value(smithay::backend::input::Axis::Vertical, amount);
        }
        if event.source() == smithay::backend::input::AxisSource::Finger {
            frame = frame.source(smithay::backend::input::AxisSource::Finger);
        }
        pointer_handle.axis(&mut self.wayland_state, frame);
        pointer_handle.frame(&mut self.wayland_state);
        self.needs_redraw = true;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared helper functions (mirrored from startup/wayland.rs)
// ═══════════════════════════════════════════════════════════════════════════

fn find_hovered_window(
    wm: &Wm,
    state: &WaylandState,
    pointer_location: Point<f64, smithay::utils::Logical>,
) -> Option<WindowId> {
    use crate::backend::wayland::compositor::WindowIdMarker;

    let px = pointer_location.x;
    let py = pointer_location.y;
    for w in wm.g.focus_stack_iter() {
        let Some(c) = wm.g.clients.get(&w) else {
            continue;
        };
        if c.is_hidden {
            continue;
        }
        let is_visible = c
            .mon_id
            .and_then(|mid| wm.g.monitor(mid))
            .map(|m| c.is_visible_on_tags(m.selected_tags()))
            .unwrap_or(false);
        if !is_visible {
            continue;
        }
        let bw = c.border_width.max(0) as f64;
        let ox = c.geo.x as f64;
        let oy = c.geo.y as f64;
        let ow = c.geo.w as f64 + 2.0 * bw;
        let oh = c.geo.h as f64 + 2.0 * bw;
        if px >= ox && px < ox + ow && py >= oy && py < oy + oh {
            return Some(w);
        }
    }

    state
        .space
        .element_under(pointer_location)
        .and_then(|(window, _)| window.user_data().get::<WindowIdMarker>().map(|m| m.id))
}

fn wayland_active_drag_window(wm: &Wm) -> Option<WindowId> {
    if wm.g.drag.hover_resize.active {
        return Some(wm.g.drag.hover_resize.win);
    }
    if wm.g.drag.title.active {
        return Some(wm.g.drag.title.win);
    }
    None
}

fn wayland_hover_resize_drag_begin(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
    btn: MouseButton,
) -> bool {
    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }
    let mut ctx = wm.ctx();
    let Some((win, dir)) = crate::mouse::hover::hover_resize_target_at(&ctx, root_x, root_y)
    else {
        return false;
    };
    let Some((geo, is_floating, has_tiling)) = ctx.g.clients.get(&win).map(|c| {
        (
            c.geo,
            c.isfloating,
            ctx.g
                .selmon()
                .map(|m| m.is_tiling_layout())
                .unwrap_or(true),
        )
    }) else {
        return false;
    };
    if !is_floating && has_tiling {
        return false;
    }
    let move_mode =
        btn == MouseButton::Right || crate::mouse::hover::is_at_top_middle_edge(&geo, root_x, root_y);
    ctx.g.drag.hover_resize = crate::globals::HoverResizeDragState {
        active: true,
        win,
        button: btn,
        direction: dir,
        move_mode,
        start_x: root_x,
        start_y: root_y,
        win_start_x: geo.x,
        win_start_y: geo.y,
        win_start_w: geo.w,
        win_start_h: geo.h,
        last_root_x: root_x,
        last_root_y: root_y,
    };
    ctx.g.altcursor = AltCursor::Resize;
    ctx.g.drag.resize_direction = Some(dir);
    if move_mode {
        set_cursor_move(&mut ctx);
    } else {
        set_cursor_resize(&mut ctx, Some(dir));
    }
    crate::focus::focus_soft(&mut ctx, Some(win));
    true
}

fn wayland_hover_resize_drag_motion(wm: &mut Wm, root_x: i32, root_y: i32) -> bool {
    let mut ctx = wm.ctx();
    if !ctx.g.drag.hover_resize.active {
        return false;
    }
    let drag = ctx.g.drag.hover_resize.clone();
    ctx.g.drag.hover_resize.last_root_x = root_x;
    ctx.g.drag.hover_resize.last_root_y = root_y;
    if drag.move_mode {
        let new_x = drag.win_start_x + (root_x - drag.start_x);
        let new_y = drag.win_start_y + (root_y - drag.start_y);
        resize(
            &mut ctx,
            drag.win,
            &Rect {
                x: new_x,
                y: new_y,
                w: drag.win_start_w.max(1),
                h: drag.win_start_h.max(1),
            },
            true,
        );
        if let Some(client) = ctx.g.clients.get_mut(&drag.win) {
            client.float_geo.x = new_x;
            client.float_geo.y = new_y;
        }
        return true;
    }

    let orig_left = drag.win_start_x;
    let orig_top = drag.win_start_y;
    let orig_right = drag.win_start_x + drag.win_start_w;
    let orig_bottom = drag.win_start_y + drag.win_start_h;
    let (affects_left, affects_right, affects_top, affects_bottom) =
        drag.direction.affected_edges();
    let (new_x, new_w) = if affects_left {
        (root_x, (orig_right - root_x).max(1))
    } else if affects_right {
        (orig_left, (root_x - orig_left + 1).max(1))
    } else {
        (orig_left, drag.win_start_w.max(1))
    };
    let (new_y, new_h) = if affects_top {
        (root_y, (orig_bottom - root_y).max(1))
    } else if affects_bottom {
        (orig_top, (root_y - orig_top + 1).max(1))
    } else {
        (orig_top, drag.win_start_h.max(1))
    };
    resize(
        &mut ctx,
        drag.win,
        &Rect {
            x: new_x,
            y: new_y,
            w: new_w,
            h: new_h,
        },
        true,
    );
    true
}

fn wayland_hover_resize_drag_finish(wm: &mut Wm, btn: MouseButton) -> bool {
    let mut ctx = wm.ctx();
    if !ctx.g.drag.hover_resize.active || ctx.g.drag.hover_resize.button != btn {
        return false;
    }
    let drag = ctx.g.drag.hover_resize.clone();
    ctx.g.drag.hover_resize = crate::globals::HoverResizeDragState::default();
    ctx.g.altcursor = AltCursor::None;
    ctx.g.drag.resize_direction = None;
    set_cursor_default(&mut ctx);
    if drag.move_mode {
        crate::mouse::drag::complete_move_drop(
            &mut ctx,
            drag.win,
            drag.win_start_x,
            drag.win_start_y,
            drag.win_start_w,
            drag.win_start_h,
            None,
            Some((drag.last_root_x, drag.last_root_y)),
        );
    } else {
        crate::mouse::monitor::handle_client_monitor_switch(&mut ctx, drag.win);
    }
    true
}

fn update_wayland_bar_hit_state(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
    reset_start_menu: bool,
) -> Option<BarPosition> {
    let rect = Rect {
        x: root_x,
        y: root_y,
        w: 1,
        h: 1,
    };
    let mid = crate::types::find_monitor_by_rect(&wm.g.monitors, &rect)?;
    let mut ctx = wm.ctx();
    if mid != ctx.g.selmon_id() {
        ctx.g.set_selmon(mid);
    }

    let bar_h = ctx.g.cfg.bar_height.max(1);
    let in_bar = ctx
        .g
        .selmon()
        .is_some_and(|m| m.showbar && root_y >= m.by && root_y < m.by + bar_h);
    if !in_bar {
        let had_hover = ctx
            .g
            .selmon()
            .is_some_and(|m| m.gesture != crate::types::Gesture::None);
        if had_hover {
            crate::bar::reset_bar(&mut ctx);
        }
        return None;
    }

    let mon = ctx.g.selmon().cloned()?;
    let local_x = root_x - mon.work_rect.x;
    let pos = bar_position_at_x(&mon, &ctx, local_x);
    if reset_start_menu && pos == BarPosition::StartMenu {
        crate::bar::reset_bar(&mut ctx);
    }

    let gesture = if pos == BarPosition::StatusText {
        ctx.g.selmon().map(|m| m.gesture).unwrap_or_default()
    } else {
        bar_position_to_gesture(pos)
    };
    if let Some(m) = ctx.g.selmon_mut() {
        m.gesture = gesture;
    }

    Some(pos)
}

fn dispatch_wayland_bar_click(
    wm: &mut Wm,
    pos: BarPosition,
    button_code: u32,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    let Some(button_code) = wayland_button_to_wm_button(button_code) else {
        return;
    };
    let Some(button) = MouseButton::from_u8(button_code) else {
        return;
    };
    let mut ctx = wm.ctx();
    dispatch_wayland_bar_button(&mut ctx, pos, button, root_x, root_y, clean_state);
}

fn dispatch_wayland_bar_scroll(
    wm: &mut Wm,
    pos: BarPosition,
    delta: f64,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    let button = if delta > 0.0 {
        MouseButton::ScrollUp
    } else {
        MouseButton::ScrollDown
    };
    let mut ctx = wm.ctx();
    dispatch_wayland_bar_button(&mut ctx, pos, button, root_x, root_y, clean_state);
}

fn dispatch_wayland_bar_button(
    ctx: &mut crate::contexts::WmCtx<'_>,
    pos: BarPosition,
    btn: MouseButton,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    let numlockmask = ctx.g.cfg.numlockmask;
    let buttons = ctx.g.cfg.buttons.clone();
    for b in &buttons {
        if !b.matches(pos) || b.button != btn {
            continue;
        }
        if crate::util::clean_mask(b.mask, numlockmask) != clean_state {
            continue;
        }
        (b.action)(
            ctx,
            ButtonArg {
                pos,
                btn: b.button,
                rx: root_x,
                ry: root_y,
            },
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Border rendering (shared with wayland.rs)
// ═══════════════════════════════════════════════════════════════════════════

fn wayland_border_elements(
    g: &crate::globals::Globals,
    state: &WaylandState,
) -> Vec<SolidColorRenderElement> {
    // Delegate to the shared implementation in the wayland startup module.
    crate::startup::wayland::wayland_border_elements_shared(g, state)
}

// ═══════════════════════════════════════════════════════════════════════════
// Initialisation helpers
// ═══════════════════════════════════════════════════════════════════════════

fn init_drm_globals(wm: &mut Wm) {
    let cfg = init_config();
    wm.g.cfg.screen_width = 1280;
    wm.g.cfg.screen_height = 800;
    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.cfg.showbar = true;
    let font_size = wayland_font_size_from_config(&cfg.fonts);
    let font_height = wayland_font_height_from_size(font_size);
    wm.bar_painter.set_font_size(font_size);
    let min_bar_height = CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL + 2;
    wm.g.cfg.bar_height = (if cfg.barheight > 0 {
        font_height + cfg.barheight
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    wm.g.cfg.horizontal_padding = font_height;
    wm.g.cfg.numlockmask = 0;
    monitor::update_geom_ctx(&mut wm.ctx());
}

fn wayland_font_size_from_config(fonts: &[String]) -> f32 {
    fonts
        .iter()
        .find_map(|font| {
            let idx = font.find("size=")?;
            let tail = &font[idx + 5..];
            let num: String = tail
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            num.parse::<f32>().ok().filter(|s| *s > 0.0)
        })
        .unwrap_or(14.0)
}

fn wayland_font_height_from_size(font_size: f32) -> i32 {
    ((font_size * 1.3).ceil() as i32).max(font_size.ceil() as i32 + 2)
}

/// Sync instantWM monitor list from DRM output surfaces.
fn sync_monitors_from_outputs(wm: &mut Wm, surfaces: &HashMap<crtc::Handle, OutputSurface>) {
    let mut outputs: Vec<(&crtc::Handle, &OutputSurface)> = surfaces.iter().collect();
    outputs.sort_by_key(|(_, s)| s.mon_idx);

    wm.g.monitors.clear();
    let tag_template = wm.g.cfg.tag_template.clone();

    for (i, (_, surface)) in outputs.iter().enumerate() {
        let mode = surface.output.current_mode().unwrap();
        let (w, h) = (mode.size.w, mode.size.h);
        let pos = surface.output.current_location();
        let x = pos.x;
        let y = pos.y;

        let mut mon =
            crate::types::Monitor::new_with_values(wm.g.cfg.mfact, wm.g.cfg.nmaster, wm.g.cfg.showbar, wm.g.cfg.topbar);
        mon.num = i as i32;
        mon.monitor_rect = Rect { x, y, w, h };
        mon.work_rect = Rect { x, y, w, h };
        mon.current_tag = 1;
        mon.prev_tag = 1;
        mon.tagset = [1, 1];
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    wm.g.cfg.screen_width = outputs
        .iter()
        .map(|(_, s)| {
            let pos = s.output.current_location();
            let mode = s.output.current_mode().unwrap();
            pos.x + mode.size.w
        })
        .max()
        .unwrap_or(1280);
    wm.g.cfg.screen_height = outputs
        .iter()
        .map(|(_, s)| {
            let pos = s.output.current_location();
            let mode = s.output.current_mode().unwrap();
            pos.y + mode.size.h
        })
        .max()
        .unwrap_or(800);

    if wm.g.monitors.is_empty() {
        let mut mon =
            crate::types::Monitor::new_with_values(wm.g.cfg.mfact, wm.g.cfg.nmaster, wm.g.cfg.showbar, wm.g.cfg.topbar);
        mon.monitor_rect = Rect { x: 0, y: 0, w: 1280, h: 800 };
        mon.work_rect = Rect { x: 0, y: 0, w: 1280, h: 800 };
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    // Fix up monitor_id fields.
    for (i, mon) in wm.g.monitors.iter_mut().enumerate() {
        mon.monitor_id = i;
    }

    if wm.g.selmon_id() >= wm.g.monitors.len() {
        wm.g.set_selmon(0);
    }
}

fn apply_drm_session_env(socket_name: &str) {
    std::env::set_var("WAYLAND_DISPLAY", socket_name);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::remove_var("DISPLAY");
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::set_var("SDL_VIDEODRIVER", "wayland");
    std::env::set_var("CLUTTER_BACKEND", "wayland");
}

fn connector_type_name(interface: connector::Interface) -> &'static str {
    match interface {
        connector::Interface::DVII => "DVI-I",
        connector::Interface::DVID => "DVI-D",
        connector::Interface::DVIA => "DVI-A",
        connector::Interface::SVideo => "S-Video",
        connector::Interface::DisplayPort => "DP",
        connector::Interface::HDMIA => "HDMI-A",
        connector::Interface::HDMIB => "HDMI-B",
        connector::Interface::EDP => "eDP",
        connector::Interface::VGA => "VGA",
        connector::Interface::LVDS => "LVDS",
        connector::Interface::DSI => "DSI",
        connector::Interface::DPI => "DPI",
        connector::Interface::Composite => "Composite",
        connector::Interface::TV => "TV",
        _ => "Unknown",
    }
}

fn physical_size_from_connector(
    conn: &drm::control::connector::Info,
) -> smithay::utils::Size<i32, smithay::utils::Raw> {
    let (mm_w, mm_h) = conn.size().unwrap_or((0, 0));
    (mm_w as i32, mm_h as i32).into()
}

fn modifiers_to_x11_mask(mods: &smithay::input::keyboard::ModifiersState) -> u32 {
    let mut mask = 0u32;
    if mods.shift {
        mask |= crate::config::SHIFT;
    }
    if mods.ctrl {
        mask |= crate::config::CONTROL;
    }
    if mods.alt {
        mask |= crate::config::MOD1;
    }
    if mods.logo {
        mask |= crate::config::MODKEY;
    }
    mask
}

#[inline]
fn wayland_button_to_wm_button(code: u32) -> Option<u8> {
    match code {
        0x110 => Some(1), // BTN_LEFT
        0x112 => Some(2), // BTN_MIDDLE
        0x111 => Some(3), // BTN_RIGHT
        _ => None,
    }
}

fn button_to_mouse_button(code: u32) -> MouseButton {
    match code {
        0x110 => MouseButton::Left,
        0x111 => MouseButton::Right,
        0x112 => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

fn push_solid(
    out: &mut Vec<SolidColorRenderElement>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: [f32; 4],
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let buffer = SolidColorBuffer::new((w, h), color);
    out.push(SolidColorRenderElement::from_buffer(
        &buffer,
        (x, y),
        Scale::from(1.0),
        1.0,
        Kind::Unspecified,
    ));
}

fn color_to_rgba(color: &crate::drw::Color) -> [f32; 4] {
    [
        color.color.color.red as f32 / 65535.0,
        color.color.color.green as f32 / 65535.0,
        color.color.color.blue as f32 / 65535.0,
        color.color.color.alpha as f32 / 65535.0,
    ]
}

// Bar layout constants (same as wayland.rs).
const CLOSE_BUTTON_WIDTH: i32 = 16;
const CLOSE_BUTTON_DETAIL: i32 = 3;
