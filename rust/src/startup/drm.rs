//! DRM/KMS bare-metal backend for running directly on hardware.
//!
//! Uses libseat for session management, udev for GPU discovery, libinput
//! for input, and DRM/GBM/EGL for rendering.  This backend is vblank-driven:
//! each page-flip completion triggers the next frame.

use std::process::{exit, Stdio};
use std::sync::Arc;
use std::time::Duration;

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{Bind, ImportDma};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev;
use smithay::desktop::space::render_output;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::desktop::PopupManager;
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::Display;
use smithay::utils::{DeviceFd, Physical, Rectangle};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::xwayland::{X11Wm, XWayland, XWaylandEvent};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::config::init_config;
use crate::monitor::update_geom;
use crate::startup::common_wayland::{
    wayland_font_height_from_size, wayland_font_size_from_config,
};
use crate::types::*;
use crate::wm::Wm;

use super::autostart::run_autostart;

/// Default screen width when no DRM outputs are detected
const DEFAULT_SCREEN_WIDTH: i32 = 1280;
/// Default screen height when no DRM outputs are detected
const DEFAULT_SCREEN_HEIGHT: i32 = 800;

// Access drm/rustix types through smithay's re-exports.
use drm::control::{connector, crtc};
use smithay::reexports::drm;
use smithay::reexports::rustix;

// Re-use the same render element enum from the winit backend.
render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════

pub fn run() -> ! {
    log::info!("Starting DRM/KMS backend");

    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_drm_globals(&mut wm);

    // Use EventLoop<WaylandState> so that WaylandState::new, XWayland, etc.
    // all type-check against the same calloop state type.
    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("event loop");
    let loop_handle = event_loop.handle();

    // ── Session ──────────────────────────────────────────────────────
    let (mut session, notifier) = LibSeatSession::new().expect("libseat session");
    let seat_name = session.seat();
    log::info!("Session on seat: {seat_name}");

    // ── Wayland display ──────────────────────────────────────────────
    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut state = WaylandState::new(display, &loop_handle);
    state.attach_globals(&mut wm.g);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut state);
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
    log::info!("Using GPU: {:?}", primary_gpu_path);

    // ── Open DRM device via session ──────────────────────────────────
    let fd = session
        .open(
            &primary_gpu_path,
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOCTTY
                | rustix::fs::OFlags::NONBLOCK,
        )
        .expect("session open DRM device");
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (mut drm_device, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), true).expect("DrmDevice::new");

    // ── GBM + EGL + GLES renderer ───────────────────────────────────
    let gbm_device = GbmDevice::new(drm_fd.clone()).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    state.attach_renderer(&mut renderer);
    state.init_dmabuf_global(renderer.dmabuf_formats().into_iter().collect());

    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    // ── Scan connectors and create outputs ───────────────────────────
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;
    let mut mon_idx_counter: usize = 0;

    // DrmDevice derefs to the raw drm ControlDevice.
    // We query resources through the drm crate's ControlDevice trait.
    {
        use drm::control::{Device as ControlDevice, ModeTypeFlags};

        let res = drm_device.resource_handles().expect("drm resource_handles");
        let _available_crtcs: Vec<crtc::Handle> = res.crtcs().to_vec();
        let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

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

            // Find an available CRTC.
            let encoder_crtcs: Vec<crtc::Handle> = conn_info
                .encoders()
                .iter()
                .filter_map(|&enc_h| drm_device.get_encoder(enc_h).ok())
                .flat_map(|enc| res.filter_crtcs(enc.possible_crtcs()))
                .collect();

            let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) else {
                continue;
            };
            used_crtcs.push(picked_crtc);

            let drm_surface = drm_device
                .create_surface(picked_crtc, mode, &[conn_handle])
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
            let output_name = format!(
                "{}-{}",
                connector_type_name(conn_info.interface()),
                conn_info.interface_id()
            );
            log::info!(
                "Output {output_name}: {mode_w}x{mode_h}@{}Hz on CRTC {:?}",
                mode.vrefresh(),
                picked_crtc
            );

            let output = Output::new(
                output_name,
                PhysicalProperties {
                    size: {
                        let (mm_w, mm_h) = conn_info.size().unwrap_or((0, 0));
                        (mm_w as i32, mm_h as i32).into()
                    },
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
                Some(smithay::utils::Transform::Normal),
                Some(Scale::Integer(1)),
                Some((output_x_offset, 0).into()),
            );
            output.set_preferred(out_mode);
            let _global = output.create_global::<WaylandState>(&state.display_handle);
            state.space.map_output(&output, (output_x_offset, 0));

            let damage_tracker = OutputDamageTracker::from_output(&output);
            let mon_idx = mon_idx_counter;
            mon_idx_counter += 1;

            output_surfaces.push(OutputSurfaceEntry {
                crtc: picked_crtc,
                surface: gbm_surface,
                output,
                damage_tracker,
                mon_idx,
            });
            output_x_offset += mode_w;
        }
    }

    let _total_width = output_x_offset;
    let _total_height = output_surfaces
        .iter()
        .map(|s| s.output.current_mode().unwrap().size.h)
        .max()
        .unwrap_or(800);

    // Sync instantWM monitor state.
    sync_monitors_from_outputs_vec(&mut wm, &output_surfaces);

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
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    // ── XWayland ─────────────────────────────────────────────────────
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
                        Ok(wm_handle) => data.xwm = Some(wm_handle),
                        Err(e) => log::error!("failed to start X11 WM: {e}"),
                    }
                }
                XWaylandEvent::Error => log::error!("XWayland failed"),
            }) {
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

    // ── Session events ───────────────────────────────────────────────
    loop_handle
        .insert_source(notifier, move |event, _, _data| match event {
            SessionEvent::PauseSession => {
                log::info!("Session paused (VT switch away)");
            }
            SessionEvent::ActivateSession => {
                log::info!("Session activated (VT switch back)");
            }
        })
        .expect("session source");

    // ── DRM vblank events ────────────────────────────────────────────
    loop_handle
        .insert_source(drm_notifier, |event, _metadata, _data| match event {
            DrmEvent::VBlank(_crtc) => {
                // Handled in the main loop tick.
            }
            DrmEvent::Error(err) => {
                log::error!("DRM error: {err}");
            }
        })
        .expect("drm notifier source");

    // ── libinput source ──────────────────────────────────────────────
    loop_handle
        .insert_source(libinput_backend, |_event, _, _data| {
            // Input events to be processed via polling in the main loop.
        })
        .expect("libinput source");

    run_autostart();

    let mut ipc_server = crate::ipc::IpcServer::bind().ok();
    let start_time = std::time::Instant::now();
    let session_active = true;

    // Move DRM-specific state into the main loop closure.
    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            state.attach_globals(&mut wm.g);

            // ── Layout + IPC ─────────────────────────────────────────
            {
                let mut ctx = wm.ctx();
                if !ctx.g.clients.is_empty() {
                    let selected_monitor_id = ctx.g.selected_monitor_id();
                    crate::layouts::arrange(&mut ctx, Some(selected_monitor_id));
                }
            }
            if let Some(server) = ipc_server.as_mut() {
                server.process_pending(&mut wm);
            }
            state.sync_space_from_globals();

            // ── Render all outputs ───────────────────────────────────
            if session_active {
                for entry in output_surfaces.iter_mut() {
                    render_drm_output(&mut wm, state, &mut renderer, entry, start_time);
                }
            }

            if state.display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("event loop run");

    exit(0);
}

