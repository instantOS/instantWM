//! X11 client backend helpers.

use crate::backend::x11::X11BackendRef;
use crate::client::constants::{
    SIZE_HINTS_P_ASPECT, SIZE_HINTS_P_BASE_SIZE, SIZE_HINTS_P_MAX_SIZE, SIZE_HINTS_P_MIN_SIZE,
    SIZE_HINTS_P_RESIZE_INC,
};
use crate::contexts::CoreCtx;
use crate::types::WindowId;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::ConnectionExt;

/// Read `WM_NORMAL_HINTS` from the X server and populate the client's size hints,
/// `min_aspect`, `max_aspect`, and `isfixed`.
pub fn update_size_hints_x11(core: &mut CoreCtx, x11: &X11BackendRef, win: WindowId) {
    let Some(data) = fetch_wm_normal_hints(x11, win) else {
        return;
    };
    let Some(c) = core.g.clients.get_mut(&win) else {
        return;
    };
    let flags = *data.first().unwrap_or(&0);
    let at = |idx: usize| -> i32 { data.get(idx).copied().unwrap_or(0) as i32 };

    // Base size (idx 15-16), fallback to min size (idx 5-6)
    (c.size_hints.basew, c.size_hints.baseh) =
        if flags & SIZE_HINTS_P_BASE_SIZE != 0 && data.len() > 16 {
            (at(15), at(16))
        } else if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() > 6 {
            (at(5), at(6))
        } else {
            (0, 0)
        };

    // Min size (idx 5-6), fallback to base size
    (c.size_hints.minw, c.size_hints.minh) = if flags & SIZE_HINTS_P_MIN_SIZE != 0 && data.len() > 6
    {
        (at(5), at(6))
    } else if flags & SIZE_HINTS_P_BASE_SIZE != 0 {
        (c.size_hints.basew, c.size_hints.baseh)
    } else {
        (0, 0)
    };

    // Max size (idx 7-8)
    (c.size_hints.maxw, c.size_hints.maxh) = if flags & SIZE_HINTS_P_MAX_SIZE != 0 && data.len() > 8
    {
        (at(7), at(8))
    } else {
        (0, 0)
    };

    // Resize increments (idx 9-10)
    (c.size_hints.incw, c.size_hints.inch) =
        if flags & SIZE_HINTS_P_RESIZE_INC != 0 && data.len() > 10 {
            (at(9), at(10))
        } else {
            (0, 0)
        };

    // Aspect ratios (idx 11-14)
    (c.min_aspect, c.max_aspect) = if flags & SIZE_HINTS_P_ASPECT != 0 && data.len() > 14 {
        let min_d = at(12);
        let max_d = at(14);
        (
            if min_d != 0 {
                at(11) as f32 / min_d as f32
            } else {
                0.0
            },
            if max_d != 0 {
                at(13) as f32 / max_d as f32
            } else {
                0.0
            },
        )
    } else {
        (0.0, 0.0)
    };

    c.is_fixed_size = c.size_hints.is_fixed();
    c.size_hints_valid = 1;
}

fn fetch_wm_normal_hints(x11: &X11BackendRef, win: WindowId) -> Option<Vec<u32>> {
    let conn = x11.conn;
    let reply = conn
        .get_property(
            false,
            win.into(),
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            0,
            24,
        )
        .ok()?
        .reply()
        .ok()?;
    reply.value32().map(|v| v.collect())
}
