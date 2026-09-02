//! X11 event loop built on calloop.
//!
//! This replaces the previous raw `libc::poll` loop with a calloop-based
//! event loop, bringing the X11 backend closer to the Wayland backend's
//! architecture and making animations non-blocking.

use std::os::unix::io::AsRawFd;

use calloop::generic::Generic;
use calloop::{EventLoop, Interest, LoopSignal, Mode, PostAction};

use crate::geometry::GeometryApplyMode;
use crate::ipc::IpcServer;
use crate::runtime::{AnimationTimerGuard, animation_frame_interval};
use crate::wm::Wm;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

use super::handlers;

pub fn run(wm: &mut Wm, ipc_server: &mut Option<IpcServer>) {
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

    // ── IPC listener fd source ──────────────────────────────────────────
    crate::runtime::register_ipc_source(&loop_handle, ipc_server);

    // ── Internal status ping source ────────────────────────────────────
    let (status_ping, status_ping_source) = calloop::ping::make_ping().expect("status ping");
    crate::bar::status::set_internal_status_ping(status_ping);
    loop_handle
        .insert_source(status_ping_source, |_, _, _| {})
        .expect("failed to insert status ping source");

    // ── Region-selection ping source ───────────────────────────────────
    let (slop_ping, slop_ping_source) = calloop::ping::make_ping().expect("slop ping");
    crate::mouse::slop::set_region_selection_ping(slop_ping);
    loop_handle
        .insert_source(slop_ping_source, |_, _, _| {})
        .expect("failed to insert region-selection ping source");

    // ── StatusNotifier worker ───────────────────────────────────────────
    // Items position their own menus under X11, so no native-menu slot is
    // provided; the wake ping makes icon changes render while idle.
    wm.start_systray(None, crate::runtime::make_wake_ping(&loop_handle));

    // ── Animation timer (on-demand, not persistent) ─────────────────────
    let anim_guard = AnimationTimerGuard::new();
    let loop_handle_for_timer = event_loop.handle();
    let animation_interval = wm.backend.x11_data().and_then(|data| {
        crate::backend::x11::randr::max_active_refresh_millihertz(&data.conn, data.x11_runtime.root)
    });
    let animation_interval = animation_frame_interval(animation_interval);

    let loop_signal: LoopSignal = event_loop.get_signal();

    event_loop
        .run(None, wm, move |wm| {
            // ── 1. Drain all pending X11 events ─────────────────────────
            drain_x11_events(wm);

            // ── 2. Shared tick: IPC, monitor config, layout arrangement ─
            crate::runtime::event_loop_tick_with_options(wm, ipc_server, Default::default());

            // X11 focus is projected synchronously. End the shared selection
            // transaction here so changes from separate ticks never coalesce.
            let _ = wm.focus.take_pending_selection();

            // ── 3. Arm animation timer if needed ────────────────────────
            let has_animations = has_x11_animations(wm);
            anim_guard.ensure_armed_with_interval(
                has_animations,
                animation_interval,
                &loop_handle_for_timer,
                |wm| {
                    tick_x11_animations(wm);
                    has_x11_animations(wm)
                },
            );

            // ── 4. Flush X11 connection ─────────────────────────────────
            crate::backend::WindowOps::flush(&wm.backend);

            // ── 5. Stop loop if WM is shutting down ─────────────────────
            if !wm.running {
                loop_signal.stop();
            }
        })
        .expect("X11 event loop run");
}

fn has_x11_animations(wm: &Wm) -> bool {
    wm.backend.x11_data().is_some_and(|data| {
        !data.x11_runtime.window_animations.is_empty()
            || data.x11_runtime.layout_preview_animation.is_active()
    })
}

/// Drain all pending X11 events from the connection and dispatch them.
fn drain_x11_events(wm: &mut Wm) {
    let mut raw_motion_pending = false;
    while let Some((conn, _)) = wm.backend.x11_conn() {
        match conn.poll_for_event() {
            Ok(Some(x11rb::protocol::Event::XinputRawMotion(_))) => {
                // Raw motion carries device valuators rather than the
                // accelerated root position used by shared pointer policy.
                // Collapse a queued run into one root-position snapshot so a
                // high-rate device cannot force one synchronous QueryPointer
                // round trip per sample.
                raw_motion_pending = true;
            }
            Ok(Some(event)) => {
                if raw_motion_pending && event_requires_current_pointer_state(&event) {
                    dispatch_raw_motion(wm);
                    raw_motion_pending = false;
                }
                dispatch_event(wm, event);
            }
            Ok(None) => {
                if raw_motion_pending {
                    dispatch_raw_motion(wm);
                    raw_motion_pending = false;
                    // Waiting for QueryPointer may also read and queue events
                    // from the connection. Drain those now; the fd need not
                    // remain readable once x11rb owns the parsed events.
                    continue;
                }
                break;
            }
            Err(err) => {
                log::warn!("X11 poll_for_event error: {}", err);
                if raw_motion_pending {
                    dispatch_raw_motion(wm);
                }
                break;
            }
        }
    }
}

