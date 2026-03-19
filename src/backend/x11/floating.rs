//! X11 floating window helpers.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::types::WindowId;

pub fn apply_floating_borderscheme(
    x11: &X11BackendRef,
    win: WindowId,
    x11_runtime: &X11RuntimeConfig,
) {
    let pixel = x11_runtime.borderscheme.float_focus.bg.color.pixel;
    let _ = x11rb::protocol::xproto::change_window_attributes(
        x11.conn,
        win.into(),
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().border_pixel(Some(pixel as u32)),
    );
}
