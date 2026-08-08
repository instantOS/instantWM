//! Mouse gesture operations.
//!
//! This module handles root-window gestures like vertical swipes.

use crate::backend::{BackendEvent, PointerOps};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::*;

/// Sidebar vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn sidebar_gesture_begin(
    ctx: &mut WmCtx,
    btn: MouseButton,
    target: SidebarTarget,
    start: Point,
) -> bool {
    match ctx {
        WmCtx::X11(x11) => sidebar_gesture_x11(x11, btn, target, start),
        WmCtx::Wayland(_) => begin_sidebar_gesture(ctx, btn, target, start),
    }
}

fn begin_sidebar_gesture(
    ctx: &mut WmCtx,
    btn: MouseButton,
    target: SidebarTarget,
    start: Point,
) -> bool {
    let threshold = ctx
        .core()
        .model()
        .monitor(target.monitor_id)
        .map(|monitor| (monitor.monitor_rect.h / 30).max(1))
        .unwrap_or_else(|| (target.rect.h / 30).max(1));
    if ctx
        .core_mut()
        .drag_state_mut()
        .begin_sidebar_volume(crate::core_state::SidebarVolumeDrag::new(
            btn,
            target.monitor_id,
            start.y,
            threshold,
        ))
        .is_err()
    {
        return false;
    }
    crate::mouse::clear_hover_offer(ctx);
    ctx.set_cursor_style(AltCursor::VerticalAdjust);
    true
}

pub fn update_sidebar_gesture(ctx: &mut WmCtx, root_y: i32) {
    let Some(monitor_id) = ctx.core().drag_state().sidebar_volume_monitor() else {
        return;
    };
    if ctx.core().model().monitor(monitor_id).is_none() {
        ctx.core_mut().drag_state_mut().cancel_sidebar_volume();
        ctx.set_cursor_style(AltCursor::Default);
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

pub fn finish_sidebar_gesture(
    ctx: &mut WmCtx,
    btn: MouseButton,
    hover_target: Option<SidebarTarget>,
) -> bool {
    if ctx.core().drag_state().sidebar_volume_button() != Some(btn) {
        return false;
    }
    ctx.core_mut().drag_state_mut().finish_sidebar_volume(btn);
    ctx.set_cursor_style(AltCursor::Default);
    let _ = crate::mouse::set_sidebar_offer(ctx, hover_target);
    true
}

fn sidebar_gesture_x11(
    ctx: &mut WmCtxX11,
    btn: MouseButton,
    target: SidebarTarget,
    start: Point,
) -> bool {
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        if !begin_sidebar_gesture(&mut wm_ctx, btn, target, start) {
            return false;
        }
    }

    crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::VerticalAdjust,
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, .. } = event {
                let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                update_sidebar_gesture(&mut wm_ctx, root.y);
            }
            true
        },
    );

    let root = ctx.x11.pointer_location().unwrap_or(start);
    let window_at_root = crate::backend::x11::mouse::cursor_client_win(
        ctx.core.state,
        ctx.x11.conn,
        ctx.x11_runtime.root,
    );
    let hover_target =
        crate::mouse::pointer::desktop_sidebar_target_at(ctx.core.model(), root, window_at_root);
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    let _ = finish_sidebar_gesture(&mut wm_ctx, btn, hover_target);
    true
}