/// Events whose semantics depend on hover state established by earlier
/// motion must not overtake the coalesced pointer update. Unrelated display
/// traffic remains freely batchable, so Expose/Configure bursts cannot split
/// one raw-motion run into repeated server round trips.
fn event_requires_current_pointer_state(event: &x11rb::protocol::Event) -> bool {
    matches!(
        event,
        x11rb::protocol::Event::ButtonPress(_)
            | x11rb::protocol::Event::EnterNotify(_)
            | x11rb::protocol::Event::LeaveNotify(_)
            | x11rb::protocol::Event::XinputTouchBegin(_)
    )
}

fn dispatch_raw_motion(wm: &mut Wm) {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };
    handlers::raw_motion_notify(&mut ctx);
}

/// Tick active X11 window animations, interpolating geometry each frame.
fn tick_x11_animations(wm: &mut Wm) {
    let finished_targets = {
        let data = match wm.backend.x11_data_mut() {
            Some(d) => d,
            None => return,
        };

        let preview_active = data.x11_runtime.layout_preview_animation.is_active();
        if data.x11_runtime.window_animations.is_empty() && !preview_active {
            return;
        }

        let now = std::time::Instant::now();
        if preview_active {
            let x11 = crate::backend::x11::X11BackendRef::new(&data.conn, data.screen_num);
            crate::backend::x11::keyboard::tick_layout_preview(&x11, &mut data.x11_runtime, now);
        }
        let mut finished = Vec::new();
        let mut finished_targets = Vec::new();
        let mut needs_flush = false;

        for (win, anim) in data.x11_runtime.window_animations.iter() {
            let tick = anim.tick(now);
            let rect = tick.rect;

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
                needs_flush = true;
            }

            if tick.done {
                finished.push(*win);
            }
        }

        for win in &finished {
            // Completed animations snap to their final target.
            if let Some(target) = data
                .x11_runtime
                .take_window_animation(*win)
                .map(|anim| anim.to)
            {
                finished_targets.push((*win, target));
            }
        }

        if needs_flush {
            let _ = data.conn.flush();
        }

        finished_targets
    };

    if finished_targets.is_empty() {
        return;
    }

    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };
    for (win, rect) in finished_targets {
        let mut wmctx = crate::contexts::WmCtx::X11(ctx.reborrow());
        wmctx.set_geometry_impl(win, rect, GeometryApplyMode::Logical);
    }
}

pub fn dispatch_event(wm: &mut Wm, event: x11rb::protocol::Event) {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
        return;
    };

    match event {
        x11rb::protocol::Event::ButtonPress(e) => handlers::button_press(&mut ctx, &e),
        x11rb::protocol::Event::ClientMessage(e) => handlers::client_message(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureNotify(e) => handlers::configure_notify(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureRequest(e) => handlers::configure_request(&mut ctx, &e),
        x11rb::protocol::Event::CreateNotify(e) => handlers::create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => handlers::destroy_notify(&mut ctx, &e),
        x11rb::protocol::Event::EnterNotify(e) => handlers::enter_notify(&mut ctx, &e),
        x11rb::protocol::Event::Expose(e) => handlers::expose(&mut ctx, &e),
        x11rb::protocol::Event::FocusIn(e) => handlers::focus_in(&mut ctx, &e),
        x11rb::protocol::Event::KeyPress(e) => {
            crate::backend::x11::keyboard::key_press(&mut ctx, &e)
        }
        x11rb::protocol::Event::KeyRelease(_) => crate::backend::x11::keyboard::key_release(),
        x11rb::protocol::Event::MappingNotify(e) => handlers::mapping_notify(&mut ctx, &e),
        x11rb::protocol::Event::MapRequest(e) => handlers::map_request(&mut ctx, &e),
        x11rb::protocol::Event::MotionNotify(e) => handlers::motion_notify(&mut ctx, &e),
        x11rb::protocol::Event::RandrNotify(_) => handlers::randr_notify(&mut ctx),
        x11rb::protocol::Event::RandrScreenChangeNotify(e) => {
            handlers::randr_screen_change_notify(&mut ctx, &e)
        }
        // Raw motion is coalesced by `drain_x11_events`; dispatching an
        // individual sample would reintroduce a QueryPointer round trip per
        // device event.
        x11rb::protocol::Event::XinputRawMotion(_) => {}
        x11rb::protocol::Event::XinputTouchBegin(e) => handlers::touch_begin(&mut ctx, &e),
        x11rb::protocol::Event::PropertyNotify(e) => handlers::property_notify(&mut ctx, &e),
        x11rb::protocol::Event::ResizeRequest(e) => handlers::resize_request(&mut ctx, &e),
        x11rb::protocol::Event::UnmapNotify(e) => handlers::unmap_notify(&mut ctx, &e),
        x11rb::protocol::Event::LeaveNotify(e) => handlers::leave_notify(&mut ctx, &e),
        _ => {}
    };
}
