//! Pure lifecycle transitions for interactive move and resize drags.
//!
//! Native resize state is derived through `InteractionPresentation`; this
//! module never invokes a backend protocol edge directly.

use crate::core_state::{
    ActiveResizeParams, ArmedDragType, DragCancelReason, DragInteraction, DragNotArmed, DragState,
    InteractionAlreadyActive,
};
use crate::types::{MouseButton, Point, Rect, ResizeDirection, WindowId};

pub type ResizeDragParams = ActiveResizeParams;

pub fn begin_resize(
    interactions: &mut DragState,
    params: ResizeDragParams,
) -> Result<(), InteractionAlreadyActive> {
    interactions.begin_resize_with_policy(params)
}

pub fn activate_armed_resize(
    interactions: &mut DragState,
    direction: ResizeDirection,
    start: Point,
    geometry: Rect,
) -> Result<(), DragNotArmed> {
    interactions
        .activate_armed(ArmedDragType::Resize(direction), start, geometry)
        .map(|_| ())
}

pub fn finish(interactions: &mut DragState, button: MouseButton) -> Option<DragInteraction> {
    interactions.finish_active(button)
}

pub fn cancel(interactions: &mut DragState, reason: DragCancelReason) -> Option<DragInteraction> {
    let cancelled = interactions.cancel_interactive()?;
    log::debug!(
        "cancelled {:?} interaction for {:?}: {reason:?}",
        cancelled.drag_type(),
        cancelled.win(),
    );
    Some(cancelled)
}

