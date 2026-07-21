//! Click and drag interactions for tag indicators in the bar.

use crate::backend::BackendEvent;
use crate::config::{CONTROL, MOD1};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::mouse::constants::DRAG_THRESHOLD;
use crate::mouse::cursor::set_cursor_style;
use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TagDropBehavior {
    Move,
    MoveAndFollow,
}

impl TagDropBehavior {
    fn from_modifiers(modifiers: u32) -> Self {
        if modifiers & MOD1 != 0 {
            Self::MoveAndFollow
        } else {
            Self::Move
        }
    }
}

/// Apply the common window-on-tag drop contract.
///
/// A plain drop moves the window without disrupting the current view. Holding
/// Alt at release moves the window and follows it to the destination tag.
pub(crate) fn apply_window_tag_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    tag_mask: TagMask,
    modifiers: u32,
) {
    match TagDropBehavior::from_modifiers(modifiers) {
        TagDropBehavior::Move => crate::tags::client_tags::set_client_tag(ctx, win, tag_mask),
        TagDropBehavior::MoveAndFollow => crate::tags::client_tags::follow_tag(ctx, win, tag_mask),
    }
}

fn selected_on_monitor(ctx: &WmCtx<'_>, monitor_id: MonitorId) -> Option<WindowId> {
    ctx.core()
        .state()
        .model
        .monitors
        .get(monitor_id)
        .and_then(|monitor| monitor.selected)
}

fn position_at(ctx: &WmCtx<'_>, monitor_id: MonitorId, root: Point) -> Option<BarPosition> {
    let core = ctx.core();
    let monitor = core.state().model.monitors.get(monitor_id)?;
    let mask = monitor.selected_tags();
    if !monitor.show_bar_for_mask(mask)
        || !monitor.y_in_bar(root.y)
        || root.x < monitor.monitor_rect.x
        || root.x >= monitor.monitor_rect.x + monitor.monitor_rect.w
    {
        return None;
    }
    Some(monitor.bar_position_at_x(core, root.x - monitor.work_rect().x))
}

/// Arm a tag click. Motion beyond [`DRAG_THRESHOLD`] promotes it to a drag.
/// Keeping clicks armed until release makes modifier handling identical on X11
/// and Wayland, and avoids a visual flash when the pointer merely clicks a tag.
pub fn drag_tag_begin(
    ctx: &mut WmCtx,
    bar_pos: BarPosition,
    btn: MouseButton,
    start: Point,
) -> bool {
    let BarPosition::Tag(tag_idx) = bar_pos else {
        return false;
    };
    let Some(initial_tag) = TagMask::from_index(tag_idx) else {
        return false;
    };
    let monitor_id = ctx.core().model().selected_monitor_id();
    ctx.core_mut().drag_state_mut().tag = crate::core_state::TagDragState {
        active: true,
        initial_tag,
        start,
        dragging: false,
        monitor_id,
        last_tag: Some(tag_idx),
        cursor_on_bar: true,
        last_motion: Some((start, 0)),
        button: btn,
    };
    true
}

/// Update an armed tag interaction. The interaction remains active outside the
/// bar so users can leave and re-enter before releasing.
pub fn drag_tag_motion(ctx: &mut WmCtx, root: Point) -> bool {
    if !ctx.core().drag_state().tag.active {
        return false;
    }

    let (monitor_id, start, was_dragging, previous_modifiers) = {
        let drag = &ctx.core().drag_state().tag;
        (
            drag.monitor_id,
            drag.start,
            drag.dragging,
            drag.last_motion.map_or(0, |(_, modifiers)| modifiers),
        )
    };
    ctx.core_mut().drag_state_mut().tag.last_motion = Some((root, previous_modifiers));

    if !was_dragging && root.manhattan_distance(&start) <= DRAG_THRESHOLD {
        return true;
    }
    if !was_dragging {
        // A tag can still be clicked when there is no selected window, but
        // there is no meaningful object to drag.
        if selected_on_monitor(ctx, monitor_id).is_none() {
            return true;
        }
        ctx.core_mut().drag_state_mut().tag.dragging = true;
        set_cursor_style(ctx, AltCursor::Move);
    }

    let position = position_at(ctx, monitor_id, root);
    let gesture = position.map_or(Gesture::None, BarPosition::to_gesture);
    let tag_idx = match position {
        Some(BarPosition::Tag(idx)) => Some(idx),
        _ => None,
    };
    let cursor_on_bar = position.is_some();
    let changed = {
        let drag = &ctx.core().drag_state().tag;
        drag.cursor_on_bar != cursor_on_bar || drag.last_tag != tag_idx
    };
    if changed || !was_dragging {
        let drag = &mut ctx.core_mut().drag_state_mut().tag;
        drag.cursor_on_bar = cursor_on_bar;
        drag.last_tag = tag_idx;
        if cursor_on_bar {
            ctx.core_mut().bar.hover.set(monitor_id, gesture, true);
        } else {
            ctx.core_mut().bar.hover.clear();
        }
        ctx.request_bar_update();
    }
    true
}

