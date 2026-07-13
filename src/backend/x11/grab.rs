//! X11 pointer-grab helpers.
//!
//! This module handles active, modal pointer grabs ([`grab_pointer`], [`ungrab`])
//! used during interactive move/resize loops.  Every drag loop in
//! `drag.rs` and `resize.rs` calls [`grab_pointer`] at the start and
//! [`ungrab`] when it exits.
//!
//! # Typical drag loop skeleton
//!
//! ```text
//! if !grab_pointer(ctx, x11_runtime, cursor) { return; }
//! loop {
//!     let Some(event) = wait_event(ctx) else { break };
//!     match event {
//!         ButtonRelease(_) => break,
//!         MotionNotify(m)  => { /* update geometry */ }
//!         _                => {}
//!     }
//! }
//! ungrab_ctx(ctx);
//! ```

use crate::backend::BackendEvent;
use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::contexts::WmCtxX11;
use crate::types::{AltCursor, MouseButton, Point};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Active (modal) pointer grab ───────────────────────────────────────────────

/// Grab the pointer for a modal drag/resize loop.
///
/// Returns `true` on success, `false` if the grab fails (e.g. another client
/// already holds the grab).
///
/// The grab captures `ButtonPress | ButtonRelease | PointerMotion` in async
/// mode on the root window with no event-window confinement.
///
/// After a successful grab, use [`wait_event`] to poll events inside the
/// loop and [`ungrab_ctx`] to release the grab when done.
pub fn grab_pointer(
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    cursor: AltCursor,
) -> bool {
    let cursor_index = cursor.to_x11_index();
    let xcursor = x11_runtime
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor as u32)
        .unwrap_or(x11rb::NONE);

    grab_pointer_impl(
        x11.conn,
        x11_runtime.root,
        xcursor,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
    )
}

/// Like [`grab_pointer`] but additionally listens for `KeyPress` events.
///
/// Used by [`crate::mouse::hover::run_x11_hover_resize_offer_loop`] so that pressing
/// Escape can abort the hover-resize wait before the user clicks.
pub fn grab_pointer_with_keys(
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    cursor: AltCursor,
) -> bool {
    let cursor_index = cursor.to_x11_index();
    let xcursor = x11_runtime
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor as u32)
        .unwrap_or(x11rb::NONE);

    grab_pointer_impl(
        x11.conn,
        x11_runtime.root,
        xcursor,
        EventMask::BUTTON_PRESS
            | EventMask::BUTTON_RELEASE
            | EventMask::POINTER_MOTION
            | EventMask::KEY_PRESS,
    )
}

fn grab_pointer_impl<C: Connection>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    cursor: u32,
    event_mask: EventMask,
) -> bool {
    conn.grab_pointer(
        false,
        root,
        event_mask,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        x11rb::NONE,
        cursor,
        CURRENT_TIME,
    )
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .map(|r| r.status == GrabStatus::SUCCESS)
    .unwrap_or(false)
}

/// Wait for the next X11 event.
///
/// Borrows the connection only for the duration of the call, so the caller
/// can freely mutate `ctx` between events.
pub fn wait_event(x11: &X11BackendRef) -> Option<x11rb::protocol::Event> {
    match x11.conn.wait_for_event() {
        Ok(event) => Some(event),
        Err(err) => {
            log::warn!("X11 wait_for_event error in drag loop: {}", err);
            None
        }
    }
}

/// Release an active pointer grab via context.
///
/// Always call this when a drag/resize loop ends, even on early returns,
/// to avoid leaving the pointer permanently grabbed.
#[inline]
pub fn ungrab(x11: &X11BackendRef) {
    let _ = ungrab_pointer(x11.conn, CURRENT_TIME);
    let _ = x11.conn.flush();
}

fn pump_deferred_work(ctx: &mut WmCtxX11<'_>) {
    if ctx.core.bar.needs_redraw() {
        crate::backend::x11::bar::draw_bars(&mut ctx.core, ctx.x11_runtime, ctx.systray.as_deref());
    }
}

