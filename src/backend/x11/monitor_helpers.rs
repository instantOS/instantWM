//! X11-specific monitor helpers: Xinerama, bar destruction, stacking.

use crate::backend::{BackendOutputInfo, BackendVrrSupport};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::{Rect, WindowId};
use x11rb::protocol::xinerama;
use x11rb::protocol::xproto::*;

/// Destroy an X11 bar window for a monitor.
pub fn destroy_monitor_bar_x11(ctx: &mut WmCtx, bar_win: WindowId) {
    if bar_win != WindowId::default()
        && let WmCtx::X11(x11) = ctx
    {
        let x11_bar_win: Window = bar_win.into();
        let _ = unmap_window(x11.x11.conn, x11_bar_win);
        let _ = destroy_window(x11.x11.conn, x11_bar_win);
    }
}

/// Raise a client window above siblings on X11.
pub fn raise_client_window_x11(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let x11_win: Window = win.into();
    let _ = configure_window(
        ctx.x11.conn,
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
}

/// Query Xinerama screen information and return outputs (no monitor sync).
pub fn xinerama_outputs(x11: &mut WmCtxX11) -> Option<Vec<BackendOutputInfo>> {
    let conn = x11.x11.conn;
    let is_active = xinerama::is_active(conn).ok()?.reply().ok()?;
    if is_active.state == 0 {
        return None;
    }

    let screens = xinerama::query_screens(conn).ok()?.reply().ok()?;
    let mut unique = Vec::new();
    for s in &screens.screen_info {
        let info = Rect {
            x: s.x_org as i32,
            y: s.y_org as i32,
            w: s.width as i32,
            h: s.height as i32,
        };
        if !unique
            .iter()
            .any(|u: &Rect| u.x == info.x && u.y == info.y && u.w == info.w && u.h == info.h)
        {
            unique.push(info);
        }
    }

    let outputs: Vec<BackendOutputInfo> = unique
        .into_iter()
        .enumerate()
        .map(|(i, rect)| BackendOutputInfo {
            name: format!("XINERAMA-{i}"),
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        })
        .collect();

    Some(outputs)
}
