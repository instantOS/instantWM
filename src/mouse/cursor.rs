use crate::contexts::{WmCtx, WmCtxWayland, WmCtxX11};
use crate::types::input::Cursor;
use crate::types::ResizeDirection;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{change_window_attributes, ChangeWindowAttributesAux};

fn set_x11_root_cursor(ctx: &mut WmCtxX11<'_>, cursor: Cursor) {
    let cursor_index = cursor.to_x11_index();
    if ctx.core.g.drag.last_x11_cursor_index == Some(cursor_index) {
        return;
    }
    let conn = ctx.x11.conn;
    let root = ctx.x11_runtime.root;
    if let Some(ref loaded_cursor) = ctx.core.g.cfg.cursors[cursor_index] {
        let _ = change_window_attributes(
            conn,
            root,
            &ChangeWindowAttributesAux::new().cursor(loaded_cursor.cursor as u32),
        );
        let _ = conn.flush();
        ctx.core.g.drag.last_x11_cursor_index = Some(cursor_index);
    }
}

pub fn set_cursor_default_x11(ctx: &mut WmCtxX11<'_>) {
    set_x11_root_cursor(ctx, Cursor::Normal);
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
    set_x11_root_cursor(ctx, Cursor::Move);
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
    let cursor = match dir {
        Some(ResizeDirection::Left | ResizeDirection::Right) => Cursor::Hor,
        Some(ResizeDirection::Top | ResizeDirection::Bottom) => Cursor::Vert,
        Some(ResizeDirection::TopRight | ResizeDirection::BottomLeft) => Cursor::TR,
        Some(ResizeDirection::TopLeft | ResizeDirection::BottomRight) => Cursor::TL,
        None => Cursor::Resize,
    };
    set_x11_root_cursor(ctx, cursor);
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
