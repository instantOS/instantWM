//! X11 mouse backend helpers.

use crate::backend::x11::{PointerGrabKind, X11BackendRef, X11RuntimeConfig};
use crate::contexts::WmCtxX11;
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

impl crate::backend::InteractionProjectionOps for WmCtxX11<'_> {
    fn reconcile_interaction_projection(
        &mut self,
        desired: crate::core_state::InteractionPresentation,
    ) {
        set_x11_cursor(&self.x11, self.x11_runtime, desired.cursor);
        match hover_ownership_update(
            self.x11_runtime.active_pointer_grab.map(|grab| grab.kind),
            desired.pointer_routing,
        ) {
            HoverOwnershipUpdate::Acquire => {
                ensure_hover_offer_pointer_ownership(self, desired.cursor)
            }
            HoverOwnershipUpdate::Release => release_hover_offer_pointer_ownership(self),
            HoverOwnershipUpdate::Keep => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoverOwnershipUpdate {
    Acquire,
    Release,
    Keep,
}

fn hover_ownership_update(
    active: Option<PointerGrabKind>,
    desired: crate::core_state::PointerRouting,
) -> HoverOwnershipUpdate {
    match (active, desired) {
        (None, crate::core_state::PointerRouting::HoverOffer) => HoverOwnershipUpdate::Acquire,
        (Some(PointerGrabKind::HoverOffer), routing)
            if routing != crate::core_state::PointerRouting::HoverOffer =>
        {
            HoverOwnershipUpdate::Release
        }
        _ => HoverOwnershipUpdate::Keep,
    }
}

/// Ensure X11 satisfies the shared hover-offer routing guarantee.
///
/// A passive offer only owns the root window. When its border zone overlaps a
/// client, the client would otherwise own both cursor feedback and the press.
/// The native grab is an X11 projection detail and is never exposed to shared
/// interaction policy.
fn ensure_hover_offer_pointer_ownership(ctx: &mut WmCtxX11<'_>, cursor: AltCursor) {
    // A modal drag owns its native transport separately. Never replace it.
    if ctx.x11_runtime.active_pointer_grab.is_some() {
        return;
    }
    if crate::backend::x11::grab::grab_hover_offer_pointer(&ctx.x11, ctx.x11_runtime, cursor) {
        let _ = ctx.x11.conn.flush();
    }
}

fn release_hover_offer_pointer_ownership(ctx: &mut WmCtxX11<'_>) {
    let held_by_offer = ctx
        .x11_runtime
        .active_pointer_grab
        .is_some_and(|grab| grab.kind == PointerGrabKind::HoverOffer);
    if held_by_offer {
        crate::backend::x11::grab::ungrab(&ctx.x11, ctx.x11_runtime);
    }
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
    use super::{
        CursorUpdateTargets, HoverOwnershipUpdate, cursor_update_targets, hover_ownership_update,
    };
    use crate::backend::x11::{ActivePointerGrab, PointerGrabKind};
    use crate::core_state::PointerRouting;
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

    #[test]
    fn hover_ownership_reconciliation_is_level_triggered() {
        assert_eq!(
            hover_ownership_update(None, PointerRouting::HoverOffer),
            HoverOwnershipUpdate::Acquire
        );
        assert_eq!(
            hover_ownership_update(
                Some(PointerGrabKind::HoverOffer),
                PointerRouting::HoverOffer
            ),
            HoverOwnershipUpdate::Keep
        );
        assert_eq!(
            hover_ownership_update(Some(PointerGrabKind::HoverOffer), PointerRouting::Normal),
            HoverOwnershipUpdate::Release
        );
        assert_eq!(
            hover_ownership_update(
                Some(PointerGrabKind::Drag),
                PointerRouting::CapturedInteraction,
            ),
            HoverOwnershipUpdate::Keep
        );
    }
}