/// Finish a tag click or drag using the modifiers held at release time.
pub fn drag_tag_finish(ctx: &mut WmCtx, modifiers: u32) {
    if !ctx.core().drag_state().tag.active {
        return;
    }
    let drag = std::mem::take(&mut ctx.core_mut().drag_state_mut().tag);
    let root = drag.last_motion.map_or(drag.start, |(root, _)| root);
    let final_position = position_at(ctx, drag.monitor_id, root);
    let final_tag = final_position.and_then(|position| match position {
        BarPosition::Tag(idx) => TagMask::from_index(idx).map(|mask| (idx, mask)),
        _ => None,
    });

    if drag.dragging {
        if let (Some(win), Some((_, tag_mask))) =
            (selected_on_monitor(ctx, drag.monitor_id), final_tag)
        {
            if modifiers & CONTROL != 0 {
                crate::tags::client_tags::tag_all(ctx, tag_mask);
                if modifiers & MOD1 != 0 {
                    crate::tags::view::view_tags(ctx, tag_mask);
                }
            } else {
                apply_window_tag_drop(ctx, win, tag_mask, modifiers);
            }
        }
    } else if modifiers & MOD1 != 0 {
        if let Some(win) = selected_on_monitor(ctx, drag.monitor_id) {
            apply_window_tag_drop(ctx, win, drag.initial_tag, modifiers);
        } else {
            crate::tags::view::view_tags(ctx, drag.initial_tag);
        }
    } else {
        crate::tags::view::view_tags(ctx, drag.initial_tag);
    }

    // Leave the bar in its ordinary hover state. Clearing it unconditionally
    // causes a visible one-frame flash before the next pointer-motion event.
    if let Some(position) = final_position {
        ctx.core_mut()
            .bar
            .hover
            .set(drag.monitor_id, position.to_gesture(), false);
    } else {
        ctx.core_mut().bar.hover.clear();
    }
    if drag.dragging {
        set_cursor_style(ctx, AltCursor::Default);
    }
    ctx.request_bar_update();
}

/// X11's synchronous wrapper around the shared tag interaction state machine.
pub fn drag_tag(ctx: &mut WmCtxX11, bar_pos: BarPosition, btn: MouseButton, start: Point) {
    if !{
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        drag_tag_begin(&mut wm_ctx, bar_pos, btn, start)
    } {
        return;
    }

    let release_modifiers = crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::Default,
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, modifiers } = event {
                ctx.core.drag_state_mut().tag.last_motion = Some((*root, *modifiers));
                return drag_tag_motion(&mut WmCtx::X11(ctx.reborrow()), *root);
            }
            true
        },
    )
    .or_else(|| {
        ctx.core
            .drag_state()
            .tag
            .last_motion
            .map(|(_, modifiers)| modifiers)
    })
    .unwrap_or(0);

    drag_tag_finish(&mut WmCtx::X11(ctx.reborrow()), release_modifiers);
}

#[cfg(test)]
mod tests {
    use super::{CONTROL, MOD1, TagDropBehavior};

    #[test]
    fn alt_is_the_only_modifier_that_makes_a_tag_drop_follow() {
        assert_eq!(TagDropBehavior::from_modifiers(0), TagDropBehavior::Move);
        assert_eq!(
            TagDropBehavior::from_modifiers(CONTROL),
            TagDropBehavior::Move
        );
        assert_eq!(
            TagDropBehavior::from_modifiers(MOD1),
            TagDropBehavior::MoveAndFollow
        );
        assert_eq!(
            TagDropBehavior::from_modifiers(MOD1 | CONTROL),
            TagDropBehavior::MoveAndFollow
        );
    }
}
