use super::{
    BottomBarDrag, OverviewCardAction, OverviewCardDrag, PointerInteractionState,
    SidebarVolumeDrag, SwipeDirection,
};
use crate::actions::ButtonAction;
use crate::types::{
    AltCursor, InteractionSource, MonitorId, MouseButton, Point, Rect, ResizeDirection, TagMask,
    WindowId,
};

fn bottom_bar_drag(anchor_x: i32, anchor_y: i32) -> BottomBarDrag {
    BottomBarDrag::new(
        MouseButton::Left,
        InteractionSource::Pointer,
        MonitorId::from_raw(3),
        Point::new(anchor_x, anchor_y),
        30,
        0,
        super::BottomBarActions {
            left: Box::new(ButtonAction::named(crate::actions::NamedAction::ScrollLeft)),
            right: Box::new(ButtonAction::named(
                crate::actions::NamedAction::ScrollRight,
            )),
            up: Box::new(ButtonAction::named(
                crate::actions::NamedAction::ToggleOverview,
            )),
            click: Box::new(ButtonAction::named(crate::actions::NamedAction::Spawn)),
            hold: Box::new(ButtonAction::named(
                crate::actions::NamedAction::ToggleOverview,
            )),
        },
    )
}

#[test]
fn bottom_bar_swipe_latches_direction_at_threshold() {
    let mut drag = bottom_bar_drag(500, 1000);

    assert_eq!(drag.update(Point::new(529, 1000)), None); // 29px < 30: not yet
    assert_eq!(
        drag.update(Point::new(530, 1000)),
        Some(SwipeDirection::Right)
    );
    assert_eq!(drag.update(Point::new(700, 1000)), None); // already latched
    assert_eq!(drag.latched_direction(), Some(SwipeDirection::Right));

    let mut left = bottom_bar_drag(500, 1000);
    assert_eq!(
        left.update(Point::new(470, 1000)),
        Some(SwipeDirection::Left)
    );
    assert_eq!(left.latched_direction(), Some(SwipeDirection::Left));

    let mut up = bottom_bar_drag(500, 1000);
    assert_eq!(up.update(Point::new(500, 970)), Some(SwipeDirection::Up));
    assert_eq!(up.latched_direction(), Some(SwipeDirection::Up));
}

#[test]
fn bottom_bar_swipe_dominant_axis_wins() {
    // A mostly-upward drag latches Up even though horizontal travel exists.
    let mut drag = bottom_bar_drag(500, 1000);
    assert_eq!(drag.update(Point::new(520, 960)), Some(SwipeDirection::Up));
    assert_eq!(drag.latched_direction(), Some(SwipeDirection::Up));

    // A mostly-rightward drag with some upward travel latches Right.
    let mut drag = bottom_bar_drag(500, 1000);
    assert_eq!(
        drag.update(Point::new(560, 990)),
        Some(SwipeDirection::Right)
    );
}

#[test]
fn bottom_bar_swipe_direction_is_locked_after_first_crossing() {
    let mut drag = bottom_bar_drag(500, 1000);

    assert_eq!(
        drag.update(Point::new(560, 1000)),
        Some(SwipeDirection::Right)
    );
    // Reversing past the threshold on the other side must not re-latch.
    assert_eq!(drag.update(Point::new(300, 100)), None);
    assert_eq!(drag.latched_direction(), Some(SwipeDirection::Right));
}

#[test]
fn bottom_bar_drag_exposes_bound_directional_actions() {
    let drag = bottom_bar_drag(100, 1000);
    assert!(matches!(
        drag.left(),
        ButtonAction::Named {
            action: crate::actions::NamedAction::ScrollLeft,
            ..
        }
    ));
    assert!(matches!(
        drag.right(),
        ButtonAction::Named {
            action: crate::actions::NamedAction::ScrollRight,
            ..
        }
    ));
    assert!(matches!(
        drag.up(),
        ButtonAction::Named {
            action: crate::actions::NamedAction::ToggleOverview,
            ..
        }
    ));
}