// Temporary struct for collecting output data before the event loop starts.
struct OutputSurfaceEntry {
    crtc: drm::control::crtc::Handle,
    surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    output: Output,
    damage_tracker: OutputDamageTracker,
    mon_idx: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// Rendering
// ═══════════════════════════════════════════════════════════════════════════

fn render_drm_output(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    start_time: std::time::Instant,
) {
    // Acquire the next buffer from the swapchain.
    let (dmabuf, age) = match entry.surface.next_buffer() {
        Ok(buf) => buf,
        Err(e) => {
            log::trace!("next_buffer: {e}");
            return;
        }
    };

    // Bind the dmabuf to the renderer.
    let mut dmabuf_clone = dmabuf.clone();
    let Ok(mut target) = renderer.bind(&mut dmabuf_clone) else {
        log::warn!("renderer bind failed");
        return;
    };

    // Build custom elements (bar + borders).
    let mut custom_elements: Vec<DrmExtras> = Vec::new();

    if wm.g.cfg.showbar {
        let mut ctx = wm.ctx();
        let bar_buffers = crate::bar::wayland::render_bar_buffers(
            &mut ctx.core,
            smithay::utils::Scale::from(1.0),
        );
        for (buffer, x, y) in bar_buffers {
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (x as f64, y as f64),
                &buffer,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                Ok(elem) => custom_elements.push(DrmExtras::Memory(elem)),
                Err(e) => log::warn!("bar buffer upload: {:?}", e),
            }
        }
    }

    for elem in crate::startup::wayland::wayland_border_elements_shared(&wm.g, state) {
        custom_elements.push(DrmExtras::Solid(elem));
    }

    // Render.
    let render_result = render_output(
        &entry.output,
        renderer,
        &mut target,
        1.0,
        age as usize,
        [&state.space],
        &custom_elements,
        &mut entry.damage_tracker,
        [0.05, 0.05, 0.07, 1.0],
    );
    drop(target);

