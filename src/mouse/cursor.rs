use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::AltCursor;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{change_window_attributes, ChangeWindowAttributesAux};

fn set_x11_root_cursor_by_index(ctx: &mut WmCtxX11<'_>, cursor_index: usize) {
    if ctx.x11_runtime.last_x11_cursor_index == Some(cursor_index) {
        return;
    }
    let conn = ctx.x11.conn;
    let root = ctx.x11_runtime.root;
    if let Some(ref loaded_cursor) = ctx.x11_runtime.cursors[cursor_index] {
        let _ = change_window_attributes(
            conn,
            root,
            &ChangeWindowAttributesAux::new().cursor(loaded_cursor.cursor as u32),
        );
        let _ = conn.flush();
        ctx.x11_runtime.last_x11_cursor_index = Some(cursor_index);
    }
}

pub fn set_cursor_style(ctx: &mut WmCtx, style: AltCursor) {
    ctx.g_mut().behavior.cursor_icon = style;
    match ctx {
        WmCtx::X11(x11) => {
            set_x11_root_cursor_by_index(x11, style.to_x11_index());
        }
        WmCtx::Wayland(wayland) => {
            let icon = match style {
                AltCursor::Default => None,
                AltCursor::Move => Some(smithay::input::pointer::CursorIcon::Grabbing),
                AltCursor::Resize(dir) => Some(dir.to_wayland_icon()),
            };
            wayland.wayland.backend.set_cursor_icon_override(icon);
        }
    }
}
