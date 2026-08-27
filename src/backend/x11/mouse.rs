//! X11 mouse backend helpers.

use crate::backend::x11::{PointerGrabKind, X11BackendRef, X11RuntimeConfig};
use crate::contexts::{CoreCtx, WmCtxX11};
use crate::types::{AltCursor, Point, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorUpdateTargets {
    root: bool,
    active_grab: bool,
}

fn cursor_update_targets(
    last_root_cursor: Option<AltCursor>,
    active_grab: Option<crate::backend::x11::ActivePointerGrab>,
    requested: AltCursor,
) -> CursorUpdateTargets {
    CursorUpdateTargets {
        root: last_root_cursor != Some(requested),
        active_grab: active_grab.is_some_and(|grab| grab.cursor != requested),
    }
}

/// Project the requested cursor to the root window and any active pointer grab.
///
/// Active grabs own their cursor independently of the root window, so both
/// projections must remain synchronized with the shared requested style.
pub fn set_x11_cursor(
    x11: &X11BackendRef<'_>,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
) {
    let targets = cursor_update_targets(
        x11_runtime.last_x11_cursor,
        x11_runtime.active_pointer_grab,
        cursor,
    );
    if !targets.root && !targets.active_grab {
        return;
    }
    let conn = x11.conn;
    let root = x11_runtime.root;
    let cursor_index = cursor.to_x11_index();
    if let Some(Some(loaded_cursor)) = x11_runtime.cursors.get(cursor_index) {
        if targets.root {
            let _ = xproto::change_window_attributes(
                conn,
                root,
                &xproto::ChangeWindowAttributesAux::new().cursor(loaded_cursor.cursor as u32),
            );
            x11_runtime.last_x11_cursor = Some(cursor);
        }
        if let Some(grab) = x11_runtime.active_pointer_grab.as_mut()
            && targets.active_grab
        {
            let _ = conn.change_active_pointer_grab(
                loaded_cursor.cursor as u32,
                x11rb::CURRENT_TIME,
                grab.event_mask,
            );
            grab.cursor = cursor;
        }
        let _ = conn.flush();
    }
}

impl crate::backend::CursorOps for WmCtxX11<'_> {
    fn apply_cursor_style(&mut self, style: AltCursor) {
        set_x11_cursor(&self.x11, self.x11_runtime, style);
    }
}

/// Grant the active hover-resize offer pointer ownership, or take it back.
///
/// X11 alone needs this: its passive offer can only project the root cursor
/// and observe presses on the root window, so a floating window's border zone
/// overlapping another client loses both the cursor feedback and the
/// committing click to the client beneath. A non-modal grab — the pointer
/// counterpart of the modal loop this backend used to run — restores both for
/// exactly as long as the offer is armed.
pub fn set_hover_pointer_grab(ctx: &mut WmCtxX11<'_>, active: bool) {
    if active {
        arm_hover_pointer_grab(ctx);
    } else {
        release_hover_pointer_grab(ctx);
    }
}

fn arm_hover_pointer_grab(ctx: &mut WmCtxX11<'_>) {
    // A modal drag already owns the pointer; never clobber it.
    if ctx.x11_runtime.active_pointer_grab.is_some() || hover_grab_blocked(&ctx.core) {
        return;
    }
    let Some((_, dir)) = ctx.core.interaction().drag.hover_offer.resize_target() else {
        return;
    };
    if crate::backend::x11::grab::grab_hover_offer_pointer(
        &ctx.x11,
        ctx.x11_runtime,
        AltCursor::Resize(dir),
    ) {
        let _ = ctx.x11.conn.flush();
    }
}

fn release_hover_pointer_grab(ctx: &mut WmCtxX11<'_>) {
    let held_by_offer = ctx
        .x11_runtime
        .active_pointer_grab
        .is_some_and(|grab| grab.kind == PointerGrabKind::HoverOffer);
    if held_by_offer {
        crate::backend::x11::grab::ungrab(&ctx.x11, ctx.x11_runtime);
    }
}

/// Scenes that own pointer semantics in their own right must keep the offer
/// from borrowing the pointer.
fn hover_grab_blocked(core: &CoreCtx<'_>) -> bool {
    core.model().is_overview_active()
        || core.behavior().current_mode.tree_placement().is_some()
        || core.interaction().drag.capture().is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerSnapshot {
    pub root: Point,
    pub child: Option<WindowId>,
}

/// Read position and root child together. QueryPointer already returns both;
/// keeping them together prevents a second round trip in hover handling.
pub fn pointer_snapshot(
    globals: &crate::core_state::CoreState,
    conn: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
) -> Option<PointerSnapshot> {
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;
    Some(PointerSnapshot {
        root: Point::new(reply.root_x as i32, reply.root_y as i32),
        child: managed_window(globals, reply.child),
    })
}

pub fn managed_window(
    globals: &crate::core_state::CoreState,
    window: x11rb::protocol::xproto::Window,
) -> Option<WindowId> {
    if window == x11rb::NONE {
        return None;
    }
    let window = WindowId::from(window);
    globals.model.client(window).is_some().then_some(window)
}

#[cfg(test)]
mod cursor_tests {
    use super::{CursorUpdateTargets, cursor_update_targets};
    use crate::backend::x11::{ActivePointerGrab, PointerGrabKind};
    use crate::types::AltCursor;
    use x11rb::protocol::xproto::EventMask;

    fn active(cursor: AltCursor) -> ActivePointerGrab {
        ActivePointerGrab {
            kind: PointerGrabKind::Drag,
            event_mask: EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            cursor,
        }
    }

    #[test]
    fn active_grab_cursor_updates_even_when_root_already_matches() {
        assert_eq!(
            cursor_update_targets(
                Some(AltCursor::Move),
                Some(active(AltCursor::Default)),
                AltCursor::Move,
            ),
            CursorUpdateTargets {
                root: false,
                active_grab: true,
            }
        );
    }

    #[test]
    fn matching_root_and_grab_suppress_redundant_native_updates() {
        assert_eq!(
            cursor_update_targets(
                Some(AltCursor::Move),
                Some(active(AltCursor::Move)),
                AltCursor::Move,
            ),
            CursorUpdateTargets {
                root: false,
                active_grab: false,
            }
        );
    }
}
