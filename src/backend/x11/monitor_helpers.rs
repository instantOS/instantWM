//! X11-specific monitor helpers: Xinerama, bar destruction, stacking.

use crate::backend::{BackendOutputInfo, BackendVrrSupport};
use crate::contexts::WmCtx;
use crate::types::{Rect, WindowId};
use x11rb::protocol::xinerama;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

/// Destroy an X11 bar window for a monitor.
pub fn destroy_monitor_bar(ctx: &mut WmCtx, bar_win: WindowId) {
    if bar_win != WindowId::default()
        && let WmCtx::X11(x11) = ctx
    {
        let x11_bar_win: Window = bar_win.into();
        let _ = unmap_window(x11.x11.conn, x11_bar_win);
        let _ = destroy_window(x11.x11.conn, x11_bar_win);
    }
}

/// Unique Xinerama screens as outputs; empty when Xinerama is inactive.
pub fn xinerama_outputs(conn: &RustConnection) -> Vec<BackendOutputInfo> {
    let active = xinerama::is_active(conn)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.state != 0);
    let Some(screens) = active
        .then(|| xinerama::query_screens(conn).ok()?.reply().ok())
        .flatten()
    else {
        return Vec::new();
    };

    let mut unique: Vec<Rect> = Vec::new();
    for s in &screens.screen_info {
        let rect = Rect::new(
            i32::from(s.x_org),
            i32::from(s.y_org),
            i32::from(s.width),
            i32::from(s.height),
        );
        if !unique.contains(&rect) {
            unique.push(rect);
        }
    }

    unique
        .into_iter()
        .enumerate()
        .map(|(i, rect)| BackendOutputInfo {
            name: format!("XINERAMA-{i}"),
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
            mirrors: Vec::new(),
        })
        .collect()
}
