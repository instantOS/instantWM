//! X11-specific client visibility: mapping/unmapping windows and WM_STATE transitions.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::backend::x11::properties::set_client_state;
use crate::constants::animation::DECORATIVE_SHOW_ANIMATION_MILLIS;
use crate::contexts::WmCtxX11;
use crate::geometry::MoveResizeOptions;
use crate::types::{Rect, WindowId};
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// WM_STATE query
// ---------------------------------------------------------------------------

/// Read the `WM_STATE` property for `win` from the X server.
///
/// Returns one of the `WM_STATE_*` constants.  Falls back to
/// [`WM_STATE_NORMAL`] when the property is absent or unreadable.
pub fn get_state(x11: &X11BackendRef, wm_state_atom: u32, win: WindowId) -> i32 {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    let Ok(cookie) = conn.get_property(false, x11_win, wm_state_atom, wm_state_atom, 0, 2) else {
        return WM_STATE_NORMAL;
    };

    let Ok(reply) = cookie.reply() else {
        return WM_STATE_NORMAL;
    };

    reply
        .value32()
        .and_then(|mut it| it.next())
        .map(|v| v as i32)
        .unwrap_or(WM_STATE_NORMAL)
}

// ---------------------------------------------------------------------------
// Visibility apply
// ---------------------------------------------------------------------------

pub fn apply_visibility(ctx: &mut WmCtxX11<'_>) {
    let state = &ctx.core.state;
    let operations =
        visibility_transaction_order(crate::client::visibility::visibility_plan(&state.model));

    // A tag switch is one visual transaction. Position every incoming window
    // before parking any outgoing window, and keep other X clients (notably a
    // compositing manager) from observing the intermediate requests. Direct
    // per-window flushes are avoided; any same-connection round trips needed
    // for floating size hints remain hidden by the grab. ServerGrab flushes
    // the release when the complete visibility pass is done.
    let conn = ctx.x11.conn;
    let _grab = crate::backend::x11::ServerGrab::new(conn);

    for entry in operations {
        let win = entry.win;
        let geo = entry.rect;
        let is_visible = entry.visible;
        let _ = ctx.x11_runtime.take_window_animation(win);

        if is_visible {
            let Rect { x, y, w, h } = geo;
            let x11_win: Window = win.into();
            let width = w.max(1) as u32;
            let height = h.max(1) as u32;
            let _ = ctx.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(width)
                    .height(height),
            );

            if entry.position_on_reveal {
                let mut tmp_ctx = ctx.wm_ctx();
                tmp_ctx.move_resize(
                    win,
                    Rect { x, y, w, h },
                    MoveResizeOptions::hinted_immediate(false),
                );
            }
        } else {
            let w_val = geo.total_width(entry.border_width);
            let y = geo.y;

            let x11_win: Window = win.into();
            let _ = ctx.x11.conn.configure_window(
                x11_win,
                &ConfigureWindowAux::new()
                    .x(-2 * w_val)
                    .y(y)
                    .width(geo.w as u32)
                    .height(geo.h as u32),
            );
        }
    }
}

/// Stable transaction order for visibility changes.
///
/// Incoming windows must cover their destination before outgoing windows are
/// parked off-screen; otherwise the root wallpaper can become the only visible
/// content between two ConfigureWindow requests.
fn visibility_transaction_order(
    mut operations: Vec<crate::client::visibility::VisibilityEntry>,
) -> Vec<crate::client::visibility::VisibilityEntry> {
    operations.sort_by_key(|entry| !entry.visible);
    operations
}

// ---------------------------------------------------------------------------
// Show (unminimize)
// ---------------------------------------------------------------------------

pub fn show(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let Rect { x, y, w, h } = match ctx.core.state.model.client(win) {
        Some(c) => c.geo,
        None => return,
    };

    let x11_win: Window = win.into();
    let _ = ctx.x11.conn.map_window(x11_win);

    set_client_state(&ctx.x11, ctx.x11_runtime, win, WM_STATE_NORMAL);

    let _ = ctx.x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );

    ctx.wm_ctx().move_resize(
        win,
        Rect { x, y, w, h },
        MoveResizeOptions::animate_from(Rect { x, y: -50, w, h }, DECORATIVE_SHOW_ANIMATION_MILLIS),
    );
}

// ---------------------------------------------------------------------------
// Hide (minimize)
// ---------------------------------------------------------------------------

