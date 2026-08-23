//! Click and drag interactions for tag indicators in the bar.

use crate::config::{CONTROL, MOD1};
use crate::contexts::WmCtx;
use crate::mouse::constants::DRAG_THRESHOLD;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagReleaseAction {
    None,
    View(TagMask),
    DropWindow {
        win: WindowId,
        tags: TagMask,
        modifiers: u32,
    },
    TagAll {
        tags: TagMask,
        follow: bool,
    },
}

fn resolve_tag_release(
    drag: &crate::core_state::TagDragState,
    selected_window: Option<WindowId>,
    final_tag: Option<TagMask>,
    modifiers: u32,
) -> TagReleaseAction {
    if drag.dragging {
        let (Some(win), Some(tags)) = (selected_window, final_tag) else {
            return TagReleaseAction::None;
        };
        if modifiers & CONTROL != 0 {
            TagReleaseAction::TagAll {
                tags,
                follow: modifiers & MOD1 != 0,
            }
        } else {
            TagReleaseAction::DropWindow {
                win,
                tags,
                modifiers,
            }
        }
    } else if modifiers & MOD1 != 0 {
        selected_window.map_or(TagReleaseAction::View(drag.initial_tag), |win| {
            TagReleaseAction::DropWindow {
                win,
                tags: drag.initial_tag,
                modifiers,
            }
        })
    } else {
        TagReleaseAction::View(drag.initial_tag)
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

fn selected_on_monitor(
    monitors: &crate::monitor::MonitorManager,
    monitor_id: MonitorId,
) -> Option<WindowId> {
    monitors
        .get(monitor_id)
        .and_then(|monitor| monitor.selected)
}

/// Arm a tag click. Motion beyond [`DRAG_THRESHOLD`] promotes it to a drag.
/// Keeping clicks armed until release makes modifier handling identical on X11
/// and Wayland, and avoids a visual flash when the pointer merely clicks a tag.
pub fn drag_tag_begin(
    ctx: &mut WmCtx,
    bar_pos: BarPosition,
    btn: MouseButton,
    source: InteractionSource,
    start: Point,
) -> bool {
    let BarPosition::Tag(tag_idx) = bar_pos else {
        return false;
    };
    let Some(initial_tag) = TagMask::from_index(tag_idx) else {
        return false;
    };
    let monitor_id = ctx.core().model().selected_monitor_id();
    ctx.core_mut()
        .interaction_mut().drag
        .begin_tag_drag(crate::core_state::TagDragState {
            initial_tag,
            start,
            dragging: false,
            monitor_id,
            last_tag: Some(tag_idx),
            cursor_on_bar: true,
            last_motion: Some((start, 0)),
            button: btn,
            source,
        })
        .is_ok()
}

/// Update an armed tag interaction. The interaction remains active outside the
/// bar so users can leave and re-enter before releasing.
pub fn apply_drag_tag_motion(ctx: &mut WmCtx, root: Point) -> bool {
    let (monitor_id, start, was_dragging, previous_modifiers) = {
        let Some(drag) = ctx.core().interaction().drag.tag_drag() else {
            return false;
        };
        (
            drag.monitor_id,
            drag.start,
            drag.dragging,
            drag.last_motion.map_or(0, |(_, modifiers)| modifiers),
        )
    };
    ctx.core_mut()
        .interaction_mut().drag
        .tag_drag_mut()
        .expect("tag capture remained active")
        .last_motion = Some((root, previous_modifiers));

    if !was_dragging && root.manhattan_distance(&start) <= DRAG_THRESHOLD {
        return true;
    }
    if !was_dragging {
        // A tag can still be clicked when there is no selected window, but
        // there is no meaningful object to drag.
        if selected_on_monitor(&ctx.core().model().monitors, monitor_id).is_none() {
            return true;
        }
        ctx.core_mut()
            .interaction_mut().drag
            .tag_drag_mut()
            .expect("tag capture remained active")
            .dragging = true;
        ctx.set_cursor_style(AltCursor::Move);
    }

    let position = super::bar_position_on_monitor(ctx, monitor_id, root);
    let gesture = position.map_or(Gesture::None, BarPosition::to_gesture);
    let tag_idx = match position {
        Some(BarPosition::Tag(idx)) => Some(idx),
        _ => None,
    };
    let cursor_on_bar = position.is_some();
    let changed = {
        let drag = ctx
            .core()
            .interaction().drag
            .tag_drag()
            .expect("tag capture remained active");
        drag.cursor_on_bar != cursor_on_bar || drag.last_tag != tag_idx
    };
    if changed || !was_dragging {
        let drag = ctx
            .core_mut()
            .interaction_mut().drag
            .tag_drag_mut()
            .expect("tag capture remained active");
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
    let Some(button) = ctx.core().interaction().drag.tag_drag().map(|drag| drag.button) else {
        return;
    };
    let drag = ctx
        .core_mut()
        .interaction_mut().drag
        .finish_tag_drag(button)
        .expect("matching tag capture remained active");
    let root = drag.last_motion.map_or(drag.start, |(root, _)| root);
    let final_position = super::bar_position_on_monitor(ctx, drag.monitor_id, root);
    let final_tag = final_position.and_then(|position| match position {
        BarPosition::Tag(idx) => TagMask::from_index(idx),
        _ => None,
    });
    let selected_window = selected_on_monitor(&ctx.core().model().monitors, drag.monitor_id);
    let action = resolve_tag_release(&drag, selected_window, final_tag, modifiers);
    apply_tag_release(ctx, action);
    finish_tag_release_presentation(ctx, &drag, final_position);
}

fn apply_tag_release(ctx: &mut WmCtx<'_>, action: TagReleaseAction) {
    match action {
        TagReleaseAction::None => {}
        TagReleaseAction::View(tags) => crate::tags::view::view_tags(ctx, tags),
        TagReleaseAction::DropWindow {
            win,
            tags,
            modifiers,
        } => apply_window_tag_drop(ctx, win, tags, modifiers),
        TagReleaseAction::TagAll { tags, follow } => {
            crate::tags::client_tags::tag_all(ctx, tags);
            if follow {
                crate::tags::view::view_tags(ctx, tags);
            }
        }
    }
}

fn finish_tag_release_presentation(
    ctx: &mut WmCtx<'_>,
    drag: &crate::core_state::TagDragState,
    final_position: Option<BarPosition>,
) {
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
        ctx.set_cursor_style(AltCursor::Default);
    }
    ctx.request_bar_update();
}

#[cfg(test)]
mod tests {
    use super::{CONTROL, MOD1, TagDropBehavior, TagReleaseAction, resolve_tag_release};
    use crate::types::{InteractionSource, MonitorId, MouseButton, Point, TagMask, WindowId};

    fn drag(dragging: bool, initial_tag: TagMask) -> crate::core_state::TagDragState {
        crate::core_state::TagDragState {
            initial_tag,
            start: Point::default(),
            dragging,
            monitor_id: MonitorId::from_raw(1),
            last_tag: None,
            cursor_on_bar: true,
            last_motion: None,
            button: MouseButton::Left,
            source: InteractionSource::Pointer,
        }
    }

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

    #[test]
    fn release_policy_is_resolved_without_wm_state_or_backend_effects() {
        let initial = TagMask::single(1).unwrap();
        let target = TagMask::single(2).unwrap();
        let win = WindowId(10);

        assert_eq!(
            resolve_tag_release(&drag(false, initial), None, None, 0),
            TagReleaseAction::View(initial)
        );
        assert_eq!(
            resolve_tag_release(
                &drag(true, initial),
                Some(win),
                Some(target),
                CONTROL | MOD1
            ),
            TagReleaseAction::TagAll {
                tags: target,
                follow: true,
            }
        );
        assert_eq!(
            resolve_tag_release(&drag(true, initial), Some(win), None, 0),
            TagReleaseAction::None
        );
    }
}
