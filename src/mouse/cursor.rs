use crate::contexts::WmCtx;
use crate::types::AltCursor;

pub fn set_cursor_style(ctx: &mut WmCtx, style: AltCursor) {
    if ctx.core().globals().behavior.requested_cursor == style {
        return;
    }
    ctx.core_mut().globals_mut().behavior.requested_cursor = style;
    match ctx {
        WmCtx::X11(x11) => {
            crate::backend::x11::mouse::set_x11_root_cursor(
                &x11.x11,
                x11.x11_runtime,
                style,
            );
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
