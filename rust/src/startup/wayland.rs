use std::process::{exit, Command};
use std::sync::Arc;
use std::time::Duration;

use smithay::backend::input::{
    AbsolutePositionEvent, Event, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
    PointerButtonEvent,
};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportMem;
use smithay::backend::winit::{self, WinitEvent};
use smithay::desktop::space::render_output;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::desktop::PopupManager;
use smithay::input::keyboard::FilterResult;
use smithay::output::Mode as OutputMode;
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::wayland_server::Display;
use smithay::utils::{Point, Scale, Transform, SERIAL_COUNTER};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;

use crate::backend::wayland::compositor::{
    KeyboardFocusTarget, PointerFocusTarget, WaylandClientState, WaylandState, WindowIdMarker,
};
use crate::backend::wayland::WaylandBackend;
use crate::backend::Backend as WmBackend;
use crate::bar::{bar_position_at_x, bar_position_to_gesture};
use crate::config::init_config;
use crate::monitor;
use crate::types::*;
use crate::wm::Wm;

use super::autostart::run_autostart;

render_elements! {
    pub WaylandExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

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

    let (mut backend, mut winit_loop) =
        winit::init::<GlesRenderer>().expect("failed to init winit backend");
    let output_size = backend.window_size();
    let (initial_w, initial_h) = sanitize_wayland_size(output_size.w, output_size.h);
    wm.g.cfg.screen_width = initial_w;
    wm.g.cfg.screen_height = initial_h;
    monitor::update_geom_ctx(&mut wm.ctx());

    let output = state.create_output("winit", initial_w, initial_h);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    std::env::remove_var("DISPLAY");
    std::env::set_var("GDK_BACKEND", "wayland");
    std::env::set_var("QT_QPA_PLATFORM", "wayland");
    std::env::set_var("SDL_VIDEODRIVER", "wayland");
    std::env::set_var("CLUTTER_BACKEND", "wayland");

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

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
                    let (safe_w, safe_h) = sanitize_wayland_size(size.w, size.h);
                    let mode = OutputMode {
                        size: (safe_w, safe_h).into(),
                        refresh: 60_000,
                    };
                    wm.g.cfg.screen_width = safe_w;
                    wm.g.cfg.screen_height = safe_h;
                    monitor::update_geom_ctx(&mut wm.ctx());
                    output.change_current_state(
                        Some(mode),
                        // Keep Flipped180: this matches Smithay's demo compositor
                        // behavior in this backend and avoids inverted output.
                        Some(Transform::Flipped180),
                        None,
                        Some((0, 0).into()),
                    );
                    output.set_preferred(mode);
                }
                WinitEvent::Input(event) => match event {
                    InputEvent::Keyboard { event } => {
                        let serial = SERIAL_COUNTER.next_serial();
                        keyboard_handle.input(
                            state,
                            event.key_code(),
                            event.state(),
                            serial,
                            event.time() as u32,
                            |_data: &mut WaylandState,
                             modifiers: &smithay::input::keyboard::ModifiersState,
                             keysym: smithay::input::keyboard::KeysymHandle<'_>| {
                                if event.state() == smithay::backend::input::KeyState::Pressed {
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
                    InputEvent::PointerMotionAbsolute { event } => {
                        let size = backend.window_size();
                        let x = event.x_transformed(size.w);
                        let y = event.y_transformed(size.h);
                        pointer_location = Point::from((x, y));

                        let element_under = state.space.element_under(pointer_location);
                        let mut hovered_win = element_under.as_ref().and_then(|(window, _)| {
                            window.user_data().get::<WindowIdMarker>().map(|m| m.0)
                        });
                        // If the pointer isn't over a surface, check if it's
                        // inside a window's outer rect (content + borders),
                        // following stack order for deterministic focus edges.
                        if hovered_win.is_none() {
                            let px = pointer_location.x as i32;
                            let py = pointer_location.y as i32;
                            if let Some(mon) = wm.g.selmon() {
                                let selected = mon.selected_tags();
                                for (w, c) in mon.iter_stack(&wm.g.clients) {
                                    if !c.is_visible_on_tags(selected) || c.is_hidden {
                                        continue;
                                    }
                                    let bw = c.border_width.max(0);
                                    let ox = c.geo.x;
                                    let oy = c.geo.y;
                                    let ow = c.geo.w + 2 * bw;
                                    let oh = c.geo.h + 2 * bw;
                                    if px >= ox && px < ox + ow && py >= oy && py < oy + oh {
                                        hovered_win = Some(w);
                                        break;
                                    }
                                }
                            }
                        }
                        {
                            let mut ctx = wm.ctx();
                            crate::focus::hover_focus_target(&mut ctx, hovered_win, false);
                        }

                        {
                            let root_x = pointer_location.x.round() as i32;
                            let root_y = pointer_location.y.round() as i32;
                            let rect = Rect {
                                x: root_x,
                                y: root_y,
                                w: 1,
                                h: 1,
                            };
                            if let Some(mid) =
                                crate::types::find_monitor_by_rect(&wm.g.monitors, &rect)
                            {
                                let mut ctx = wm.ctx();
                                if mid != ctx.g.selmon_id() {
                                    ctx.g.set_selmon(mid);
                                }
                                let bar_h = ctx.g.cfg.bar_height.max(1);
                                let in_bar = ctx.g.selmon().is_some_and(|m| {
                                    m.showbar && root_y >= m.by && root_y < m.by + bar_h
                                });
                                let gesture = if in_bar {
                                    if let Some(mon) = ctx.g.selmon().cloned() {
                                        let local_x = root_x - mon.monitor_rect.x;
                                        let pos = bar_position_at_x(&mon, &ctx, local_x);
                                        if pos == BarPosition::StatusText {
                                            ctx.g.selmon().map(|m| m.gesture).unwrap_or_default()
                                        } else {
                                            bar_position_to_gesture(pos)
                                        }
                                    } else {
                                        Gesture::None
                                    }
                                } else {
                                    Gesture::None
                                };
                                if let Some(m) = ctx.g.selmon_mut() {
                                    m.gesture = gesture;
                                }
                            }
                        }

                        let focus = match element_under {
                            Some((window, location)) => window.wl_surface().map(|surface| {
                                (
                                    PointerFocusTarget::WlSurface(surface.into_owned()),
                                    location.to_f64(),
                                )
                            }),
                            None => None,
                        };

                        let serial = SERIAL_COUNTER.next_serial();
                        let motion = smithay::input::pointer::MotionEvent {
                            location: pointer_location,
                            serial,
                            time: event.time() as u32,
                        };
                        pointer_handle.motion(state, focus, &motion);
                        pointer_handle.frame(state);
                    }
                    InputEvent::PointerButton { event } => {
                        let serial = SERIAL_COUNTER.next_serial();
                        let button = smithay::input::pointer::ButtonEvent {
                            serial,
                            time: event.time() as u32,
                            button: event.button_code(),
                            state: event.state(),
                        };
                        pointer_handle.button(state, &button);
                        if event.state() == smithay::backend::input::ButtonState::Pressed {
                            let root_x = pointer_location.x.round() as i32;
                            let root_y = pointer_location.y.round() as i32;
                            let rect = Rect {
                                x: root_x,
                                y: root_y,
                                w: 1,
                                h: 1,
                            };
                            if let Some(mid) =
                                crate::types::find_monitor_by_rect(&wm.g.monitors, &rect)
                            {
                                let mut ctx = wm.ctx();
                                if mid != ctx.g.selmon_id() {
                                    ctx.g.set_selmon(mid);
                                }
                                let bar_h = ctx.g.cfg.bar_height.max(1);
                                let in_bar = ctx.g.selmon().is_some_and(|m| {
                                    m.showbar && root_y >= m.by && root_y < m.by + bar_h
                                });
                                if in_bar {
                                    if let Some(mon) = ctx.g.selmon().cloned() {
                                        let local_x = root_x - mon.monitor_rect.x;
                                        let pos = bar_position_at_x(&mon, &ctx, local_x);
                                        if pos == BarPosition::StartMenu {
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
                                        let buttons = ctx.g.cfg.buttons.clone();
                                        if let Some(btn_code) =
                                            wayland_button_to_wm_button(event.button_code())
                                        {
                                            let numlockmask = ctx.g.cfg.numlockmask;
                                            let clean_state = crate::util::clean_mask(
                                                modifiers_to_x11_mask(
                                                    &keyboard_handle.modifier_state(),
                                                ),
                                                numlockmask,
                                            );
                                            for b in &buttons {
                                                if !b.matches(pos) || b.button.as_u8() != btn_code {
                                                    continue;
                                                }
                                                if crate::util::clean_mask(b.mask, numlockmask)
                                                    != clean_state
                                                {
                                                    continue;
                                                }
                                                (b.action)(
                                                    &mut ctx,
                                                    ButtonArg {
                                                        pos,
                                                        btn: b.button,
                                                        rx: root_x,
                                                        ry: root_y,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            let keyboard_focus = state
                                .space
                                .element_under(pointer_location)
                                .map(|(window, _)| KeyboardFocusTarget::Window(window.clone()));
                            keyboard_handle.set_focus(state, keyboard_focus, serial);
                        }
                        pointer_handle.frame(state);
                    }
                    InputEvent::PointerAxis { event } => {
                        let mut frame =
                            smithay::input::pointer::AxisFrame::new(event.time() as u32);
                        frame = frame.source(event.source());
                        if let Some(amount) = event.amount(smithay::backend::input::Axis::Vertical)
                        {
                            frame = frame.value(smithay::backend::input::Axis::Vertical, amount);
                        }
                        if let Some(amount) =
                            event.amount(smithay::backend::input::Axis::Horizontal)
                        {
                            frame = frame.value(smithay::backend::input::Axis::Horizontal, amount);
                        }
                        if let Some(steps) =
                            event.amount_v120(smithay::backend::input::Axis::Vertical)
                        {
                            frame =
                                frame.v120(smithay::backend::input::Axis::Vertical, steps as i32);
                        }
                        if let Some(steps) =
                            event.amount_v120(smithay::backend::input::Axis::Horizontal)
                        {
                            frame =
                                frame.v120(smithay::backend::input::Axis::Horizontal, steps as i32);
                        }
                        pointer_handle.axis(state, frame);
                        pointer_handle.frame(state);
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
                if !ctx.g.clients.is_empty() {
                    let selmon = ctx.g.selmon_id();
                    crate::layouts::arrange(&mut ctx, Some(selmon));
                }
            }
            if let Some(server) = ipc_server.as_mut() {
                server.process_pending(&mut wm);
            }
            state.sync_space_from_globals();
            let age = 0;
            let damage = {
                let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");
                let mut custom_elements: Vec<WaylandExtras> = Vec::new();
                if wm.g.cfg.showbar {
                    let mut ctx = wm.ctx();
                    let bar_buffers =
                        crate::bar::wayland::render_bar_buffers(&mut ctx, Scale::from(1.0));
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
                            Ok(elem) => custom_elements.push(WaylandExtras::Memory(elem)),
                            Err(e) => {
                                log::warn!("bar buffer upload failed: {:?}", e);
                            }
                        }
                    }
                }
                for elem in wayland_border_elements(&wm) {
                    custom_elements.push(WaylandExtras::Solid(elem));
                }

                for window in state.space.elements() {
                    let location = state.space.element_location(window).unwrap_or_default();
                    if let Some(toplevel) = window.toplevel() {
                        for (popup, popup_location) in
                            PopupManager::popups_for_surface(toplevel.wl_surface())
                        {
                            let render_location = (location + popup_location)
                                .to_f64()
                                .to_physical(1.0)
                                .to_i32_round();
                            for elem in render_elements_from_surface_tree::<
                                GlesRenderer,
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
                                renderer,
                                popup.wl_surface(),
                                render_location,
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            ) {
                                custom_elements.push(WaylandExtras::Surface(elem));
                            }
                        }
                    }
                }

                let render_result = render_output(
                    &output,
                    renderer,
                    &mut framebuffer,
                    1.0,
                    age,
                    [&state.space],
                    &custom_elements,
                    &mut damage_tracker,
                    [0.05, 0.05, 0.07, 1.0],
                )
                .expect("render output");

                render_result.damage.cloned()
            };
            let _ = backend.submit(damage.as_deref());

            let time = start_time.elapsed();
            for window in state.space.elements() {
                if let Some(surface) = window.wl_surface() {
                    send_frames_surface_tree(
                        &surface,
                        &output,
                        time,
                        Some(Duration::from_millis(16)),
                        surface_primary_scanout_output,
                    );
                    if let Some(toplevel) = window.toplevel() {
                        for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                            send_frames_surface_tree(
                                popup.wl_surface(),
                                &output,
                                time,
                                Some(Duration::from_millis(16)),
                                surface_primary_scanout_output,
                            );
                        }
                    }
                }
            }

            if display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("wayland event loop run");
    exit(0);
}

fn wayland_border_elements(wm: &Wm) -> Vec<SolidColorRenderElement> {
    let scheme = wm.g.cfg.borderscheme.as_ref();
    let bordercolors = &wm.g.cfg.bordercolors;
    let mut out = Vec::new();
    let sel = wm.g.selected_win();
    for c in wm.g.clients.values() {
        let bw = c.border_width.max(0);
        if bw <= 0 || c.geo.w <= 0 || c.geo.h <= 0 {
            continue;
        }
        let is_visible = c
            .mon_id
            .and_then(|mid| wm.g.monitor(mid))
            .map(|m| c.is_visible_on_tags(m.selected_tags()))
            .unwrap_or(false);
        if !is_visible || c.is_hidden {
            continue;
        }
        let has_tiling = c
            .mon_id
            .and_then(|mid| wm.g.monitor(mid))
            .map(|m| m.is_tiling_layout())
            .unwrap_or(true);
        let rgba = if Some(c.win) == sel {
            if c.isfloating || !has_tiling {
                cfg_hex_to_rgba(Some(
                    bordercolors.get(crate::config::SchemeBorder::FloatFocus),
                ))
                .or_else(|| scheme.map(|s| color_to_rgba(&s.float_focus.bg)))
                .unwrap_or([0.75, 0.40, 0.28, 1.0])
            } else {
                cfg_hex_to_rgba(Some(
                    bordercolors.get(crate::config::SchemeBorder::TileFocus),
                ))
                .or_else(|| scheme.map(|s| color_to_rgba(&s.tile_focus.bg)))
                .unwrap_or([0.28, 0.52, 0.77, 1.0])
            }
        } else {
            cfg_hex_to_rgba(Some(bordercolors.get(crate::config::SchemeBorder::Normal)))
                .or_else(|| scheme.map(|s| color_to_rgba(&s.normal.bg)))
                .unwrap_or([0.18, 0.18, 0.20, 1.0])
        };

        // geo stores content position/size; the outer rect includes borders.
        let x = c.geo.x;
        let y = c.geo.y;
        let ow = c.geo.w + 2 * bw; // outer width
        let oh = c.geo.h + 2 * bw; // outer height
                                   // Top edge
        push_solid(&mut out, x, y, ow, bw, rgba);
        // Bottom edge
        push_solid(&mut out, x, y + oh - bw, ow, bw, rgba);
        // Left edge
        push_solid(&mut out, x, y + bw, bw, (oh - 2 * bw).max(0), rgba);
        // Right edge
        push_solid(
            &mut out,
            x + ow - bw,
            y + bw,
            bw,
            (oh - 2 * bw).max(0),
            rgba,
        );
    }
    out
}

fn cfg_hex_to_rgba(color: Option<&str>) -> Option<[f32; 4]> {
    let s = color?.trim();
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    let a = if hex.len() == 8 {
        u8::from_str_radix(&hex[6..8], 16).ok()?
    } else {
        255
    };
    Some([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
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

fn init_wayland_globals(wm: &mut Wm) {
    let cfg = init_config();
    wm.g.cfg.screen_width = 1280;
    wm.g.cfg.screen_height = 800;
    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.cfg.showbar = true;
    wm.g.cfg.bar_height = if cfg.barheight > 0 {
        cfg.barheight + 12
    } else {
        24
    };
    // Approximate font metrics for bar hit-testing (no X11 drw on Wayland).
    wm.g.cfg.horizontal_padding = 12;
    wm.g.cfg.numlockmask = 0;
    monitor::update_geom_ctx(&mut wm.ctx());
}

#[inline]
fn sanitize_wayland_size(w: i32, h: i32) -> (i32, i32) {
    const WAYLAND_MIN_DIM: i32 = 64;
    (w.max(WAYLAND_MIN_DIM), h.max(WAYLAND_MIN_DIM))
}

fn spawn_wayland_smoke_window() {
    if std::env::var("INSTANTWM_WL_AUTOSPAWN").ok().as_deref() == Some("0") {
        return;
    }

    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(800));
        let _ = Command::new("sh")
            .arg("-lc")
            .arg("for app in gtk3-demo thunar xmessage; do command -v \"$app\" >/dev/null 2>&1 && exec \"$app\"; done; exit 0")
            .spawn();
    });
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
