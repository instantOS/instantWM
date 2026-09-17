//! X11 pointer-grab helpers.
//!
//! This module adapts X11's modal pointer grab to the shared WM interaction
//! transport. Gesture recognition and behavior do not live here.
//!
//! Captured pointer events are handled here; other events go through the same
//! protocol dispatcher as the main loop. A pointer grab must never suppress
//! window lifecycle or property updates.

use crate::backend::x11::{PointerGrabKind, X11BackendRef, X11RuntimeConfig};
use crate::contexts::{WmCtx, WmCtxX11};
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
/// loop and [`ungrab`] to release the grab when done.
pub fn grab_pointer(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
) -> bool {
    let event_mask =
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
    grab_pointer_with_mask(x11, x11_runtime, cursor, event_mask, PointerGrabKind::Drag)
}

/// Lend the pointer to an armed hover-resize offer, without a modal loop.
///
/// The passive offer model only owns the root window: over any other window
/// the client beneath owns the cursor and the button press. This grab carries
/// the offer's resize cursor across those windows and routes the committing
/// press to the root window, where the press policy picks it up. It is
/// released by [`ungrab`] as soon as the offer clears.
///
/// The grab always selects its own `PointerMotion`: raw XI2 motion cannot be
/// relied on while an active grab is held (servers stop delivering it), and
/// without motion the offer could never observe the pointer leaving the
/// border zone. Where raw events do keep flowing, the redundant delivery is
/// harmless — offer state updates are idempotent position checks.
pub fn grab_hover_offer_pointer(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
) -> bool {
    let event_mask =
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
    grab_pointer_with_mask(
        x11,
        x11_runtime,
        cursor,
        event_mask,
        PointerGrabKind::HoverOffer,
    )
}

fn grab_pointer_with_mask(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
    event_mask: EventMask,
    kind: PointerGrabKind,
) -> bool {
    let cursor_index = cursor.to_x11_index();
    let xcursor = x11_runtime
        .cursors
        .get(cursor_index)
        .and_then(|c| c.as_ref())
        .map(|c| c.cursor as u32)
        .unwrap_or(x11rb::NONE);

    let grabbed = grab_pointer_impl(x11.conn, x11_runtime.root, xcursor, event_mask);
    if grabbed {
        x11_runtime.active_pointer_grab = Some(crate::backend::x11::ActivePointerGrab {
            kind,
            event_mask,
            cursor,
        });
    }
    grabbed
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

/// Release an active pointer grab and clear its runtime-owned cursor state.
///
/// Always call this when a drag/resize loop ends, even on early returns,
/// to avoid leaving the pointer permanently grabbed.
#[inline]
pub fn ungrab(x11: &X11BackendRef, x11_runtime: &mut X11RuntimeConfig) {
    let _ = ungrab_pointer(x11.conn, CURRENT_TIME);
    let _ = x11.conn.flush();
    x11_runtime.active_pointer_grab = None;
}

fn pump_deferred_work(ctx: &mut WmCtxX11<'_>) {
    if ctx.core.bar.needs_redraw() {
        crate::backend::x11::bar::draw_bars(&mut ctx.core, ctx.x11_runtime);
    }
    // This modal loop bypasses the normal calloop tick, which ordinarily
    // flushes once after dispatch. Send the compressed motion batch now so
    // interactive geometry never sits in x11rb's write buffer until ungrab.
    let _ = ctx.x11.conn.flush();
}

/// Dispatch an event consumed by X11's modal pointer loop.
fn dispatch_grabbed_event(ctx: &mut WmCtxX11<'_>, event: &x11rb::protocol::Event) {
    // Only captured pointer input belongs to this adapter. All other protocol
    // events use the same dispatcher as the main loop, including future handlers.
    // In particular, never discard lifecycle, map, property or configure events.
    if let x11rb::protocol::Event::MotionNotify(motion) = event {
        let _ = crate::mouse::interaction::handle(
            &mut WmCtx::X11(ctx.reborrow()),
            crate::mouse::interaction::InteractionEvent::pointer_update(
                Point::new(motion.root_x as i32, motion.root_y as i32),
                u16::from(motion.state) as u32,
            ),
        );
    } else if !matches!(
        event,
        x11rb::protocol::Event::ButtonPress(_) | x11rb::protocol::Event::ButtonRelease(_)
    ) {
        // Additional button presses must not start a nested pointer interaction;
        // the matching release is handled by the loop itself.
        crate::backend::x11::events::loop_fn::dispatch_event_in_context(ctx, event);
    }
}

fn owns_pointer_capture(ctx: &WmCtxX11<'_>, btn: MouseButton) -> bool {
    ctx.core.interaction().drag.captured_button() == Some(btn)
        && ctx.core.interaction().drag.captured_source()
            == Some(crate::types::InteractionSource::Pointer)
}

/// A transport-level `Cancel` event for the pointer source.
fn cancel_event(
    reason: crate::core_state::DragCancelReason,
) -> crate::mouse::interaction::InteractionEvent {
    crate::mouse::interaction::InteractionEvent {
        source: crate::types::InteractionSource::Pointer,
        phase: crate::mouse::interaction::InteractionPhase::Cancel { reason },
        root: Default::default(),
        modifiers: 0,
        sidebar_hover: None,
    }
}

/// Re-check that the modal loop still owns a drivable capture.
///
/// Events dispatched inside the loop can end the interaction themselves (a
/// lifecycle event cancelling the capture, a quit binding) or invalidate it:
/// a keybind that switches tags hides the dragged window while it stays
/// managed, and the loop must cancel instead of steering an invisible window.
fn capture_remains_drivable(ctx: &mut WmCtxX11<'_>, btn: MouseButton) -> bool {
    if !owns_pointer_capture(ctx, btn) || !ctx.core.is_running() {
        return false;
    }
    if !crate::mouse::drag::window_drag_target_visible(&WmCtx::X11(ctx.reborrow())) {
        let _ = crate::mouse::interaction::handle(
            &mut WmCtx::X11(ctx.reborrow()),
            cancel_event(crate::core_state::DragCancelReason::WindowHidden),
        );
        return false;
    }
    true
}

/// Generic X11 mouse-drag event loop.
///
/// Handles pointer grabbing, the motion-event loop (with throttling),
/// and final ungrabbing.
///
/// Returns the root coordinates and modifier mask from the matching button
/// release. Both values are already present in the event, so callers never
/// need a synchronous `QueryPointer` round trip to finish an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X11DragRelease {
    pub root: Point,
    pub modifiers: u32,
    pub time_msec: u32,
}

