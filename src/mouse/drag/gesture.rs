//! Mouse gesture operations.
//!
//! This module handles root-window gestures like vertical swipes and the
//! bottom-bar horizontal swipe.

use crate::actions::execute_button_action;
use crate::contexts::WmCtx;
use crate::core_state::{BottomBarDrag, SidebarVolumeDrag};
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
            drag.begin(crate::core_state::SidebarVolumeDrag::new(
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
    let Some(monitor_id) = ctx
        .core()
        .interaction()
        .drag
        .captured::<SidebarVolumeDrag>()
        .map(|drag| drag.monitor_id)
    else {
        return;
    };
    if ctx.core().model().monitor(monitor_id).is_none() {
        ctx.transition_pointer_interaction(|drag| drag.cancel::<SidebarVolumeDrag>());
        return;
    }

    let steps = ctx
        .transition_pointer_interaction(|drag| {
            drag.captured_mut::<SidebarVolumeDrag>()
                .map(|drag| drag.update(root_y))
        })
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
    if ctx
        .transition_pointer_interaction(|drag| drag.finish::<SidebarVolumeDrag>(btn))
        .is_none()
    {
        return false;
    }
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
            drag.begin(BottomBarDrag::new(
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
    let Some(monitor_id) = ctx
        .core()
        .interaction()
        .drag
        .captured::<BottomBarDrag>()
        .map(|drag| drag.monitor_id)
    else {
        return;
    };
    if ctx.core().model().monitor(monitor_id).is_none() {
        ctx.transition_pointer_interaction(|drag| drag.cancel::<BottomBarDrag>());
        return;
    }
    ctx.transition_pointer_interaction(|drag| {
        drag.captured_mut::<BottomBarDrag>()
            .and_then(|drag| drag.update(root))
    });
}

pub fn bottom_bar_gesture_finish(
    ctx: &mut WmCtx,
    btn: MouseButton,
    root: Point,
    time_msec: u32,
) -> bool {
    let Some(drag) = ctx.transition_pointer_interaction(|drag| drag.finish::<BottomBarDrag>(btn))
    else {
        return false;
    };
    let actions = drag.actions;
    let action = match drag.latched_direction() {
        Some(crate::core_state::SwipeDirection::Left) => actions.left,
        Some(crate::core_state::SwipeDirection::Right) => actions.right,
        Some(crate::core_state::SwipeDirection::Up) => actions.up,
        // No swipe: distinguish click (short press) from hold (long press).
        None if time_msec.wrapping_sub(drag.press_time_msec) >= BOTTOM_BAR_HOLD_MS => {
            actions.hold
        }
        None => actions.click,
    };
    let arg = crate::types::ButtonArg {
        target: crate::types::ButtonTarget::BottomBar,
        window: None,
        btn,
        source: drag.source,
        root,
        time_msec,
    };
    execute_button_action(ctx, &action, arg);
    true
}
