//! X11 pointer-grab helpers.
//!
//! This module adapts X11 pointer grabs to the shared WM interaction transport.
//! Gesture recognition and behavior do not live here.
//!
//! Captured pointer input is intercepted by the normal event dispatcher. The
//! native grab outlives an early logical cancellation until the initiating
//! button is physically released, preventing half an input sequence from
//! leaking to a client.

use crate::backend::x11::{PointerGrabKind, X11BackendRef, X11RuntimeConfig};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::{AltCursor, ModMask, MouseButton, Point};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Active pointer grabs ─────────────────────────────────────────────────────

/// Grab the pointer for an event-loop-driven WM interaction.
///
/// Returns `true` on success, `false` if the grab fails (e.g. another client
/// already holds the grab).
///
/// The grab captures `ButtonPress | ButtonRelease | PointerMotion` in async
/// mode on the root window with no event-window confinement.
///
pub fn grab_pointer(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
    button: MouseButton,
) -> bool {
    let event_mask =
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
    grab_pointer_with_mask(
        x11,
        x11_runtime,
        cursor,
        event_mask,
        PointerGrabKind::Interaction(button),
    )
}

/// Lend the pointer to an armed hover-resize offer.
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

/// Release an active pointer grab and clear its runtime-owned cursor state.
#[inline]
pub fn ungrab(x11: &X11BackendRef, x11_runtime: &mut X11RuntimeConfig) {
    let _ = ungrab_pointer(x11.conn, CURRENT_TIME);
    let _ = x11.conn.flush();
    x11_runtime.active_pointer_grab = None;
}

fn owns_pointer_capture(ctx: &WmCtxX11<'_>, btn: MouseButton) -> bool {
    ctx.core.interaction().drag.captured_button() == Some(btn)
        && ctx.core.interaction().drag.captured_source()
            == Some(crate::types::InteractionSource::Pointer)
}

/// Begin native ownership for an interaction already captured by shared state.
/// Subsequent motion and release events are handled by
/// [`dispatch_captured_pointer_event`] in the normal X11 event loop.
pub fn begin_wm_interaction(ctx: &mut WmCtxX11<'_>, btn: MouseButton) -> bool {
    if !owns_pointer_capture(ctx, btn) {
        return false;
    }
    let cursor = ctx.core.interaction().drag.projection().cursor;
    if !grab_pointer(&ctx.x11, ctx.x11_runtime, cursor, btn) {
        let _ = crate::mouse::interaction::handle(
            &mut WmCtx::X11(ctx.reborrow()),
            crate::mouse::interaction::InteractionEvent::pointer_cancel(
                crate::core_state::DragCancelReason::InputCaptureLost,
            ),
        );
    }
    true
}

/// Consume pointer events owned by an active WM interaction grab.
///
/// Once shared state cancels, motion remains swallowed and the native grab is
/// retained as a release quarantine. Only the initiating physical release ends
/// native ownership.
pub fn dispatch_captured_pointer_event(
    ctx: &mut WmCtxX11<'_>,
    event: &x11rb::protocol::Event,
) -> bool {
    let Some(crate::backend::x11::ActivePointerGrab {
        kind: PointerGrabKind::Interaction(button),
        ..
    }) = ctx.x11_runtime.active_pointer_grab
    else {
        return false;
    };

    match event {
        x11rb::protocol::Event::MotionNotify(motion) => {
            if owns_pointer_capture(ctx, button) {
                let _ = crate::mouse::interaction::handle(
                    &mut WmCtx::X11(ctx.reborrow()),
                    crate::mouse::interaction::InteractionEvent::pointer_update(
                        Point::new(motion.root_x as i32, motion.root_y as i32),
                        ModMask::new(u16::from(motion.state)),
                    ),
                );
            }
            true
        }
        x11rb::protocol::Event::ButtonPress(_) => true,
        x11rb::protocol::Event::ButtonRelease(release) => {
            if release.detail != button.to_x11_detail() {
                return true;
            }

            if owns_pointer_capture(ctx, button) {
                let root = Point::new(release.root_x as i32, release.root_y as i32);
                let sidebar_hover =
                    crate::mouse::pointer::sidebar_target_at(ctx.core.model(), root);
                let _ = crate::mouse::interaction::handle(
                    &mut WmCtx::X11(ctx.reborrow()),
                    crate::mouse::interaction::InteractionEvent::pointer_end(
                        root,
                        button,
                        ModMask::new(u16::from(release.state)),
                        sidebar_hover,
                        release.time,
                    ),
                );
            }
            ungrab(&ctx.x11, ctx.x11_runtime);
            true
        }
        _ => false,
    }
}
