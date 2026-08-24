//! X11-specific client visibility: mapping/unmapping windows and WM_STATE transitions.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::backend::x11::properties::set_client_state;
use crate::constants::animation::DECORATIVE_SHOW_ANIMATION_MILLIS;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;
use crate::types::{Rect, WindowId};
use x11rb::connection::Connection;
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
    let state = ctx.core.state();
    let operations =
        visibility_transaction_order(crate::client::visibility::visibility_plan(&state.model));
    let has_tiling = state
        .model
        .monitors_iter()
        .any(|(_, m)| m.is_tiling_layout());

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
        let mode = entry.mode;
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

            let should_position = mode.is_free_positioned()
                || mode.is_fake_fullscreen()
                || (mode.is_normal_tiling() && !has_tiling);
            if should_position {
                let mut tmp_ctx = WmCtx::X11(ctx.reborrow());
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
    let Rect { x, y, w, h } = match ctx.core.model().client(win) {
        Some(c) => c.geo,
        None => return,
    };

    let x11_win: Window = win.into();
    let _ = ctx.x11.conn.map_window(x11_win);
    let _ = ctx.x11.conn.flush();

    set_client_state(&ctx.x11, ctx.x11_runtime, win, WM_STATE_NORMAL);

    let _ = ctx.x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = ctx.x11.conn.flush();

    WmCtx::X11(ctx.reborrow()).move_resize(
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
    let _event_suppression = UnmapEventSuppression::new(ctx.x11.conn, root, x11_win);

    let _ = ctx.x11.conn.unmap_window(x11_win);
    let _ = ctx.x11.conn.flush();
    set_client_state(&ctx.x11, ctx.x11_runtime, win, WM_STATE_ICONIC);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

struct UnmapEventSuppression<'a> {
    conn: &'a x11rb::rust_connection::RustConnection,
    root: Window,
    win: Window,
    root_mask: Option<EventMask>,
    window_mask: Option<EventMask>,
}

impl<'a> UnmapEventSuppression<'a> {
    fn new(conn: &'a x11rb::rust_connection::RustConnection, root: Window, win: Window) -> Self {
        let root_mask = conn
            .get_window_attributes(root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|attrs| attrs.your_event_mask);
        let window_mask = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|attrs| attrs.your_event_mask);

        if let Some(mask) = root_mask {
            let suppressed = EventMask::from(mask.bits() & !EventMask::SUBSTRUCTURE_NOTIFY.bits());
            let _ = conn.change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().event_mask(suppressed),
            );
        }
        if let Some(mask) = window_mask {
            let suppressed = EventMask::from(mask.bits() & !EventMask::STRUCTURE_NOTIFY.bits());
            let _ = conn.change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().event_mask(suppressed),
            );
        }

        Self {
            conn,
            root,
            win,
            root_mask,
            window_mask,
        }
    }
}

impl Drop for UnmapEventSuppression<'_> {
    fn drop(&mut self) {
        if let Some(mask) = self.root_mask {
            let _ = self.conn.change_window_attributes(
                self.root,
                &ChangeWindowAttributesAux::new().event_mask(mask),
            );
        }
        if let Some(mask) = self.window_mask {
            let _ = self.conn.change_window_attributes(
                self.win,
                &ChangeWindowAttributesAux::new().event_mask(mask),
            );
        }
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::visibility_transaction_order;
    use crate::client::visibility::VisibilityEntry;
    use crate::types::{ClientMode, Rect, WindowId};

    fn entry(win: u32, visible: bool) -> VisibilityEntry {
        VisibilityEntry {
            win: WindowId(win),
            rect: Rect::new(0, 0, 100, 100),
            border_width: 1,
            mode: ClientMode::tiled(),
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