#[test]
fn bottom_bar_lifecycle_rejects_overlap_and_wrong_button_release() {
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_overview_card(OverviewCardDrag::new(
            WindowId(1),
            MouseButton::Left,
            InteractionSource::Pointer,
            Point::new(0, 0),
            10,
        ))
        .unwrap();
    assert!(
        interactions
            .begin_bottom_bar(bottom_bar_drag(500, 1000))
            .is_err()
    );
    assert!(!interactions.bottom_bar_gesture_active());

    interactions.cancel_capture();
    interactions
        .begin_bottom_bar(bottom_bar_drag(500, 1000))
        .unwrap();
    assert!(interactions.captured_source() == Some(InteractionSource::Pointer));
    assert!(!interactions.finish_bottom_bar(MouseButton::Right));
    assert!(interactions.bottom_bar_gesture_active());
    assert!(interactions.finish_bottom_bar(MouseButton::Left));
    assert!(!interactions.bottom_bar_gesture_active());
}

#[test]
fn bottom_bar_cancel_clears_the_gesture() {
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_bottom_bar(bottom_bar_drag(500, 1000))
        .unwrap();
    assert!(interactions.cancel_bottom_bar());
    assert!(!interactions.bottom_bar_gesture_active());
    assert!(!interactions.cancel_bottom_bar());
}

fn armed_title_drag(win: WindowId, origin: super::ArmedDragOrigin) -> PointerInteractionState {
    let mut interactions = PointerInteractionState::default();
    interactions
        .arm_title_drag(super::ArmedDragStart {
            win,
            button: MouseButton::Left,
            source: InteractionSource::Pointer,
            origin,
            start: Point::new(300, 10),
            restore_geometry: Rect::new(0, 0, 400, 300),
            was_focused: true,
            was_hidden: false,
            suppress_click_action: false,
        })
        .unwrap();
    interactions
}

#[test]
fn title_reorder_transitions_from_armed_to_move_and_release() {
    let win = WindowId(5);
    let monitor = MonitorId::from_raw(2);
    let mut interactions = armed_title_drag(win, super::ArmedDragOrigin::BarTitle);

    interactions
        .begin_title_reorder(super::TitleReorderDrag::new(monitor))
        .unwrap();
    assert!(interactions.owns_bar_hover());
    let (drag, reorder) = interactions.reordering_interaction().unwrap();
    assert_eq!(drag.win(), win);
    assert_eq!(reorder.monitor_id(), monitor);

    // An active drag or another capture cannot begin a reorder.
    assert!(
        interactions
            .begin_title_reorder(super::TitleReorderDrag::new(monitor))
            .is_err()
    );

    // Leaving the title strip converts the reorder into a move drag.
    interactions
        .activate_reordering_as_move(Point::new(320, 240), Rect::new(0, 0, 400, 300))
        .unwrap();
    assert!(!interactions.owns_bar_hover());
    assert!(interactions.reordering_interaction().is_none());
    let active = interactions.active_interaction().unwrap();
    assert_eq!(active.win(), win);
    assert!(matches!(
        active.operation(),
        super::ActiveWindowOperation::Move
    ));

    // A fresh reorder ends cleanly on release.
    let mut interactions = armed_title_drag(win, super::ArmedDragOrigin::BarTitle);
    interactions
        .begin_title_reorder(super::TitleReorderDrag::new(monitor))
        .unwrap();
    assert!(interactions.owns_bar_hover());
    assert!(interactions.finish_reordering().is_some());
    assert!(!interactions.has_capture());
    assert!(!interactions.owns_bar_hover());
}

#[test]
fn title_reorder_requires_an_armed_capture() {
    let mut interactions = PointerInteractionState::default();
    assert!(
        interactions
            .begin_title_reorder(super::TitleReorderDrag::new(MonitorId::from_raw(1)))
            .is_err()
    );

    // A client-origin armed drag arms fine but the caller never promotes
    // it to a reorder; the state machine itself stays in Armed.
    let interactions = armed_title_drag(WindowId(6), super::ArmedDragOrigin::Client);
    assert!(interactions.reordering_interaction().is_none());
    assert!(interactions.armed_interaction().is_some());
}