/// Convert an X11 event to a backend-agnostic [`BackendEvent`].
fn event_to_backend(event: &x11rb::protocol::Event) -> Option<BackendEvent> {
    match event {
        x11rb::protocol::Event::MotionNotify(m) => Some(BackendEvent::Motion {
            root: Point::new(m.root_x as i32, m.root_y as i32),
            modifiers: u16::from(m.state) as u32,
        }),
        x11rb::protocol::Event::ButtonRelease(br) => MouseButton::from_x11_detail(br.detail)
            .map(|button| BackendEvent::ButtonRelease { button }),
        x11rb::protocol::Event::ButtonPress(bp) => MouseButton::from_x11_detail(bp.detail)
            .map(|button| BackendEvent::ButtonPress { button }),
        x11rb::protocol::Event::KeyPress(kp) => Some(BackendEvent::KeyPress {
            keycode: kp.detail as u32,
        }),
        _ => None,
    }
}

/// Call `on_event` with the given event converted to [`BackendEvent`].
///
/// Returns `true` (continue) when the event cannot be converted.
fn call_on_event<F>(
    on_event: &mut F,
    ctx: &mut WmCtxX11<'_>,
    event: &x11rb::protocol::Event,
) -> bool
where
    F: FnMut(&mut WmCtxX11<'_>, &BackendEvent) -> bool,
{
    if let Some(be) = event_to_backend(event) {
        on_event(ctx, &be)
    } else {
        true
    }
}

/// Generic X11 mouse-drag event loop.
///
/// Handles pointer grabbing, the motion-event loop (with throttling),
/// and final ungrabbing.
///
/// If `with_keys` is true, also captures KeyPress events.
/// The closure `on_event` returns `true` to continue the loop, `false` to break.
/// Events are converted to [`BackendEvent`] so callers are backend-agnostic.
pub fn mouse_drag_loop<F>(
    ctx: &mut WmCtxX11<'_>,
    btn: MouseButton,
    cursor: AltCursor,
    with_keys: bool,
    mut on_event: F,
) where
    F: FnMut(&mut WmCtxX11<'_>, &BackendEvent) -> bool,
{
    let grabbed = if with_keys {
        grab_pointer_with_keys(&ctx.x11, ctx.x11_runtime, cursor)
    } else {
        grab_pointer(&ctx.x11, ctx.x11_runtime, cursor)
    };

    if !grabbed {
        return;
    }

    pump_deferred_work(ctx);

    loop {
        // Wait for at least one event (blocking).
        let Some(mut event) = wait_event(&ctx.x11) else {
            break;
        };

        // If it's a motion event, compress it by eating all subsequent pending
        // motion events in the queue, keeping only the absolute latest.
        // This ensures zero-latency dragging without artificial 16ms FPS caps.
        if let x11rb::protocol::Event::MotionNotify(_) = event {
            loop {
                match ctx.x11.conn.poll_for_event() {
                    Ok(Some(next_evt)) => {
                        if let x11rb::protocol::Event::MotionNotify(_) = next_evt {
                            event = next_evt; // Discard older motion, keep newest.
                        } else {
                            // It's a different event (e.g. ButtonRelease). We must put it
                            // back so wait_event/poll_for_event yield it next time!
                            // x11rb doesn't let us un-read events easily, so we process
                            // the compressed motion *now*, then process this next_evt.
                            if !call_on_event(&mut on_event, ctx, &event) {
                                pump_deferred_work(ctx);
                                ungrab(&ctx.x11);
                                return;
                            }
                            pump_deferred_work(ctx);

                            // Now process the non-motion event we peeked.
                            if let x11rb::protocol::Event::ButtonRelease(br) = next_evt
                                && br.detail == btn.to_x11_detail()
                            {
                                pump_deferred_work(ctx);
                                ungrab(&ctx.x11);
                                return;
                            }
                            if !call_on_event(&mut on_event, ctx, &next_evt) {
                                pump_deferred_work(ctx);
                                ungrab(&ctx.x11);
                                return;
                            }
                            pump_deferred_work(ctx);

                            // We've processed the peeking; continue the main `wait_event` loop.
                            continue;
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        log::warn!("X11 poll_for_event error in drag loop: {}", err);
                        break;
                    }
                }
            }
        }

        let should_continue = match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.to_x11_detail() {
                    false
                } else {
                    call_on_event(&mut on_event, ctx, &event)
                }
            }
            _ => call_on_event(&mut on_event, ctx, &event),
        };

        pump_deferred_work(ctx);

        if !should_continue {
            break;
        }
    }

    pump_deferred_work(ctx);
    ungrab(&ctx.x11);
}

