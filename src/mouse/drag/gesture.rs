//! Mouse gesture operations.
//!
//! This module handles root-window gestures like vertical swipes.

use crate::contexts::{WmCtx, WmCtxX11};
use crate::mouse::warp::get_root_ptr;
use crate::types::*;

/// Root-window vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn gesture_mouse(ctx: &mut WmCtx, btn: MouseButton) {
    if let WmCtx::X11(x11) = ctx {
        gesture_mouse_x11(x11, btn);
    }
}

pub fn gesture_mouse_x11(ctx: &mut WmCtxX11, btn: MouseButton) {
    let wm_ctx = WmCtx::X11(ctx.reborrow());
    let Some((_, start_y)) = get_root_ptr(&wm_ctx) else {
        return;
    };

    let mut last_y = start_y;

    crate::backend::x11::grab::mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            let threshold = ctx.core.globals().selected_monitor().monitor_rect.h / 30;
            if (last_y - m.event_y as i32).abs() > threshold {
                let event_y = m.event_y as i32;
                let cmd = if event_y < last_y {
                    &["/usr/share/instantassist/utils/p.sh", "+"]
                } else {
                    &["/usr/share/instantassist/utils/p.sh", "-"]
                };
                let wm_ctx = WmCtx::X11(ctx.reborrow());
                crate::util::spawn(&wm_ctx, cmd);
                last_y = event_y;
            }
        }
        true
    });
}