#[test]
fn tag_drag_owns_bar_hover_for_its_complete_capture() {
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_tag_drag(super::TagDragState {
            initial_tag: TagMask::single(1).unwrap(),
            start: Point::new(10, 10),
            dragging: false,
            monitor_id: MonitorId::from_raw(1),
            last_tag: Some(1),
            cursor_on_bar: true,
            last_motion: None,
            button: MouseButton::Left,
            source: InteractionSource::Pointer,
        })
        .unwrap();

    assert!(interactions.owns_bar_hover());
    assert!(interactions.finish_tag_drag(MouseButton::Left).is_some());
    assert!(!interactions.owns_bar_hover());
}

#[test]
fn volume_drag_preserves_distance_across_compressed_motion() {
    let mut drag = SidebarVolumeDrag::new(
        MouseButton::Left,
        InteractionSource::Pointer,
        MonitorId::from_raw(3),
        500,
        30,
    );

    assert_eq!(drag.update(395), 3);
    assert_eq!(drag.update(381), 0);
    assert_eq!(drag.update(379), 1);
}

#[test]
fn volume_drag_handles_direction_reversal_with_residual_distance() {
    let mut drag = SidebarVolumeDrag::new(
        MouseButton::Left,
        InteractionSource::Pointer,
        MonitorId::from_raw(3),
        500,
        30,
    );

    assert_eq!(drag.update(475), 0);
    assert_eq!(drag.update(510), 0);
    assert_eq!(drag.update(531), -1);
    assert_eq!(drag.update(469), 2);
}

#[test]
fn sidebar_volume_lifecycle_rejects_overlap_and_wrong_button_release() {
    let drag = SidebarVolumeDrag::new(
        MouseButton::Left,
        InteractionSource::Pointer,
        MonitorId::from_raw(3),
        500,
        30,
    );
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_overview_card(OverviewCardDrag::new(
            WindowId(1),
            MouseButton::Left,
            InteractionSource::Pointer,
            Point::new(0, 0),
            10,
        ))
        .unwrap();
    assert!(interactions.begin_sidebar_volume(drag).is_err());
    assert!(!interactions.sidebar_volume_active());

    interactions.cancel_capture();
    interactions.begin_sidebar_volume(drag).unwrap();
    assert!(!interactions.finish_sidebar_volume(MouseButton::Right));
    assert!(interactions.sidebar_volume_active());
    assert!(interactions.finish_sidebar_volume(MouseButton::Left));
    assert!(!interactions.sidebar_volume_active());
}

#[test]
fn overview_card_gesture_resolves_tap_upward_drag_and_other_drag() {
    let win = WindowId(7);
    let gesture = || {
        OverviewCardDrag::new(
            win,
            MouseButton::Left,
            InteractionSource::Pointer,
            Point::new(500, 400),
            30,
        )
    };

    let mut tap = gesture();
    assert_eq!(tap.update(Point::new(510, 390)), None);
    assert_eq!(tap.action(), OverviewCardAction::Select(win));

    let mut upward = gesture();
    assert_eq!(upward.update(Point::new(520, 350)), Some(true));
    assert_eq!(upward.action(), OverviewCardAction::Close(win));
    assert_eq!(upward.update(Point::new(500, 390)), Some(false));
    assert_eq!(upward.action(), OverviewCardAction::Select(win));

    let mut horizontal = gesture();
    assert_eq!(horizontal.update(Point::new(550, 380)), None);
    assert_eq!(horizontal.action(), OverviewCardAction::Cancel);

    let mut downward = gesture();
    assert_eq!(downward.update(Point::new(500, 450)), None);
    assert_eq!(downward.action(), OverviewCardAction::Cancel);
}

