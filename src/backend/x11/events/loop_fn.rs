//! X11 event loop built on calloop.
//!
//! This replaces the previous raw `libc::poll` loop with a calloop-based
//! event loop, bringing the X11 backend closer to the Wayland backend's
//! architecture and making animations non-blocking.

use std::os::unix::io::AsRawFd;

use calloop::generic::Generic;
use calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction};

use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::ipc::IpcServer;
use crate::types::WindowId;
use crate::wm::Wm;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

use super::handlers;

pub fn run(wm: &mut Wm, ipc_server: Option<IpcServer>) {
    let mut event_loop: EventLoop<Wm> =
        EventLoop::try_new().expect("failed to create X11 calloop event loop");
    let loop_handle = event_loop.handle();

    // ── X11 connection fd source ────────────────────────────────────────
    let x11_fd = wm
        .backend
        .x11_conn()
        .map(|(conn, _)| conn.stream().as_raw_fd())
        .expect("X11 backend must have a connection");

    let x11_source = Generic::new(
        unsafe { std::os::unix::io::BorrowedFd::borrow_raw(x11_fd) },
        Interest::READ,
        Mode::Level,
    );
    loop_handle
        .insert_source(x11_source, |_, _, _wm| {
            // The actual event draining happens in the main loop body
            // (we need &mut Wm which is the calloop data parameter).
            // This source just wakes the loop when data arrives.
            Ok(PostAction::Continue)
        })
        .expect("failed to insert X11 fd source");

    // ── IPC as event-driven calloop source ──────────────────────────────
    // This mirrors the Wayland/DRM backend approach - IPC is processed
    // immediately when a client connects, not polled every iteration.
    if let Some(ipc) = ipc_server {
        crate::wayland::runtime::calloop_helpers::setup_ipc_source(
            loop_handle.clone(),
            ipc,
            |ipc, wm| {
                if ipc.process_pending(wm) {
                    let mut ctx = wm.ctx();
                    crate::runtime::apply_monitor_config_if_dirty(&mut ctx);
                }
            },
        );
    }

    // ── Animation timer (~60 fps) ───────────────────────────────────────
    // Uses smart timing: 16ms when animations active, long sleep when idle
    // to reduce CPU usage. This mirrors the Wayland/DRM backend approach.
    crate::wayland::runtime::calloop_helpers::setup_animation_timer(
        loop_handle.clone(),
        tick_x11_animations,
        |wm| wm.has_active_animations(),
    );

    let loop_signal: LoopSignal = event_loop.get_signal();

    event_loop
        .run(None, wm, move |wm| {
            // ── 1. Drain all pending X11 events ─────────────────────────
            drain_x11_events(wm);

            // ── 2. Apply monitor config and arrange layout ──────────────
            // NOTE: IPC is now handled by calloop source, not polled here
            let mut ctx = wm.ctx();
            crate::runtime::apply_monitor_config_if_dirty(&mut ctx);
            crate::runtime::arrange_layout_if_dirty(&mut ctx);

            // ── 3. Flush X11 connection ─────────────────────────────────
            BackendRef::from_backend(&wm.backend).flush();

            // ── 4. Stop loop if WM is shutting down ─────────────────────
            if !wm.running {
                loop_signal.stop();
            }
        })
        .expect("X11 event loop run");
}

/// Drain all pending X11 events from the connection and dispatch them.
fn drain_x11_events(wm: &mut Wm) {
    loop {
        let event = wm
            .backend
            .x11_conn()
            .and_then(|(conn, _)| conn.poll_for_event().ok())
            .flatten();
        match event {
            Some(event) => dispatch_event(wm, event),
            None => break,
        }
    }
}

/// Tick active X11 window animations, interpolating geometry each frame.
fn tick_x11_animations(wm: &mut Wm) {
    let data = match wm.backend.x11_data_mut() {
        Some(d) => d,
        None => return,
    };

    if data.x11_runtime.window_animations.is_empty() {
        return;
    }

    let now = std::time::Instant::now();

    // Collect finished animation targets and windows that need geometry updates.
    let mut finished: Vec<(WindowId, crate::types::Rect)> = Vec::new();
    for (win, anim) in data.x11_runtime.window_animations.iter() {
        let elapsed = now.duration_since(anim.started_at);
        let progress = if anim.duration.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f64() / anim.duration.as_secs_f64()).min(1.0)
        };
        let eased = crate::animation::ease_out_cubic(progress);

        let x = anim.from.x as f64 + (anim.to.x - anim.from.x) as f64 * eased;
        let y = anim.from.y as f64 + (anim.to.y - anim.from.y) as f64 * eased;
        let w = anim.from.w as f64 + (anim.to.w - anim.from.w) as f64 * eased;
        let h = anim.from.h as f64 + (anim.to.h - anim.from.h) as f64 * eased;

        let rect = crate::types::Rect {
            x: x.round() as i32,
            y: y.round() as i32,
            w: w.round() as i32,
            h: h.round() as i32,
        };

        if rect.is_valid() {
            let x11_win: x11rb::protocol::xproto::Window = (*win).into();
            let width = rect.w.max(1) as u32;
            let height = rect.h.max(1) as u32;
            let _ = data.conn.configure_window(
                x11_win,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .x(rect.x)
                    .y(rect.y)
                    .width(width)
                    .height(height),
            );
        }

        if progress >= 1.0 {
            finished.push((*win, anim.to));
        }
    }

    // Remove completed animations and update client geometry.
    for (win, to) in &finished {
        data.x11_runtime.window_animations.remove(win);
        if let Some(client) = wm.g.clients.get_mut(win) {
            client.old_geo = client.geo;
            client.geo = *to;
        }
    }
}

pub fn dispatch_event(wm: &mut Wm, event: x11rb::protocol::Event) {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };

    match event {
        x11rb::protocol::Event::ButtonPress(e) => handlers::button_press_x11(&mut ctx, &e),
        x11rb::protocol::Event::ClientMessage(e) => handlers::client_message(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureNotify(e) => handlers::configure_notify(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureRequest(e) => handlers::configure_request(&mut ctx, &e),
        x11rb::protocol::Event::CreateNotify(e) => handlers::create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => handlers::destroy_notify(&mut ctx, &e),
        x11rb::protocol::Event::EnterNotify(e) => handlers::enter_notify(&mut ctx, &e),
        x11rb::protocol::Event::Expose(e) => handlers::expose(&mut ctx, &e),
        x11rb::protocol::Event::FocusIn(e) => handlers::focus_in(&mut ctx, &e),
        x11rb::protocol::Event::KeyPress(e) => {
            crate::backend::x11::grab::key_press_x11(&mut ctx, &e)
        }
        x11rb::protocol::Event::KeyRelease(e) => {
            crate::backend::x11::grab::key_release_x11(&mut ctx, &e)
        }
        x11rb::protocol::Event::MappingNotify(e) => handlers::mapping_notify(&mut ctx, &e),
        x11rb::protocol::Event::MapRequest(e) => handlers::map_request(&mut ctx, &e),
        x11rb::protocol::Event::MotionNotify(e) => handlers::motion_notify(&mut ctx, &e),
        x11rb::protocol::Event::PropertyNotify(e) => handlers::property_notify(&mut ctx, &e),
        x11rb::protocol::Event::ResizeRequest(e) => handlers::resize_request(&mut ctx, &e),
        x11rb::protocol::Event::UnmapNotify(e) => handlers::unmap_notify(&mut ctx, &e),
        x11rb::protocol::Event::LeaveNotify(e) => handlers::leave_notify(&mut ctx, &e),
        _ => {}
    };
}
