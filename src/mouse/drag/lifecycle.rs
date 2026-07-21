//! Narrow lifecycle coordination for interactive move and resize drags.
//!
//! This module deliberately does not accept `WmCtx`. It coordinates only the
//! backend-neutral interaction state and the backend capability needed to
//! project an active resize into the window-system protocol.

use crate::backend::InteractiveResizeOps;
use crate::core_state::{
    ArmedDragType, DragAlreadyActive, DragCancelReason, DragInteraction, DragNotArmed, DragState,
    DragType,
};
use crate::types::{MouseButton, Point, Rect, ResizeDirection, WindowId};

#[derive(Debug, Clone, Copy)]
pub struct ResizeDragParams {
    pub win: WindowId,
    pub button: MouseButton,
    pub direction: ResizeDirection,
    pub start: Point,
    pub geometry: Rect,
}

pub fn begin_resize(
    interactions: &mut DragState,
    protocol: &dyn InteractiveResizeOps,
    params: ResizeDragParams,
) -> Result<(), DragAlreadyActive> {
    if !interactions.interactive().is_idle() {
        return Err(DragAlreadyActive);
    }

    // The precondition above makes the state commit infallible. Apply the
    // backend projection first so an attached Wayland compositor observes the
    // resizing state before the first size configure can be emitted.
    protocol.begin_interactive_resize(params.win);
    interactions
        .begin_resize(
            params.win,
            params.button,
            params.direction,
            params.start,
            params.geometry,
        )
        .expect("validated idle interaction must accept resize");
    Ok(())
}

pub fn activate_armed_resize(
    interactions: &mut DragState,
    protocol: &dyn InteractiveResizeOps,
    direction: ResizeDirection,
    start: Point,
    geometry: Rect,
) -> Result<(), DragNotArmed> {
    let win = interactions.armed_interaction().ok_or(DragNotArmed)?.win();
    protocol.begin_interactive_resize(win);
    interactions
        .activate_armed(ArmedDragType::Resize(direction), start, geometry)
        .expect("validated armed interaction must activate");
    Ok(())
}

pub fn finish(
    interactions: &mut DragState,
    protocol: &dyn InteractiveResizeOps,
    button: MouseButton,
) -> Option<DragInteraction> {
    let finished = interactions.finish_active(button)?;
    if matches!(finished.drag_type(), DragType::Resize(_)) {
        // End the backend projection so Wayland emits the final configure
        // without xdg-toplevel's `resizing` state. X11 implements this as a
        // no-op because its pointer grab owns the resize lifetime.
        protocol.end_interactive_resize(finished.win());
    }
    Some(finished)
}

pub fn cancel(
    interactions: &mut DragState,
    protocol: &dyn InteractiveResizeOps,
    reason: DragCancelReason,
) -> Option<DragInteraction> {
    let cancelled = interactions.cancel_interactive()?;
    if matches!(cancelled.drag_type(), DragType::Resize(_)) {
        protocol.end_interactive_resize(cancelled.win());
    }
    log::debug!(
        "cancelled {:?} interaction for {:?}: {reason:?}",
        cancelled.drag_type(),
        cancelled.win(),
    );
    Some(cancelled)
}

