//! Mouse gesture operations.
//!
//! This module handles root-window gestures like vertical swipes.

use crate::backend::BackendEvent;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::*;

/// Sidebar vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn sidebar_gesture_begin(ctx: &mut WmCtx, btn: MouseButton) {
    match ctx {
        WmCtx::X11(x11) => sidebar_gesture_x11(x11, btn),
        WmCtx::Wayland(_) => begin_sidebar_gesture(ctx, btn),
    }
}

pub fn begin_sidebar_gesture(ctx: &mut WmCtx, btn: MouseButton) {
    let Some(ptr) = ctx.pointer_backend().pointer_location() else {
        return;
    };
    let Some(target) = crate::mouse::pointer::sidebar_target_at(ctx.core().model(), ptr) else {
        return;
    };
    ctx.core_mut().drag_state_mut().gesture = crate::core_state::GestureInteraction {
        active: true,
        button: btn,
        monitor_id: target.monitor_id,
        last_y: ptr.y,
    };
    crate::mouse::set_cursor_style(ctx, AltCursor::Move);
}

pub fn update_sidebar_gesture(ctx: &mut WmCtx, root_y: i32) {
    let (monitor_id, last_y) = {
        let gesture = &ctx.core().drag_state().gesture;
        if !gesture.active {
            return;
        }
        (gesture.monitor_id, gesture.last_y)
    };
    let threshold = ctx
        .core()
        .state()
        .monitor(monitor_id)
        .map(|mon| (mon.monitor_rect.h / 30).max(1))
        .unwrap_or(1);

    if (last_y - root_y).abs() <= threshold {
        return;
    }

    let cmd = if root_y < last_y {
        &["ins", "assist", "volume", "+"][..]
    } else {
        &["ins", "assist", "volume", "-"][..]
    };
    crate::util::spawn(ctx, cmd);
    ctx.core_mut().drag_state_mut().gesture.last_y = root_y;
}

pub fn finish_sidebar_gesture(ctx: &mut WmCtx, btn: MouseButton) -> bool {
    let active = {
        let gesture = &ctx.core().drag_state().gesture;
        gesture.active && gesture.button == btn
    };
    if !active {
        return false;
    }
    ctx.core_mut().drag_state_mut().gesture = crate::core_state::GestureInteraction::default();
    crate::mouse::set_cursor_style(ctx, AltCursor::Default);
    true
}

pub fn sidebar_gesture_x11(ctx: &mut WmCtxX11, btn: MouseButton) {
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        begin_sidebar_gesture(&mut wm_ctx, btn);
        if !wm_ctx.core().drag_state().gesture.active {
            return;
        }
    }

    crate::backend::x11::grab::mouse_drag_loop(ctx, btn, AltCursor::Move, false, |ctx, event| {
        if let BackendEvent::Motion { root, .. } = event {
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            update_sidebar_gesture(&mut wm_ctx, root.y);
        }
        true
    });

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    let _ = finish_sidebar_gesture(&mut wm_ctx, btn);
}
