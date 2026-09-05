//! Backend-independent touchscreen event handling.
//!
//! Input backends provide normalized absolute coordinates. This module maps
//! them into compositor space, applies output transforms, resolves the target
//! surface, updates keyboard focus on touch-down, and emits native `wl_touch`
//! events through Smithay.

use smithay::backend::input::TouchSlot;
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Transform};
use smithay::wayland::seat::WaylandFocus;

use crate::backend::wayland::compositor::layer_shell::LayerFocusRequest;
use crate::backend::wayland::compositor::{
    PointerFocusTarget, TOUCH_POINTER_BUTTON_CODE, WaylandState,
};
use crate::backend::wayland::input::modifiers_to_x11_mask;
use crate::types::MouseButton;
use crate::wm::Wm;

/// Coordinate space used for an absolute touch device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchMappingTarget {
    /// Map normalized coordinates across the bounding rectangle of all active
    /// outputs.
    Layout,
    /// Map normalized coordinates to one named output.
    Output(String),
}

impl TouchMappingTarget {
    /// Interpret the configured output selector.
    pub fn configured(value: &str) -> Self {
        if value == "*" {
            Self::Layout
        } else {
            Self::Output(value.to_owned())
        }
    }
}

/// Backend-neutral absolute position in the inclusive `[0, 1]` range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedTouchPosition {
    x: f64,
    y: f64,
}

impl NormalizedTouchPosition {
    /// Validate and clamp coordinates supplied by an input backend.
    pub fn new(x: f64, y: f64) -> Option<Self> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some(Self {
            x: x.clamp(0.0, 1.0),
            y: y.clamp(0.0, 1.0),
        })
    }

    fn as_point(self) -> Point<f64, Logical> {
        Point::from((self.x, self.y))
    }
}

/// Backend-neutral data for touch-down and touch-motion events.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchPointEvent {
    pub slot: TouchSlot,
    pub position: NormalizedTouchPosition,
    pub time_msec: u32,
}

struct TouchHit {
    focus: Option<(PointerFocusTarget, Point<f64, Logical>)>,
    hovered_window: Option<crate::types::WindowId>,
    is_layer: bool,
}

/// Deliver a new touch point.
pub fn handle_touch_down(
    wm: &mut Wm,
    state: &mut WaylandState,
    event: TouchPointEvent,
    mapping: &TouchMappingTarget,
) {
    let Some(location) = event_location(state, event.position, mapping) else {
        log::warn!("dropping touch-down: mapping target {mapping:?} is not currently available");
        return;
    };

    state.runtime.cursor_hidden_by_touch = true;

    let serial = SERIAL_COUNTER.next_serial();
    let hit = focus_at(state, location);

    if !state.is_locked() {
        if hit.is_layer {
            if let Some((PointerFocusTarget::WlSurface(surface), _)) = hit.focus.as_ref() {
                state.focus_layer_keyboard(surface, serial, LayerFocusRequest::UserInteraction);
            }
        } else if !state.is_pointer_over_overlay(location)
            && can_claim_wm_gesture_slot(state.runtime.wm_gesture_touch_slot)
        {
            let root = root_point(location);
            state.dismiss_native_systray_menu();
            let modifiers = clean_modifier_state(state);
            let input = crate::mouse::press::PressInput {
                root,
                button: Some(MouseButton::Left),
                raw_button: MouseButton::Left.to_x11_detail(),
                modifiers,
                clicked_window: hit.hovered_window,
                source: crate::types::InteractionSource::Touch(event.slot.into()),
                time_msec: event.time_msec,
            };
            let outcome = {
                let mut ctx = wm.ctx();
                crate::mouse::press::dispatch_press_policy(&mut ctx, input)
            };
            match outcome {
                crate::mouse::press::PressOutcome::CapturedInteraction { .. }
                | crate::mouse::press::PressOutcome::Consumed => {
                    state.runtime.wm_gesture_touch_slot = Some(event.slot);
                    return;
                }
                crate::mouse::press::PressOutcome::SystrayIconPress {
                    index,
                    button,
                    root,
                } => {
                    let mut ctx = wm.ctx();
                    crate::systray::press_icon(ctx.core_mut(), index, button, root);
                    state.runtime.wm_gesture_touch_slot = Some(event.slot);
                    return;
                }
                crate::mouse::press::PressOutcome::ReplayToClient { .. } => {}
            }
        }
    }

    let emulate_pointer = hit
        .focus
        .as_ref()
        .is_some_and(|(target, _)| !supports_native_touch(state, target));
    let touch_was_grabbed = state.touch.is_grabbed();

    state.touch.clone().down(
        state,
        hit.focus.clone(),
        &DownEvent {
            slot: event.slot,
            location,
            serial,
            time: event.time_msec,
        },
    );
    if should_emulate_pointer(
        emulate_pointer,
        state.runtime.pointer_touch_slot.is_some(),
        touch_was_grabbed,
    ) {
        state.runtime.pointer_touch_slot = Some(event.slot);
        state.runtime.pointer_location = location;

        let pointer = state.pointer.clone();
        crate::backend::wayland::input::pointer::motion::dispatch_smithay_pointer_motion(
            state,
            &pointer,
            hit.focus,
            &smithay::input::pointer::MotionEvent {
                location,
                serial,
                time: event.time_msec,
            },
        );
        pointer.button(
            state,
            &smithay::input::pointer::ButtonEvent {
                button: TOUCH_POINTER_BUTTON_CODE,
                state: smithay::backend::input::ButtonState::Pressed,
                serial,
                time: event.time_msec,
            },
        );
        pointer.frame(state);
    }
}

