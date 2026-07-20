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
    let threshold = ctx
        .core()
        .model()
        .monitor(target.monitor_id)
        .map(|monitor| (monitor.monitor_rect.h / 30).max(1))
        .unwrap_or(1);
    if ctx
        .core_mut()
        .drag_state_mut()
        .begin_sidebar_volume(crate::core_state::SidebarVolumeDrag::new(
            btn,
            target.monitor_id,
            ptr.y,
            threshold,
        ))
        .is_err()
    {
        return;
    }
    crate::mouse::set_cursor_style(ctx, AltCursor::Move);
}

pub fn update_sidebar_gesture(ctx: &mut WmCtx, root_y: i32) {
    let Some(monitor_id) = ctx.core().drag_state().sidebar_volume_monitor() else {
        return;
    };
    if ctx.core().model().monitor(monitor_id).is_none() {
        ctx.core_mut().drag_state_mut().cancel_sidebar_volume();
        crate::mouse::set_cursor_style(ctx, AltCursor::Default);
        return;
    }

    let steps = ctx
        .core_mut()
        .drag_state_mut()
        .update_sidebar_volume(root_y)
        .unwrap_or(0);
    if steps == 0 {
        return;
    }

    let command = if steps > 0 {
        ctx.core()
            .config()
            .external_commands
            .get(crate::config::commands::Cmd::UpVol)
    } else {
        ctx.core()
            .config()
            .external_commands
            .get(crate::config::commands::Cmd::DownVol)
    };
    for _ in 0..steps.unsigned_abs() {
        crate::util::spawn(ctx, command);
    }
}

pub fn finish_sidebar_gesture(ctx: &mut WmCtx, btn: MouseButton) -> bool {
    if ctx.core().drag_state().sidebar_volume_button() != Some(btn) {
        return false;
    }
    ctx.core_mut().drag_state_mut().finish_sidebar_volume(btn);
    crate::mouse::set_cursor_style(ctx, AltCursor::Default);
    true
}

pub fn sidebar_gesture_x11(ctx: &mut WmCtxX11, btn: MouseButton) {
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        begin_sidebar_gesture(&mut wm_ctx, btn);
        if !wm_ctx.core().drag_state().sidebar_volume_active() {
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
