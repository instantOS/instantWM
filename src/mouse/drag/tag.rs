//! Click and drag interactions for tag indicators in the bar.

use crate::contexts::WmCtx;
use crate::mouse::constants::DRAG_THRESHOLD;
use crate::tags::client_tags::TagTransferMode;
use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TagDropBehavior {
    Move,
    MoveAndFollow,
}

impl TagDropBehavior {
    fn from_modifiers(modifiers: ModMask) -> Self {
        if modifiers.contains(Modifier::Alt) {
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
        modifiers: ModMask,
    },
    TransferTags {
        monitor_id: MonitorId,
        source: TagMask,
        target: TagMask,
        mode: TagTransferMode,
        follow: bool,
    },
}

fn resolve_tag_release(
    drag: &crate::core_state::TagDragState,
    selected_window: Option<WindowId>,
    final_tag: Option<TagMask>,
    modifiers: ModMask,
) -> TagReleaseAction {
    if drag.dragging {
        let Some(tags) = final_tag else {
            return TagReleaseAction::None;
        };
        let mode = if modifiers.contains(Modifier::Shift) {
            Some(TagTransferMode::Swap)
        } else if modifiers.contains(Modifier::Control) {
            Some(TagTransferMode::Move)
        } else {
            None
        };
        if let Some(mode) = mode {
            TagReleaseAction::TransferTags {
                monitor_id: drag.monitor_id,
                source: drag.initial_tag,
                target: tags,
                mode,
                follow: modifiers.contains(Modifier::Alt),
            }
        } else {
            selected_window.map_or(TagReleaseAction::None, |win| TagReleaseAction::DropWindow {
                win,
                tags,
                modifiers,
            })
        }
    } else if modifiers.contains(Modifier::Alt) {
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
    modifiers: ModMask,
) {
    match TagDropBehavior::from_modifiers(modifiers) {
        TagDropBehavior::Move => {
            let _ = crate::tags::client_tags::set_client_tag(ctx, win, tag_mask);
        }
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
    let monitor_id = ctx.core().state.model.selected_monitor_id();
    ctx.transition_pointer_interaction(|drag| {
        drag.begin(crate::core_state::TagDragState {
            initial_tag,
            start,
            dragging: false,
            monitor_id,
            last_tag: Some(tag_idx),
            cursor_on_bar: true,
            last_motion: Some((start, ModMask::NONE)),
            button: btn,
            source,
        })
        .is_ok()
    })
}

/// Update an armed tag interaction. The interaction remains active outside the
/// bar so users can leave and re-enter before releasing.
pub fn apply_drag_tag_motion(ctx: &mut WmCtx, root: Point) -> bool {
    let (monitor_id, start, was_dragging, previous_modifiers) = {
        let Some(drag) = ctx
            .core()
            .state
            .interaction
            .drag
            .captured::<crate::core_state::TagDragState>()
        else {
            return false;
        };
        (
            drag.monitor_id,
            drag.start,
            drag.dragging,
            drag.last_motion
                .map_or(ModMask::NONE, |(_, modifiers)| modifiers),
        )
    };
    ctx.transition_pointer_interaction(|state| {
        state
            .captured_mut::<crate::core_state::TagDragState>()
            .expect("tag capture remained active")
            .last_motion = Some((root, previous_modifiers));
    });

    if !was_dragging && root.manhattan_distance(&start) <= DRAG_THRESHOLD {
        return true;
    }
    if !was_dragging {
        ctx.transition_pointer_interaction(|state| {
            state
                .captured_mut::<crate::core_state::TagDragState>()
                .expect("tag capture remained active")
                .dragging = true;
        });
    }

    let position = super::bar_position_on_monitor(ctx.core(), monitor_id, root);
    let gesture = position.map_or(Gesture::None, BarPosition::to_gesture);
    let tag_idx = match position {
        Some(BarPosition::Tag(idx)) => Some(idx),
        _ => None,
    };
    let cursor_on_bar = position.is_some();
    let changed = {
        let drag = ctx
            .core()
            .state
            .interaction
            .drag
            .captured::<crate::core_state::TagDragState>()
            .expect("tag capture remained active");
        drag.cursor_on_bar != cursor_on_bar || drag.last_tag != tag_idx
    };
    if changed || !was_dragging {
        ctx.transition_pointer_interaction(|state| {
            let drag = state
                .captured_mut::<crate::core_state::TagDragState>()
                .expect("tag capture remained active");
            drag.cursor_on_bar = cursor_on_bar;
            drag.last_tag = tag_idx;
        });
        if cursor_on_bar {
            ctx.core_mut().bar.hover.set(monitor_id, gesture, true);
        } else {
            ctx.core_mut().bar.hover.clear();
        }
        ctx.core_mut().bar.invalidate_content();
    }
    true
}

/// Finish a tag click or drag using the modifiers held at release time.
/// Ctrl moves all windows from the dragged tag; Shift swaps its windows with
/// the destination (taking precedence over Ctrl). Alt follows the destination.
pub fn drag_tag_finish(ctx: &mut WmCtx, modifiers: ModMask) {
    let Some(button) = ctx
        .core()
        .state
        .interaction
        .drag
        .captured::<crate::core_state::TagDragState>()
        .map(|drag| drag.button)
    else {
        return;
    };
    let drag = ctx
        .transition_pointer_interaction(|state| {
            state.finish::<crate::core_state::TagDragState>(button)
        })
        .expect("matching tag capture remained active");
    let root = drag.last_motion.map_or(drag.start, |(root, _)| root);
    let final_position = super::bar_position_on_monitor(ctx.core(), drag.monitor_id, root);
    let final_tag = final_position.and_then(|position| match position {
        BarPosition::Tag(idx) => TagMask::from_index(idx),
        _ => None,
    });
    let selected_window = selected_on_monitor(&ctx.core().state.model.monitors, drag.monitor_id);
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
        TagReleaseAction::TransferTags {
            monitor_id,
            source,
            target,
            mode,
            follow,
        } => {
            crate::tags::client_tags::transfer_tag_clients(ctx, monitor_id, source, target, mode);
            if follow {
                crate::focus::select_monitor(ctx, monitor_id);
                crate::tags::view::view_tags(ctx, target);
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
    ctx.core_mut().bar.invalidate_content();
}

#[cfg(test)]
mod tests {
    use super::{TagDropBehavior, TagReleaseAction, resolve_tag_release};
    use crate::tags::client_tags::TagTransferMode;
    use crate::types::{
        InteractionSource, ModMask, Modifier, MonitorId, MouseButton, Point, TagMask, WindowId,
    };

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
        assert_eq!(
            TagDropBehavior::from_modifiers(ModMask::NONE),
            TagDropBehavior::Move
        );
        assert_eq!(
            TagDropBehavior::from_modifiers(ModMask::from_modifiers([Modifier::Control])),
            TagDropBehavior::Move
        );
        assert_eq!(
            TagDropBehavior::from_modifiers(ModMask::from_modifiers([Modifier::Alt])),
            TagDropBehavior::MoveAndFollow
        );
        assert_eq!(
            TagDropBehavior::from_modifiers(ModMask::from_modifiers([
                Modifier::Alt,
                Modifier::Control
            ])),
            TagDropBehavior::MoveAndFollow
        );
    }

    #[test]
    fn bulk_drag_does_not_require_focus_and_shift_takes_precedence() {
        let initial = TagMask::single(1).unwrap();
        let target = TagMask::single(2).unwrap();
        for (modifiers, mode) in [
            (vec![Modifier::Control], TagTransferMode::Move),
            (vec![Modifier::Shift], TagTransferMode::Swap),
            (
                vec![Modifier::Shift, Modifier::Control],
                TagTransferMode::Swap,
            ),
        ] {
            assert_eq!(
                resolve_tag_release(
                    &drag(true, initial),
                    None,
                    Some(target),
                    ModMask::from_modifiers(modifiers),
                ),
                TagReleaseAction::TransferTags {
                    monitor_id: MonitorId::from_raw(1),
                    source: initial,
                    target,
                    mode,
                    follow: false,
                }
            );
        }
        assert_eq!(
            resolve_tag_release(&drag(true, initial), None, Some(target), ModMask::NONE),
            TagReleaseAction::None
        );
        for modifier in [Modifier::Shift, Modifier::Control] {
            assert_eq!(
                resolve_tag_release(
                    &drag(false, initial),
                    None,
                    Some(target),
                    ModMask::from_modifiers([modifier]),
                ),
                TagReleaseAction::View(initial)
            );
        }
    }

    #[test]
    fn release_policy_is_resolved_without_wm_state_or_backend_effects() {
        let initial = TagMask::single(1).unwrap();
        let target = TagMask::single(2).unwrap();
        let win = WindowId(10);

        assert_eq!(
            resolve_tag_release(&drag(false, initial), None, None, ModMask::NONE),
            TagReleaseAction::View(initial)
        );
        assert_eq!(
            resolve_tag_release(
                &drag(true, initial),
                Some(win),
                Some(target),
                ModMask::from_modifiers([Modifier::Alt, Modifier::Control])
            ),
            TagReleaseAction::TransferTags {
                monitor_id: MonitorId::from_raw(1),
                source: initial,
                target,
                mode: TagTransferMode::Move,
                follow: true,
            }
        );
        assert_eq!(
            resolve_tag_release(&drag(true, initial), Some(win), None, ModMask::NONE),
            TagReleaseAction::None
        );
    }
}
