//! Client geometry: resizing, size-hint enforcement, and dimension helpers.
//!
//! # Responsibilities
//!
//! * [`resize`] – high-level resize that runs size-hint validation first.
//! * [`resize_client`] – low-level X11 configure + state update.
//! * [`apply_size_hints`] – clamp a proposed geometry to ICCCM `WM_NORMAL_HINTS`.
//! * [`update_size_hints`] / [`update_size_hints_win`] – read `WM_NORMAL_HINTS` from X.
//! * [`scale_client`] – resize a client to a percentage of its monitor.
//!
//! # Dimension helpers
//!
//! Client dimensions including borders are available as methods:
//! * [`Client::total_width`](crate::types::Client::total_width) – total width including borders
//! * [`Client::total_height`](crate::types::Client::total_height) – total height including borders

use crate::backend::BackendOps;
use crate::backend::x11::apply_size_hints_x11;
use crate::contexts::{CoreCtx, X11Ctx};
use crate::types::{Rect, WindowId};

use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, Window};

// ---------------------------------------------------------------------------
// High-level resize (validates size hints)
// ---------------------------------------------------------------------------

/// Resize `win` to the given `rect`, enforcing `WM_NORMAL_HINTS` constraints.
///
/// If the size-hint check determines that nothing changed *and* there is more
/// than one client on screen, the X11 configure call is skipped.  With a
/// single client we always apply the resize so the window fills its space.
pub fn resize_x11(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, rect: &Rect, interact: bool) {
    if !core.g.clients.contains(&win) {
        return;
    }

    let mut new_rect = *rect;
    let changed = apply_size_hints_x11(core, x11, win, &mut new_rect, interact);
    let client_count = core.g.clients.len();
    if changed || client_count == 1 {
        resize_client_x11(core, x11, win, &new_rect);
    }
}

/// Backend-agnostic resize entry point.
///
pub fn resize(ctx: &mut crate::contexts::WmCtx<'_>, win: WindowId, rect: &Rect, interact: bool) {
    match ctx {
        crate::contexts::WmCtx::X11(ref mut x11_ctx) => {
            resize_x11(&mut x11_ctx.core, &x11_ctx.x11, win, rect, interact)
        }
        crate::contexts::WmCtx::Wayland(ref mut wl_ctx) => {
            let _ = interact;
            if let Some(c) = wl_ctx.core.g.clients.get_mut(&win) {
                c.old_geo = c.geo;
                c.geo = *rect;
                if c.isfloating {
                    c.float_geo = *rect;
                }
                wl_ctx.backend.resize_window(win, *rect);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Low-level resize (direct X11 configure)
// ---------------------------------------------------------------------------

/// Update the stored geometry for `win` and issue an X11 `ConfigureWindow`.
///
/// This is the single source of truth for moving/resizing a window at the X11
/// level.  Always call [`resize`] from layout code so that size hints are
/// respected; call this directly only when you have already validated the
/// geometry (e.g. during fullscreen transitions).
pub fn resize_client_x11(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, rect: &Rect) {
    core.g.clients.update_geometry(win, *rect);

    let x11_win: Window = win.into();
    let width = rect.w.max(1) as u32;
    let height = rect.h.max(1) as u32;
    let _ = x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new()
            .x(rect.x)
            .y(rect.y)
            .width(width)
            .height(height),
    );

    if let Some(border_width) = core.g.clients.get(&win).map(|c| c.border_width) {
        let _ = x11.conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().border_width(border_width as u32),
        );
    }

    // Send a synthetic ConfigureNotify so the client knows its geometry.
    crate::client::focus::configure_x11(core, x11, win);
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Calculate the target rect for scaling a client to `scale` percent of its monitor.
fn calculate_scaled_geometry(
    monitor_id: Option<crate::types::MonitorId>,
    old_geo: Rect,
    border_width: i32,
    scale: i32,
    get_monitor_rect: impl FnOnce(Option<crate::types::MonitorId>) -> Rect,
) -> Rect {
    let mon_rect = get_monitor_rect(monitor_id).unwrap_or(old_geo);

    let new_w = old_geo.w * scale / 100;
    let new_h = old_geo.h * scale / 100;
    let new_x = mon_rect.x + (mon_rect.w - new_w) / 2 - border_width;
    let new_y = mon_rect.y + (mon_rect.h - new_h) / 2 - border_width;

    Rect {
        x: new_x,
        y: new_y,
        w: new_w,
        h: new_h,
    }
}

/// Resize `win` to `scale` percent of its monitor dimensions, centred on screen.
///
/// `scale` is an integer percentage (e.g. `75` means 75 %).
pub fn scale_client(ctx: &mut crate::contexts::WmCtx<'_>, win: WindowId, scale: i32) {
    let target = match ctx {
        crate::contexts::WmCtx::X11(ref mut x11_ctx) => {
            let c = match x11_ctx.core.g.clients.get(&win) {
                Some(c) => c,
                None => return,
            };
            calculate_scaled_geometry(
                c.monitor_id,
                c.geo,
                c.border_width,
                scale,
                |mid| {
                    mid.and_then(|m| x11_ctx.core.g.monitors.get(m))
                        .map(|m| m.monitor_rect)
                        .unwrap_or(c.geo)
                },
            )
        }
        crate::contexts::WmCtx::Wayland(ref mut wl_ctx) => {
            let c = match wl_ctx.core.g.clients.get(&win) {
                Some(c) => c,
                None => return,
            };
            calculate_scaled_geometry(
                c.monitor_id,
                c.geo,
                c.border_width,
                scale,
                |mid| {
                    mid.and_then(|m| wl_ctx.core.g.monitors.get(m))
                        .map(|m| m.monitor_rect)
                        .unwrap_or(c.geo)
                },
            )
        }
    };

    resize(ctx, win, &target, false);
}
