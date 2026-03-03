use crate::backend::BackendRef;
use crate::contexts::WmCtx;
use crate::types::ResizeDirection;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{change_window_attributes, ChangeWindowAttributesAux};

fn set_x11_root_cursor(ctx: &WmCtx, cursor_index: usize) {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };
    if let Some(ref cursor) = ctx.g.cfg.cursors[cursor_index] {
        let _ = change_window_attributes(
            conn,
            ctx.g.cfg.root,
            &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
        );
        let _ = conn.flush();
    }
}

pub fn set_cursor_default(ctx: &mut WmCtx) {
    match &ctx.backend {
        BackendRef::X11(_) => set_x11_root_cursor(ctx, 0),
        BackendRef::Wayland(wayland) => wayland.set_cursor_icon_override(None),
    }
}

pub fn set_cursor_move(ctx: &mut WmCtx) {
    match &ctx.backend {
        BackendRef::X11(_) => set_x11_root_cursor(ctx, 2),
        BackendRef::Wayland(wayland) => {
            wayland.set_cursor_icon_override(Some(smithay::input::pointer::CursorIcon::Grabbing));
        }
    }
}

pub fn set_cursor_resize(ctx: &mut WmCtx, dir: Option<ResizeDirection>) {
    match &ctx.backend {
        BackendRef::X11(_) => {
            let idx = dir.map(ResizeDirection::cursor_index).unwrap_or(1);
            set_x11_root_cursor(ctx, idx);
        }
        BackendRef::Wayland(wayland) => {
            let icon = match dir {
                Some(ResizeDirection::Left) | Some(ResizeDirection::Right) => {
                    smithay::input::pointer::CursorIcon::EwResize
                }
                Some(ResizeDirection::Top) | Some(ResizeDirection::Bottom) => {
                    smithay::input::pointer::CursorIcon::NsResize
                }
                _ => smithay::input::pointer::CursorIcon::NwseResize,
            };
            wayland.set_cursor_icon_override(Some(icon));
        }
    }
}