    match render_result {
        Ok(result) => {
            let damage: Option<Vec<Rectangle<i32, Physical>>> = result.damage.cloned();
            if let Err(e) = entry.surface.queue_buffer(None, damage, ()) {
                log::warn!("queue_buffer: {e}");
            }
        }
        Err(e) => {
            log::warn!("render_output: {:?}", e);
        }
    }

    // Send frame callbacks.
    let time = start_time.elapsed();
    for window in state.space.elements() {
        if let Some(wl_surface) = window.wl_surface() {
            send_frames_surface_tree(
                &wl_surface,
                &entry.output,
                time,
                Some(Duration::from_millis(16)),
                surface_primary_scanout_output,
            );
            if let Some(toplevel) = window.toplevel() {
                for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                    send_frames_surface_tree(
                        popup.wl_surface(),
                        &entry.output,
                        time,
                        Some(Duration::from_millis(16)),
                        surface_primary_scanout_output,
                    );
                }
            }
        }
    }
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
    wm.g.cfg.bar_height = (if cfg.bar_height > 0 {
        font_height + cfg.bar_height
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    wm.g.cfg.horizontal_padding = font_height;
    wm.g.x11.numlockmask = 0;
    update_geom(&mut wm.ctx());
}

fn sync_monitors_from_outputs_vec(wm: &mut Wm, surfaces: &[OutputSurfaceEntry]) {
    wm.g.monitors.clear();
    let tag_template = wm.g.cfg.tag_template.clone();

    for (i, surface) in surfaces.iter().enumerate() {
        let mode = surface.output.current_mode().unwrap();
        let (w, h) = (mode.size.w, mode.size.h);
        let pos = surface.output.current_location();
        let x = pos.x;
        let y = pos.y;

        let mut mon = crate::types::Monitor::new_with_values(
            wm.g.cfg.mfact,
            wm.g.cfg.nmaster,
            wm.g.cfg.showbar,
            wm.g.cfg.topbar,
        );
        mon.num = i as i32;
        mon.monitor_rect = Rect { x, y, w, h };
        mon.work_rect = Rect { x, y, w, h };
        mon.current_tag = 1;
        mon.prev_tag = 1;
        mon.tag_set = [1, 1];
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    wm.g.cfg.screen_width = surfaces
        .iter()
        .map(|s| {
            let pos = s.output.current_location();
            let mode = s.output.current_mode().unwrap();
            pos.x + mode.size.w
        })
        .max()
        .unwrap_or(DEFAULT_SCREEN_WIDTH);
    wm.g.cfg.screen_height = surfaces
        .iter()
        .map(|s| {
            let pos = s.output.current_location();
            let mode = s.output.current_mode().unwrap();
            pos.y + mode.size.h
        })
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);

    if wm.g.monitors.is_empty() {
        let mut mon = crate::types::Monitor::new_with_values(
            wm.g.cfg.mfact,
            wm.g.cfg.nmaster,
            wm.g.cfg.showbar,
            wm.g.cfg.topbar,
        );
        mon.monitor_rect = Rect {
            x: 0,
            y: 0,
            w: DEFAULT_SCREEN_WIDTH,
            h: DEFAULT_SCREEN_HEIGHT,
        };
        mon.work_rect = Rect {
            x: 0,
            y: 0,
            w: DEFAULT_SCREEN_WIDTH,
            h: DEFAULT_SCREEN_HEIGHT,
        };
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    for (i, mon) in wm.g.monitors.iter_mut() {
        mon.num = i as i32;
    }

    if wm.g.selected_monitor_id() >= wm.g.monitors.count() {
        wm.g.set_selected_monitor(0);
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

fn connector_type_name(interface: drm::control::connector::Interface) -> &'static str {
    match interface {
        drm::control::connector::Interface::DVII => "DVI-I",
        drm::control::connector::Interface::DVID => "DVI-D",
        drm::control::connector::Interface::DVIA => "DVI-A",
        drm::control::connector::Interface::SVideo => "S-Video",
        drm::control::connector::Interface::DisplayPort => "DP",
        drm::control::connector::Interface::HDMIA => "HDMI-A",
        drm::control::connector::Interface::HDMIB => "HDMI-B",
        drm::control::connector::Interface::EmbeddedDisplayPort => "eDP",
        drm::control::connector::Interface::VGA => "VGA",
        drm::control::connector::Interface::LVDS => "LVDS",
        drm::control::connector::Interface::DSI => "DSI",
        drm::control::connector::Interface::DPI => "DPI",
        drm::control::connector::Interface::Composite => "Composite",
        drm::control::connector::Interface::TV => "TV",
        _ => "Unknown",
    }
}