/// Deliver movement for an existing touch point.
pub fn handle_touch_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    event: TouchPointEvent,
    mapping: &TouchMappingTarget,
) {
    let Some(location) = event_location(state, event.position, mapping) else {
        return;
    };
    if state.runtime.wm_gesture_touch_slot == Some(event.slot) {
        handle_wm_gesture_touch_motion(wm, state, event.slot, root_point(location));
        return;
    }
    if state.runtime.pointer_touch_slot == Some(event.slot) {
        state.runtime.pointer_location = location;
        let hit = focus_at(state, location);
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = state.pointer.clone();
        crate::backend::wayland::input::pointer::motion::dispatch_smithay_pointer_motion(
            state,
            &pointer,
            hit.focus.clone(),
            &smithay::input::pointer::MotionEvent {
                location,
                serial,
                time: event.time_msec,
            },
        );
        pointer.frame(state);
        state.touch.clone().motion(
            state,
            hit.focus,
            &MotionEvent {
                slot: event.slot,
                location,
                time: event.time_msec,
            },
        );
        return;
    }
    let hit = focus_at(state, location);
    state.touch.clone().motion(
        state,
        hit.focus,
        &MotionEvent {
            slot: event.slot,
            location,
            time: event.time_msec,
        },
    );
}

/// Deliver the end of a touch point.
pub fn handle_touch_up(wm: &mut Wm, state: &mut WaylandState, slot: TouchSlot, time_msec: u32) {
    if state.runtime.wm_gesture_touch_slot == Some(slot) {
        state.runtime.wm_gesture_touch_slot = None;
        finish_wm_gesture_touch(wm, state, slot, time_msec);
        return;
    }
    let serial = SERIAL_COUNTER.next_serial();
    if state.runtime.pointer_touch_slot == Some(slot) {
        state.runtime.pointer_touch_slot = None;
        let pointer = state.pointer.clone();
        pointer.button(
            state,
            &smithay::input::pointer::ButtonEvent {
                button: TOUCH_POINTER_BUTTON_CODE,
                state: smithay::backend::input::ButtonState::Released,
                serial,
                time: time_msec,
            },
        );
        pointer.frame(state);
    }
    state.touch.clone().up(
        state,
        &UpEvent {
            slot,
            serial,
            time: time_msec,
        },
    );
}

/// Finish a backend-provided touch frame.
pub fn handle_touch_frame(state: &mut WaylandState) {
    state.touch.clone().frame(state);
}

/// Cancel every active touch point.
pub fn handle_touch_cancel(wm: &mut Wm, state: &mut WaylandState) {
    if state.runtime.wm_gesture_touch_slot.take().is_some() {
        cancel_wm_gesture_touch(wm, state);
    }
    state.cancel_touch_pointer_emulation(0);
    state.touch.clone().cancel(state);
}

fn supports_native_touch(state: &WaylandState, target: &PointerFocusTarget) -> bool {
    let Some(surface) = target.wl_surface() else {
        return false;
    };
    let Some(client) = surface.client() else {
        return false;
    };
    state.touch.client_touch(&client).next().is_some()
}

fn should_emulate_pointer(
    client_needs_emulation: bool,
    pointer_touch_active: bool,
    touch_was_grabbed: bool,
) -> bool {
    client_needs_emulation && !pointer_touch_active && !touch_was_grabbed
}

/// Only one touch contact may own compositor interaction state. Additional
/// contacts remain native client touch points until that owner ends.
fn can_claim_wm_gesture_slot(active: Option<TouchSlot>) -> bool {
    active.is_none()
}

