use crate::contexts::{WmCtx, WmCtxWayland, WmCtxX11};
use crate::types::ResizeDirection;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{change_window_attributes, ChangeWindowAttributesAux};

fn set_x11_root_cursor(ctx: &mut WmCtxX11<'_>, cursor_index: usize) {
    let conn = ctx.x11.conn;
    let root = ctx.core.g.x11.root;
    if let Some(ref cursor) = ctx.core.g.cfg.cursors[cursor_index] {
        let _ = change_window_attributes(
            conn,
            root,
            &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
        );
        let _ = conn.flush();
    }
}

pub fn set_cursor_default_x11(ctx: &mut WmCtxX11<'_>) {
    set_x11_root_cursor(ctx, 0);
}

pub fn set_cursor_default_wayland(ctx: &mut WmCtxWayland<'_>) {
    ctx.wayland.backend.set_cursor_icon_override(None);
}

pub fn set_cursor_default(ctx: &mut WmCtx) {
    match ctx {
        WmCtx::X11(x11) => set_cursor_default_x11(x11),
        WmCtx::Wayland(wayland) => set_cursor_default_wayland(wayland),
    }
}

pub fn set_cursor_move_x11(ctx: &mut WmCtxX11<'_>) {
    set_x11_root_cursor(ctx, 2);
}

pub fn set_cursor_move_wayland(ctx: &mut WmCtxWayland<'_>) {
    ctx.wayland
        .backend
        .set_cursor_icon_override(Some(smithay::input::pointer::CursorIcon::Grabbing));
}

pub fn set_cursor_move(ctx: &mut WmCtx) {
    match ctx {
        WmCtx::X11(x11) => set_cursor_move_x11(x11),
        WmCtx::Wayland(wayland) => set_cursor_move_wayland(wayland),
    }
}

pub fn set_cursor_resize_x11(ctx: &mut WmCtxX11<'_>, dir: Option<ResizeDirection>) {
    let idx = dir.map(ResizeDirection::cursor_index).unwrap_or(1);
    set_x11_root_cursor(ctx, idx);
}

pub fn set_cursor_resize_wayland(ctx: &mut WmCtxWayland<'_>, dir: Option<ResizeDirection>) {
    let icon = match dir {
        Some(ResizeDirection::Left) | Some(ResizeDirection::Right) => {
            smithay::input::pointer::CursorIcon::EwResize
        }
        Some(ResizeDirection::Top) | Some(ResizeDirection::Bottom) => {
            smithay::input::pointer::CursorIcon::NsResize
        }
        Some(ResizeDirection::TopRight) | Some(ResizeDirection::BottomLeft) => {
            smithay::input::pointer::CursorIcon::NeswResize
        }
        _ => smithay::input::pointer::CursorIcon::NwseResize,
    };
    ctx.wayland.backend.set_cursor_icon_override(Some(icon));
}

pub fn set_cursor_resize(ctx: &mut WmCtx, dir: Option<ResizeDirection>) {
    match ctx {
        WmCtx::X11(x11) => set_cursor_resize_x11(x11, dir),
        WmCtx::Wayland(wayland) => set_cursor_resize_wayland(wayland, dir),
    }
}