pub fn cancel_window(
    interactions: &mut DragState,
    window: WindowId,
    reason: DragCancelReason,
) -> Option<DragInteraction> {
    let belongs_to_window = interactions
        .active_interaction()
        .or_else(|| interactions.armed_interaction())
        .is_some_and(|drag| drag.win() == window);
    if !belongs_to_window {
        return None;
    }
    cancel(interactions, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_state::{ArmedDragParams, DragType};
    use crate::types::InteractionSource;

    fn geometry() -> Rect {
        Rect {
            x: 10,
            y: 20,
            w: 800,
            h: 600,
        }
    }

    fn resize_params(win: WindowId) -> ResizeDragParams {
        ResizeDragParams {
            win,
            button: MouseButton::Right,
            source: InteractionSource::Pointer,
            direction: ResizeDirection::BottomRight,
            start: Point::new(810, 620),
            geometry: geometry(),
            policy: crate::core_state::ResizePolicy::Free,
        }
    }

    #[test]
    fn resize_lifecycle_exposes_and_clears_derived_resize_projection() {
        let win = WindowId(7);
        let mut interactions = DragState::default();

        begin_resize(&mut interactions, resize_params(win)).unwrap();
        assert!(matches!(
            interactions
                .active_interaction()
                .map(DragInteraction::drag_type),
            Some(DragType::Resize(ResizeDirection::BottomRight))
        ));

        assert_eq!(interactions.presentation().active_resize_window, Some(win));
        let finished = finish(&mut interactions, MouseButton::Right).unwrap();
        assert_eq!(finished.win(), win);
        assert!(!interactions.has_capture());
        assert_eq!(interactions.presentation().active_resize_window, None);
    }

    #[test]
    fn rejected_second_resize_preserves_first_projection() {
        let first = WindowId(7);
        let second = WindowId(8);
        let mut interactions = DragState::default();

        begin_resize(&mut interactions, resize_params(first)).unwrap();
        assert_eq!(
            begin_resize(&mut interactions, resize_params(second)),
            Err(InteractionAlreadyActive)
        );
        assert_eq!(
            interactions.presentation().active_resize_window,
            Some(first)
        );
        assert_eq!(interactions.active_interaction().unwrap().win(), first);
    }

    #[test]
    fn resize_rejects_non_window_interaction_without_state_change() {
        let mut interactions = DragState::default();
        interactions
            .begin_overview_card(crate::core_state::OverviewCardDrag::new(
                WindowId(1),
                MouseButton::Left,
                InteractionSource::Pointer,
                Point::new(0, 0),
                10,
            ))
            .unwrap();

        assert_eq!(
            begin_resize(&mut interactions, resize_params(WindowId(7)),),
            Err(InteractionAlreadyActive)
        );
        assert_eq!(interactions.presentation().active_resize_window, None);
        assert!(matches!(
            interactions.capture(),
            Some(crate::core_state::CapturedInteraction::OverviewCard(_))
        ));
    }

    #[test]
    fn invalid_armed_activation_preserves_an_existing_active_drag() {
        let win = WindowId(7);
        let mut interactions = DragState::default();
        interactions
            .begin_move(
                win,
                MouseButton::Left,
                InteractionSource::Pointer,
                Point::new(100, 100),
                geometry(),
            )
            .unwrap();

        assert_eq!(
            interactions.activate_armed(
                ArmedDragType::Resize(ResizeDirection::Right),
                Point::new(810, 300),
                geometry(),
            ),
            Err(DragNotArmed)
        );
        assert_eq!(interactions.active_interaction().unwrap().win(), win);
        assert_eq!(
            interactions.active_interaction().unwrap().drag_type(),
            DragType::Move
        );
    }

    #[test]
    fn wrong_button_does_not_finish_resize() {
        let win = WindowId(7);
        let mut interactions = DragState::default();

        begin_resize(&mut interactions, resize_params(win)).unwrap();
        assert!(finish(&mut interactions, MouseButton::Left).is_none());
        assert!(interactions.active_interaction().is_some());
        assert_eq!(interactions.presentation().active_resize_window, Some(win));
    }

    #[test]
    fn tree_resize_owns_a_snapshot_without_requesting_toplevel_resize_state() {
        let win = WindowId(7);
        let mut interactions = DragState::default();
        let mut tree = crate::layouts::tree::LayoutTree::default();
        tree.apply_preset(
            crate::layouts::tree::Preset::MasterStack,
            &[win, WindowId(8)],
            1,
        );

        interactions
            .begin_tree_resize(crate::core_state::TreeResizeParams {
                win,
                button: MouseButton::Right,
                source: InteractionSource::Pointer,
                direction: ResizeDirection::Right,
                start: Point::new(100, 100),
                geometry: geometry(),
                origin: tree,
            })
            .unwrap();
        let active = interactions.active_interaction().unwrap();
        assert_eq!(
            active.drag_type(),
            DragType::TreeResize(ResizeDirection::Right)
        );
        assert!(matches!(
            active.operation(),
            crate::core_state::DragOperation::TreeResize { .. }
        ));

        assert_eq!(interactions.presentation().active_resize_window, None);
        let _ = finish(&mut interactions, MouseButton::Right).unwrap();
    }

    #[test]
    fn move_lifecycle_never_requests_resize_projection() {
        let win = WindowId(7);
        let mut interactions = DragState::default();
        interactions
            .begin_move(
                win,
                MouseButton::Left,
                InteractionSource::Pointer,
                Point::new(100, 100),
                geometry(),
            )
            .unwrap();

        let finished = finish(&mut interactions, MouseButton::Left).unwrap();
        assert_eq!(finished.drag_type(), DragType::Move);
        assert!(!interactions.has_capture());
        assert_eq!(interactions.presentation().active_resize_window, None);
    }

    #[test]
    fn armed_resize_requests_projection_only_when_activated() {
        let win = WindowId(7);
        let mut interactions = DragState::default();
        interactions
            .arm_title_drag(ArmedDragParams {
                win,
                button: MouseButton::Right,
                source: InteractionSource::Pointer,
                origin: crate::core_state::ArmedDragOrigin::Client,
                start: Point::new(100, 100),
                geometry: geometry(),
                restore_geometry: geometry(),
                was_focused: true,
                was_hidden: false,
                suppress_click_action: false,
            })
            .unwrap();

        assert_eq!(interactions.presentation().active_resize_window, None);
        activate_armed_resize(
            &mut interactions,
            ResizeDirection::Right,
            Point::new(810, 300),
            geometry(),
        )
        .unwrap();

        assert!(matches!(
            interactions
                .active_interaction()
                .map(DragInteraction::drag_type),
            Some(DragType::Resize(ResizeDirection::Right))
        ));
        assert_eq!(interactions.presentation().active_resize_window, Some(win));
    }

    #[test]
    fn cancellation_is_scoped_to_the_requested_window() {
        let win = WindowId(7);
        let mut interactions = DragState::default();
        begin_resize(&mut interactions, resize_params(win)).unwrap();

        assert!(
            cancel_window(
                &mut interactions,
                WindowId(8),
                DragCancelReason::WindowDestroyed,
            )
            .is_none()
        );
        assert!(interactions.active_interaction().is_some());
        let cancelled =
            cancel_window(&mut interactions, win, DragCancelReason::WindowDestroyed).unwrap();
        assert_eq!(cancelled.win(), win);
        assert!(!interactions.has_capture());
        assert_eq!(interactions.presentation().active_resize_window, None);
    }
}