pub fn hide(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let root = ctx.x11_runtime.root;
    let x11_win: Window = win.into();

    let _grab = crate::backend::x11::ServerGrab::new(ctx.x11.conn);
    let _event_suppression = ctx
        .x11_runtime
        .subscriptions
        .suppress_unmap(ctx.x11.conn, root, x11_win)
        .expect("minimizing requires owned root and client event subscriptions");

    let _ = ctx.x11.conn.unmap_window(x11_win);
    set_client_state(&ctx.x11, ctx.x11_runtime, win, WM_STATE_ICONIC);
}

#[cfg(test)]
mod tests {
    use super::visibility_transaction_order;
    use crate::client::visibility::VisibilityEntry;
    use crate::types::{Rect, WindowId};

    #[test]
    #[ignore = "requires a dedicated Xvfb display"]
    fn minimize_suppresses_our_notifications_and_restores_owned_masks() {
        use super::*;
        use x11rb::connection::Connection;
        use x11rb::protocol::Event;

        let (conn, screen) = x11rb::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id().unwrap();
        conn.create_window(
            0,
            win,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap()
        .check()
        .unwrap();
        let mut runtime = crate::backend::x11::X11RuntimeConfig::default();
        let root_mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE;
        let window_mask =
            EventMask::STRUCTURE_NOTIFY | EventMask::FOCUS_CHANGE | EventMask::PROPERTY_CHANGE;
        runtime
            .subscriptions
            .select(&conn, root, root_mask)
            .unwrap();
        assert!(
            runtime
                .subscriptions
                .suppress_unmap(&conn, root, win)
                .is_err()
        );
        // Resolving a missing client must not temporarily alter the root.
        assert_eq!(
            conn.get_window_attributes(root)
                .unwrap()
                .reply()
                .unwrap()
                .your_event_mask,
            root_mask
        );
        runtime
            .subscriptions
            .select(&conn, win, window_mask)
            .unwrap();
        conn.map_window(win).unwrap().check().unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();
        while conn.poll_for_event().unwrap().is_some() {}

        let (observer, _) = x11rb::connect(None).unwrap();
        observer
            .change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
            )
            .unwrap()
            .check()
            .unwrap();
        {
            let _grab = crate::backend::x11::ServerGrab::new(&conn);
            let _suppression = runtime
                .subscriptions
                .suppress_unmap(&conn, root, win)
                .unwrap();
            conn.unmap_window(win).unwrap();
        }
        assert_eq!(
            conn.get_window_attributes(root)
                .unwrap()
                .reply()
                .unwrap()
                .your_event_mask,
            root_mask
        );
        assert_eq!(
            conn.get_window_attributes(win)
                .unwrap()
                .reply()
                .unwrap()
                .your_event_mask,
            window_mask
        );
        while let Some(event) = conn.poll_for_event().unwrap() {
            assert!(!matches!(event, Event::UnmapNotify(_)));
        }
        observer.get_input_focus().unwrap().reply().unwrap();
        assert!(matches!(
            observer.poll_for_event().unwrap(),
            Some(Event::UnmapNotify(_))
        ));

        // Subscriptions still deliver later application-driven unmaps.
        conn.map_window(win).unwrap().check().unwrap();
        while conn.poll_for_event().unwrap().is_some() {}
        conn.unmap_window(win).unwrap().check().unwrap();
        assert!(matches!(
            conn.poll_for_event().unwrap(),
            Some(Event::UnmapNotify(_))
        ));
        runtime.subscriptions.release(&conn, win, false);
        assert!(
            runtime
                .subscriptions
                .suppress_unmap(&conn, root, win)
                .is_err()
        );
        runtime
            .subscriptions
            .select(&conn, win, EventMask::NO_EVENT)
            .unwrap();
        {
            let _grab = crate::backend::x11::ServerGrab::new(&conn);
            let _suppression = runtime
                .subscriptions
                .suppress_unmap(&conn, root, win)
                .unwrap();
        }
        assert_eq!(
            conn.get_window_attributes(win)
                .unwrap()
                .reply()
                .unwrap()
                .your_event_mask,
            EventMask::NO_EVENT
        );
    }

    fn entry(win: u32, visible: bool) -> VisibilityEntry {
        VisibilityEntry {
            win: WindowId(win),
            rect: Rect::new(0, 0, 100, 100),
            border_width: 1,
            position_on_reveal: false,
            visible,
        }
    }

    #[test]
    fn visibility_transaction_positions_incoming_windows_before_parking_outgoing_ones() {
        let ordered = visibility_transaction_order(vec![
            entry(1, false),
            entry(2, true),
            entry(3, false),
            entry(4, true),
        ]);

        assert_eq!(
            ordered.iter().map(|entry| entry.win).collect::<Vec<_>>(),
            vec![WindowId(2), WindowId(4), WindowId(1), WindowId(3)]
        );
    }
}