pub fn cancel_window(
    interactions: &mut DragState,
    protocol: &dyn InteractiveResizeOps,
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
    cancel(interactions, protocol, reason)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::core_state::ArmedDragParams;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ProtocolEvent {
        Begin(WindowId),
        End(WindowId),
    }

    #[derive(Default)]
    struct RecordingProtocol {
        events: RefCell<Vec<ProtocolEvent>>,
    }

    impl InteractiveResizeOps for RecordingProtocol {
        fn begin_interactive_resize(&self, window: WindowId) {
            self.events.borrow_mut().push(ProtocolEvent::Begin(window));
        }

        fn end_interactive_resize(&self, window: WindowId) {
            self.events.borrow_mut().push(ProtocolEvent::End(window));
        }
    }

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
            direction: ResizeDirection::BottomRight,
            start: Point::new(810, 620),
            geometry: geometry(),
        }
    }

    #[test]
    fn resize_lifecycle_pairs_protocol_begin_and_end() {
        let win = WindowId(7);
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();

        begin_resize(&mut interactions, &protocol, resize_params(win)).unwrap();
        assert!(matches!(
            interactions
                .active_interaction()
                .map(DragInteraction::drag_type),
            Some(DragType::Resize(ResizeDirection::BottomRight))
        ));

        let finished = finish(&mut interactions, &protocol, MouseButton::Right).unwrap();
        assert_eq!(finished.win(), win);
        assert!(interactions.interactive().is_idle());
        assert_eq!(
            *protocol.events.borrow(),
            vec![ProtocolEvent::Begin(win), ProtocolEvent::End(win)]
        );
    }

    #[test]
    fn rejected_second_resize_does_not_emit_a_protocol_event() {
        let first = WindowId(7);
        let second = WindowId(8);
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();

        begin_resize(&mut interactions, &protocol, resize_params(first)).unwrap();
        assert_eq!(
            begin_resize(&mut interactions, &protocol, resize_params(second)),
            Err(DragAlreadyActive)
        );
        assert_eq!(*protocol.events.borrow(), vec![ProtocolEvent::Begin(first)]);
        assert_eq!(interactions.active_interaction().unwrap().win(), first);
    }

    #[test]
    fn invalid_armed_activation_preserves_an_existing_active_drag() {
        let win = WindowId(7);
        let mut interactions = DragState::default();
        interactions
            .begin_move(win, MouseButton::Left, Point::new(100, 100), geometry())
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
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();

        begin_resize(&mut interactions, &protocol, resize_params(win)).unwrap();
        assert!(finish(&mut interactions, &protocol, MouseButton::Left).is_none());
        assert!(interactions.active_interaction().is_some());
        assert_eq!(*protocol.events.borrow(), vec![ProtocolEvent::Begin(win)]);
    }

    #[test]
    fn tree_resize_owns_a_snapshot_without_entering_protocol_resize() {
        let win = WindowId(7);
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();
        let mut tree = crate::layouts::tree::LayoutTree::default();
        tree.apply_preset(
            crate::layouts::tree::Preset::MasterStack,
            &[win, WindowId(8)],
            Some(win),
            1,
            0.5,
        );

        interactions
            .begin_tree_resize(
                win,
                MouseButton::Right,
                ResizeDirection::Right,
                Point::new(100, 100),
                geometry(),
                tree,
            )
            .unwrap();
        let active = interactions.active_interaction().unwrap();
        assert_eq!(
            active.drag_type(),
            DragType::TreeResize(ResizeDirection::Right)
        );
        assert!(matches!(
            active.operation(),
            crate::core_state::DragOperationRef::TreeResize { .. }
        ));

        let _ = finish(&mut interactions, &protocol, MouseButton::Right).unwrap();
        assert!(protocol.events.borrow().is_empty());
    }

    #[test]
    fn move_lifecycle_never_touches_resize_protocol() {
        let win = WindowId(7);
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();
        interactions
            .begin_move(win, MouseButton::Left, Point::new(100, 100), geometry())
            .unwrap();

        let finished = finish(&mut interactions, &protocol, MouseButton::Left).unwrap();
        assert_eq!(finished.drag_type(), DragType::Move);
        assert!(interactions.interactive().is_idle());
        assert!(protocol.events.borrow().is_empty());
    }

    #[test]
    fn armed_resize_becomes_protocol_managed_only_when_activated() {
        let win = WindowId(7);
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();
        interactions
            .arm_title_drag(ArmedDragParams {
                win,
                button: MouseButton::Right,
                start: Point::new(100, 100),
                geometry: geometry(),
                restore_geometry: geometry(),
                was_focused: true,
                was_hidden: false,
                suppress_click_action: false,
            })
            .unwrap();

        assert!(protocol.events.borrow().is_empty());
        activate_armed_resize(
            &mut interactions,
            &protocol,
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
        assert_eq!(*protocol.events.borrow(), vec![ProtocolEvent::Begin(win)]);
    }

    #[test]
    fn cancellation_is_scoped_to_the_requested_window() {
        let win = WindowId(7);
        let protocol = RecordingProtocol::default();
        let mut interactions = DragState::default();
        begin_resize(&mut interactions, &protocol, resize_params(win)).unwrap();

        assert!(
            cancel_window(
                &mut interactions,
                &protocol,
                WindowId(8),
                DragCancelReason::WindowDestroyed,
            )
            .is_none()
        );
        assert!(interactions.active_interaction().is_some());
        let cancelled = cancel_window(
            &mut interactions,
            &protocol,
            win,
            DragCancelReason::WindowDestroyed,
        )
        .unwrap();
        assert_eq!(cancelled.win(), win);
        assert!(interactions.interactive().is_idle());
        assert_eq!(
            *protocol.events.borrow(),
            vec![ProtocolEvent::Begin(win), ProtocolEvent::End(win)]
        );
    }
}