fn root_point(location: Point<f64, Logical>) -> crate::types::Point {
    crate::types::Point::from_f64_round(location.x, location.y)
}

fn clean_modifier_state(state: &WaylandState) -> u32 {
    crate::util::clean_mask(modifiers_to_x11_mask(&state.keyboard.modifier_state()), 0)
}

fn handle_wm_gesture_touch_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    slot: TouchSlot,
    root: crate::types::Point,
) {
    let mut ctx = wm.ctx();
    let _ = crate::mouse::interaction::handle(
        &mut ctx,
        crate::mouse::interaction::InteractionEvent {
            source: crate::mouse::interaction::InteractionSource::Touch(slot.into()),
            phase: crate::mouse::interaction::InteractionPhase::Update,
            root,
            modifiers: clean_modifier_state(state),
            sidebar_hover: None,
        },
    );
}

fn finish_wm_gesture_touch(wm: &mut Wm, state: &mut WaylandState, slot: TouchSlot, time_msec: u32) {
    let modifiers = clean_modifier_state(state);
    let mut ctx = wm.ctx();
    let _ = crate::mouse::interaction::handle(
        &mut ctx,
        crate::mouse::interaction::InteractionEvent {
            source: crate::mouse::interaction::InteractionSource::Touch(slot.into()),
            phase: crate::mouse::interaction::InteractionPhase::End {
                button: MouseButton::Left,
                time_msec,
            },
            root: Default::default(),
            modifiers,
            sidebar_hover: None,
        },
    );
}

fn cancel_wm_gesture_touch(wm: &mut Wm, _state: &mut WaylandState) {
    let mut ctx = wm.ctx();
    let _ = crate::mouse::interaction::handle(
        &mut ctx,
        crate::mouse::interaction::InteractionEvent {
            source: crate::mouse::interaction::InteractionSource::Touch(-1),
            phase: crate::mouse::interaction::InteractionPhase::Cancel {
                reason: crate::core_state::DragCancelReason::TouchCancelled,
            },
            root: Default::default(),
            modifiers: 0,
            sidebar_hover: None,
        },
    );
}

fn event_location(
    state: &WaylandState,
    position: NormalizedTouchPosition,
    mapping: &TouchMappingTarget,
) -> Option<Point<f64, Logical>> {
    let normalized = position.as_point();
    match mapping {
        TouchMappingTarget::Layout => {
            let bounds = active_layout_bounds(state)?;
            Some(map_normalized_to_layout(normalized, bounds))
        }
        TouchMappingTarget::Output(name) => {
            let output = state
                .space
                .outputs()
                .find(|output| output.name() == *name)?;
            let geometry = state.space.output_geometry(output)?;
            Some(map_normalized_to_output(
                normalized,
                geometry,
                output.current_transform(),
            ))
        }
    }
}

fn focus_at(state: &WaylandState, location: Point<f64, Logical>) -> TouchHit {
    if state.is_locked() {
        let focus = state
            .lock_surface_under_pointer(location)
            .map(|(surface, origin)| (PointerFocusTarget::WlSurface(surface), origin.to_f64()));
        return TouchHit {
            focus,
            hovered_window: None,
            is_layer: false,
        };
    }

    if let Some((surface, origin)) = state.layer_surface_under_pointer(location) {
        return TouchHit {
            focus: Some((PointerFocusTarget::WlSurface(surface), origin.to_f64())),
            hovered_window: None,
            is_layer: true,
        };
    }

    let contents = state.contents_under_pointer(location);
    let focus = contents
        .surface
        .map(|(surface, origin)| (PointerFocusTarget::WlSurface(surface), origin.to_f64()));
    TouchHit {
        focus,
        hovered_window: contents.hovered_win,
        is_layer: false,
    }
}

fn active_layout_bounds(state: &WaylandState) -> Option<Rectangle<i32, Logical>> {
    layout_bounds(
        state
            .space
            .outputs()
            .filter_map(|output| state.space.output_geometry(output)),
    )
}

fn layout_bounds(
    rectangles: impl IntoIterator<Item = Rectangle<i32, Logical>>,
) -> Option<Rectangle<i32, Logical>> {
    let mut rectangles = rectangles.into_iter();
    let first = rectangles.next()?;
    let (mut left, mut top) = (first.loc.x, first.loc.y);
    let (mut right, mut bottom) = (first.loc.x + first.size.w, first.loc.y + first.size.h);
    for rectangle in rectangles {
        left = left.min(rectangle.loc.x);
        top = top.min(rectangle.loc.y);
        right = right.max(rectangle.loc.x + rectangle.size.w);
        bottom = bottom.max(rectangle.loc.y + rectangle.size.h);
    }
    Some(Rectangle::new(
        (left, top).into(),
        (right - left, bottom - top).into(),
    ))
}