#[test]
fn overview_card_gesture_captures_its_complete_input_sequence() {
    let win = WindowId(9);
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_overview_card(OverviewCardDrag::new(
            win,
            MouseButton::Left,
            InteractionSource::Touch(4),
            Point::new(100, 100),
            20,
        ))
        .unwrap();

    assert_eq!(interactions.captured_button(), Some(MouseButton::Left));
    assert_eq!(
        interactions.captured_source(),
        Some(InteractionSource::Touch(4))
    );
    assert!(interactions.has_capture());
    assert_eq!(
        interactions.finish_overview_card(MouseButton::Left),
        Some(OverviewCardAction::Select(win))
    );
    assert!(!interactions.has_capture());
}

#[test]
fn presentation_is_derived_from_hover_offer() {
    let mut interactions = PointerInteractionState::default();
    assert_eq!(
        interactions.projection(),
        super::InteractionProjection::default()
    );

    interactions.set_hover_offer(super::HoverOffer::Resize {
        win: WindowId(4),
        dir: ResizeDirection::TopLeft,
    });
    assert_eq!(
        interactions.projection(),
        super::InteractionProjection {
            cursor: AltCursor::Resize(ResizeDirection::TopLeft),
            pointer_delivery: super::PointerDelivery::DeliverHoverCommitToWm,
            active_resize_window: None,
        }
    );

    interactions.clear_hover_offer();
    assert_eq!(
        interactions.projection(),
        super::InteractionProjection::default()
    );
}

#[test]
fn beginning_capture_atomically_invalidates_hover_offer() {
    let mut interactions = PointerInteractionState::default();
    interactions.set_hover_offer(super::HoverOffer::Resize {
        win: WindowId(4),
        dir: ResizeDirection::Left,
    });
    interactions
        .begin_sidebar_volume(SidebarVolumeDrag::new(
            MouseButton::Left,
            InteractionSource::Pointer,
            MonitorId::from_raw(1),
            500,
            30,
        ))
        .unwrap();

    assert_eq!(interactions.hover_offer(), super::HoverOffer::None);
    assert_eq!(interactions.projection().cursor, AltCursor::VerticalAdjust);
    assert_eq!(
        interactions.projection().pointer_delivery,
        super::PointerDelivery::Default
    );
    assert!(!interactions.set_hover_offer(super::HoverOffer::Resize {
        win: WindowId(5),
        dir: ResizeDirection::Right,
    }));
}

#[test]
fn captured_interaction_cursor_tracks_internal_transitions() {
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_bottom_bar(bottom_bar_drag(500, 1000))
        .unwrap();
    assert_eq!(interactions.projection().cursor, AltCursor::Move);

    assert_eq!(
        interactions.update_bottom_bar(Point::new(500, 960)),
        Some(SwipeDirection::Up)
    );
    assert_eq!(interactions.projection().cursor, AltCursor::VerticalAdjust);

    interactions.cancel_capture();
    interactions
        .begin_resize(
            WindowId(8),
            MouseButton::Right,
            InteractionSource::Pointer,
            ResizeDirection::BottomRight,
            Point::new(10, 10),
            Rect::new(0, 0, 100, 100),
        )
        .unwrap();
    assert_eq!(
        interactions.projection().cursor,
        AltCursor::Resize(ResizeDirection::BottomRight)
    );
}

#[test]
fn overview_close_threshold_drives_destructive_cursor() {
    let mut interactions = PointerInteractionState::default();
    interactions
        .begin_overview_card(OverviewCardDrag::new(
            WindowId(9),
            MouseButton::Left,
            InteractionSource::Pointer,
            Point::new(100, 100),
            20,
        ))
        .unwrap();
    assert_eq!(interactions.projection().cursor, AltCursor::Move);

    assert_eq!(
        interactions.update_overview_card(Point::new(100, 70)),
        Some(true)
    );
    assert_eq!(interactions.projection().cursor, AltCursor::Close);
}
