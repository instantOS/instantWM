//! Mouse gesture operations.
//!
//! This module handles root-window gestures like vertical swipes.

use crate::contexts::WmCtx;
use crate::types::*;

/// Sidebar vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn sidebar_gesture_begin(
    ctx: &mut WmCtx,
    btn: MouseButton,
    source: InteractionSource,
    target: SidebarTarget,
    start: Point,
) -> bool {
    begin_sidebar_gesture(ctx, btn, source, target, start)
}

fn begin_sidebar_gesture(
    ctx: &mut WmCtx,
    btn: MouseButton,
    source: InteractionSource,
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
            source,
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
