//! Mouse gesture operations.
//!
//! This module handles root-window gestures like vertical swipes and the
//! bottom-bar horizontal swipe.

use crate::actions::execute_button_action;
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
        .transition_pointer_interaction(|drag| {
            drag.begin_sidebar_volume(crate::core_state::SidebarVolumeDrag::new(
                btn,
                source,
                target.monitor_id,
                start.y,
                threshold,
            ))
        })
        .is_err()
    {
        return false;
    }
    true
}

pub fn update_sidebar_gesture(ctx: &mut WmCtx, root_y: i32) {
    let Some(monitor_id) = ctx.core().interaction().drag.sidebar_volume_monitor() else {
        return;
    };
    if ctx.core().model().monitor(monitor_id).is_none() {
        ctx.transition_pointer_interaction(|drag| drag.cancel_sidebar_volume());
        return;
    }

    let steps = ctx
        .transition_pointer_interaction(|drag| drag.update_sidebar_volume(root_y))
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
        let _ = crate::util::spawn(ctx, command);
    }
}

pub fn sidebar_gesture_finish(
    ctx: &mut WmCtx,
    btn: MouseButton,
    hover_target: Option<SidebarTarget>,
) -> bool {
    if ctx.core().interaction().drag.sidebar_volume_button() != Some(btn) {
        return false;
    }
    ctx.transition_pointer_interaction(|drag| drag.finish_sidebar_volume(btn));
    let _ = crate::mouse::set_sidebar_offer(ctx, hover_target);
    true
}

/// Bottom-bar swipe gesture recogniser.
///
/// Begins a swipe on the bottom gesture strip. Once the cursor travels more
/// than `monitor_width / 30` pixels from the press position, the swipe
/// direction (left, right, or up) is latched; releasing the button then runs
/// the matching bound action exactly once (adjacent-tag switching left/right,
/// overview toggle up by default). The drag may leave the strip — motion keeps
/// being delivered to the captured gesture — so a press-hold-slide-release
/// that leaves the bar still triggers exactly one action, no matter how far the
/// drag goes.
pub fn bottom_bar_gesture_begin(
    ctx: &mut WmCtx,
    btn: MouseButton,
    source: InteractionSource,
    monitor_id: MonitorId,
    start: Point,
    press_time_msec: u32,
    actions: crate::core_state::BottomBarActions,
) -> bool {
    let threshold = ctx
        .core()
        .model()
        .monitor(monitor_id)
        .map(|monitor| (monitor.monitor_rect.w / 30).max(1))
        .unwrap_or(1);
    if ctx
        .transition_pointer_interaction(|drag| {
            drag.begin_bottom_bar(crate::core_state::BottomBarDrag::new(
                btn,
                source,
                monitor_id,
                start,
                threshold,
                press_time_msec,
                actions,
            ))
        })
        .is_err()
    {
        return false;
    }
    true
}

/// Minimum press duration (in milliseconds) for a no-swipe release to count as
/// a hold rather than a click.
const BOTTOM_BAR_HOLD_MS: u32 = 400;

pub fn update_bottom_bar_gesture(ctx: &mut WmCtx, root: Point) {
    let Some(monitor_id) = ctx.core().interaction().drag.bottom_bar_monitor() else {
        return;
    };
    if ctx.core().model().monitor(monitor_id).is_none() {
        ctx.transition_pointer_interaction(|drag| drag.cancel_bottom_bar());
        return;
    }
    if let Some(direction) = ctx.transition_pointer_interaction(|drag| drag.update_bottom_bar(root))
    {
        let _ = direction;
    }
}

pub fn bottom_bar_gesture_finish(
    ctx: &mut WmCtx,
    btn: MouseButton,
    root: Point,
    time_msec: u32,
) -> bool {
    if ctx.core().interaction().drag.bottom_bar_button() != Some(btn) {
        return false;
    }
    let (source, action) = {
        let Some(drag) = ctx.core().interaction().drag.bottom_bar_drag() else {
            return false;
        };
        let action = match drag.latched_direction() {
            Some(crate::core_state::SwipeDirection::Left) => Some(drag.left().clone()),
            Some(crate::core_state::SwipeDirection::Right) => Some(drag.right().clone()),
            Some(crate::core_state::SwipeDirection::Up) => Some(drag.up().clone()),
            None => {
                // No swipe: distinguish click (short press) from hold (long press).
                let held = time_msec.wrapping_sub(drag.press_time_msec());
                if held >= BOTTOM_BAR_HOLD_MS {
                    Some(drag.hold().clone())
                } else {
                    Some(drag.click().clone())
                }
            }
        };
        (drag.source(), action)
    };
    ctx.transition_pointer_interaction(|drag| drag.finish_bottom_bar(btn));
    if let Some(action) = action {
        let arg = crate::types::ButtonArg {
            target: crate::types::ButtonTarget::BottomBar,
            window: None,
            btn,
            source,
            root,
            time_msec,
        };
        execute_button_action(ctx, &action, arg);
    }
    true
}