fn run_interaction_grab_loop(
    ctx: &mut WmCtxX11<'_>,
    btn: MouseButton,
    cursor: AltCursor,
) -> Option<X11DragRelease> {
    if !grab_pointer(&ctx.x11, ctx.x11_runtime, cursor) {
        return None;
    }

    pump_deferred_work(ctx);

    let mut release = None;
    // Wait for at least one event (blocking) each iteration.
    'events: while let Some(mut event) = wait_event(&ctx.x11) {
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
                            dispatch_grabbed_event(ctx, &event);
                            pump_deferred_work(ctx);

                            // Now process the non-motion event we peeked.
                            if let x11rb::protocol::Event::ButtonRelease(br) = next_evt
                                && br.detail == btn.to_x11_detail()
                            {
                                pump_deferred_work(ctx);
                                ungrab(&ctx.x11, ctx.x11_runtime);
                                return Some(X11DragRelease {
                                    root: Point::new(br.root_x as i32, br.root_y as i32),
                                    modifiers: u16::from(br.state) as u32,
                                    time_msec: br.time,
                                });
                            }
                            dispatch_grabbed_event(ctx, &next_evt);
                            pump_deferred_work(ctx);

                            // A lifecycle event or action may have cancelled
                            // capture or hidden the dragged window. Do not
                            // block waiting for a release after its target
                            // has disappeared.
                            if !capture_remains_drivable(ctx, btn) {
                                break 'events;
                            }
                            // Do not apply the compressed motion twice.
                            continue 'events;
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
            x11rb::protocol::Event::ButtonRelease(br) if br.detail == btn.to_x11_detail() => {
                release = Some(X11DragRelease {
                    root: Point::new(br.root_x as i32, br.root_y as i32),
                    modifiers: u16::from(br.state) as u32,
                    time_msec: br.time,
                });
                false
            }
            _ => {
                dispatch_grabbed_event(ctx, &event);
                true
            }
        };

        pump_deferred_work(ctx);

        if !should_continue || !capture_remains_drivable(ctx, btn) {
            break;
        }
    }

    pump_deferred_work(ctx);
    ungrab(&ctx.x11, ctx.x11_runtime);
    release
}

/// Drive any compositor-owned interaction through the shared transport.
///
/// Drive a captured shared interaction through X11's modal pointer grab.
/// Gesture semantics remain in `mouse::interaction`, alongside Wayland
/// pointer and touch handling.
pub fn drive_wm_interaction(ctx: &mut WmCtxX11<'_>, btn: MouseButton) -> bool {
    if !owns_pointer_capture(ctx, btn) {
        return false;
    }
    let cursor = ctx.core.interaction().drag.projection().cursor;

    let release = run_interaction_grab_loop(ctx, btn, cursor);

    let Some(release) = release else {
        let _ = crate::mouse::interaction::handle(
            &mut crate::contexts::WmCtx::X11(ctx.reborrow()),
            cancel_event(crate::core_state::DragCancelReason::InputCaptureLost),
        );
        return true;
    };

    let sidebar_hover = crate::mouse::pointer::sidebar_target_at(ctx.core.model(), release.root);
    let _ = crate::mouse::interaction::handle(
        &mut crate::contexts::WmCtx::X11(ctx.reborrow()),
        crate::mouse::interaction::InteractionEvent::pointer_end(
            release.root,
            btn,
            release.modifiers,
            sidebar_hover,
            release.time_msec,
        ),
    );
    true
}