fn map_normalized_to_layout(
    normalized: Point<f64, Logical>,
    bounds: Rectangle<i32, Logical>,
) -> Point<f64, Logical> {
    Point::from((
        bounds.loc.x as f64 + normalized.x * bounds.size.w as f64,
        bounds.loc.y as f64 + normalized.y * bounds.size.h as f64,
    ))
}

fn map_normalized_to_output(
    normalized: Point<f64, Logical>,
    geometry: Rectangle<i32, Logical>,
    transform: Transform,
) -> Point<f64, Logical> {
    // Input coordinates are expressed in the output's untransformed space.
    // Convert that space to the transformed logical geometry advertised to
    // clients, then offset it into the global compositor layout.
    let untransformed_size = transform.invert().transform_size(geometry.size);
    let point = Point::from((
        normalized.x * untransformed_size.w as f64,
        normalized.y * untransformed_size.h as f64,
    ));
    transform.transform_point_in(point, &untransformed_size.to_f64()) + geometry.loc.to_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_mapping_distinguishes_layout_and_output() {
        assert_eq!(
            TouchMappingTarget::configured("*"),
            TouchMappingTarget::Layout
        );
        assert_eq!(
            TouchMappingTarget::configured("eDP-1"),
            TouchMappingTarget::Output("eDP-1".into())
        );
    }

    #[test]
    fn normalized_positions_reject_non_finite_values_and_clamp_edges() {
        assert_eq!(NormalizedTouchPosition::new(f64::NAN, 0.5), None);
        assert_eq!(NormalizedTouchPosition::new(0.5, f64::INFINITY), None);
        assert_eq!(
            NormalizedTouchPosition::new(-0.25, 1.25),
            Some(NormalizedTouchPosition { x: 0.0, y: 1.0 })
        );
    }

    #[test]
    fn pointer_fallback_is_only_used_for_unhandled_first_touch() {
        assert!(should_emulate_pointer(true, false, false));
        assert!(!should_emulate_pointer(false, false, false));
        assert!(!should_emulate_pointer(true, true, false));
        assert!(!should_emulate_pointer(true, false, true));
    }

    #[test]
    fn a_second_touch_cannot_replace_the_wm_gesture_owner() {
        assert!(can_claim_wm_gesture_slot(None));
        assert!(!can_claim_wm_gesture_slot(Some(Some(1).into())));
    }

    #[test]
    fn layout_bounds_include_negative_and_disjoint_outputs() {
        let bounds = layout_bounds([
            Rectangle::new((-1920, 100).into(), (1920, 1080).into()),
            Rectangle::new((200, -50).into(), (2560, 1440).into()),
        ])
        .unwrap();
        assert_eq!(bounds.loc, Point::from((-1920, -50)));
        assert_eq!(bounds.size, (4680, 1440).into());
    }

    #[test]
    fn layout_mapping_includes_layout_origin() {
        let bounds = Rectangle::new((-1000, 200).into(), (3000, 1000).into());
        assert_eq!(
            map_normalized_to_layout(Point::from((0.25, 0.75)), bounds),
            Point::from((-250.0, 950.0))
        );
    }

    #[test]
    fn output_mapping_applies_rotation_and_global_origin() {
        let geometry = Rectangle::new((100, 200).into(), (1080, 1920).into());
        assert_eq!(
            map_normalized_to_output(Point::from((0.25, 0.75)), geometry, Transform::_90,),
            Point::from((370.0, 680.0))
        );
    }

    #[test]
    fn output_mapping_without_transform_is_direct() {
        let geometry = Rectangle::new((-50, 20).into(), (1920, 1080).into());
        assert_eq!(
            map_normalized_to_output(Point::from((0.5, 0.25)), geometry, Transform::Normal,),
            Point::from((910.0, 290.0))
        );
    }

    #[test]
    fn every_output_transform_preserves_the_output_center() {
        let transforms = [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ];
        for transform in transforms {
            let geometry = Rectangle::new((40, -20).into(), (1200, 800).into());
            assert_eq!(
                map_normalized_to_output(Point::from((0.5, 0.5)), geometry, transform),
                Point::from((640.0, 380.0)),
                "wrong center for {transform:?}"
            );
        }
    }

    #[test]
    fn compositor_seat_advertises_native_touch() {
        let (_event_loop, state) = crate::backend::wayland::compositor::new_event_loop_and_state();
        assert!(state.seat.get_touch().is_some());
    }
}
