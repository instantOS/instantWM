//! X11-specific client visibility: mapping/unmapping windows and WM_STATE transitions.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::constants::{WM_STATE_ICONIC, WM_STATE_NORMAL};
use crate::backend::x11::properties::set_client_state;
use crate::constants::animation::DECORATIVE_SHOW_FRAME_COUNT;
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
    let g = ctx.core.state();
    let operations = crate::client::visibility::visibility_plan(&g.model);

    let has_tiling = g.monitors_iter().any(|(_, m)| m.is_tiling_layout());

    for entry in operations {
        let win = entry.win;
        let geo = entry.rect;
        let is_visible = entry.visible;
        let mode = entry.mode;
        crate::animation::drop_x11_animation(ctx.x11_runtime, win);

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
            let _ = ctx.x11.conn.flush();

            let should_position = mode.is_free_positioned()
                || mode.is_fake_fullscreen()
                || (mode.is_tiling() && !has_tiling);
            if should_position {
                let mut tmp_ctx = WmCtx::X11(ctx.reborrow());
                tmp_ctx.move_resize(
                    win,
                    Rect { x, y, w, h },
                    MoveResizeOptions::hinted_immediate(false),
                );
            }
        } else {
            let w_val = geo.w + 2 * entry.border_width;
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
            let _ = ctx.x11.conn.flush();
        }
    }
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
        MoveResizeOptions::animate_from(Rect { x, y: -50, w, h }, DECORATIVE_SHOW_FRAME_COUNT),
    );
}

// ---------------------------------------------------------------------------
// Hide (minimize)
// ---------------------------------------------------------------------------

pub fn hide(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let root = ctx.x11_runtime.root;
    let x11_win: Window = win.into();

    let _grab = crate::backend::x11::ServerGrab::new(ctx.x11.conn);
    suppress_unmap_events(ctx.x11.conn, root, x11_win);

    let _ = ctx.x11.conn.unmap_window(x11_win);
    let _ = ctx.x11.conn.flush();
    set_client_state(&ctx.x11, ctx.x11_runtime, win, WM_STATE_ICONIC);

    restore_event_masks(ctx.x11.conn, root, x11_win);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn suppress_unmap_events(conn: &x11rb::rust_connection::RustConnection, root: Window, win: Window) {
    if let Ok(cookie) = conn.get_window_attributes(root)
        && let Ok(ra) = cookie.reply()
    {
        let mask =
            EventMask::from(ra.your_event_mask.bits() & !EventMask::SUBSTRUCTURE_NOTIFY.bits());
        let _ =
            conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
    }

    if let Ok(cookie) = conn.get_window_attributes(win)
        && let Ok(ca) = cookie.reply()
    {
        let mask = EventMask::from(ca.your_event_mask.bits() & !EventMask::STRUCTURE_NOTIFY.bits());
        let _ =
            conn.change_window_attributes(win, &ChangeWindowAttributesAux::new().event_mask(mask));
    }
}

fn restore_event_masks(conn: &x11rb::rust_connection::RustConnection, root: Window, win: Window) {
    if let Ok(cookie) = conn.get_window_attributes(root)
        && let Ok(ra) = cookie.reply()
    {
        let _ = conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(ra.your_event_mask),
        );
    }

    if let Ok(cookie) = conn.get_window_attributes(win)
        && let Ok(ca) = cookie.reply()
    {
        let _ = conn.change_window_attributes(
            win,
            &ChangeWindowAttributesAux::new().event_mask(ca.your_event_mask),
        );
    }
}
