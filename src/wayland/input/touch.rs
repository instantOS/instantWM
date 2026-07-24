//! Backend-independent touchscreen event handling.
//!
//! Input backends provide normalized absolute coordinates. This module maps
//! them into compositor space, applies output transforms, resolves the target
//! surface, updates keyboard focus on touch-down, and emits native `wl_touch`
//! events through Smithay.

use smithay::backend::input::TouchSlot;
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Transform};

use crate::backend::wayland::compositor::layer_shell::LayerFocusRequest;
use crate::backend::wayland::compositor::{PointerFocusTarget, WaylandState};
use crate::types::MouseButton;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wayland::input::focus::focus_managed_target;
use crate::wm::Wm;

const TOUCH_BUTTON_CODE: u32 = 0x110;

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
        } else if !state.is_pointer_over_overlay(location) {
            let root = root_point(location);
            let bar_position = {
                let mut ctx = wm.ctx();
                crate::bar::resolve_bar_position_at_root(ctx.core_mut(), root, true)
                    .map(|(_, position)| position)
            };
            if state.runtime.bar_touch_slot.is_none()
                && let Some(position) = bar_position
            {
                state.runtime.bar_touch_slot = Some(event.slot);
                let modifiers = clean_modifier_state(state);
                crate::wayland::input::bar::handle_bar_click(
                    wm,
                    state,
                    position,
                    TOUCH_BUTTON_CODE,
                    root,
                    modifiers,
                );
                return;
            }
            focus_managed_target(wm, hit.hovered_window, Some(MouseButton::Left));
        }
    }

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

    if state.runtime.pointer_touch_slot.is_none() && !state.touch.has_grab(serial) {
        state.runtime.pointer_touch_slot = Some(event.slot);
        state.runtime.pointer_location = location;

        let pointer = state.pointer.clone();
        pointer.motion(
            state,
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
                button: TOUCH_BUTTON_CODE,
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
    if state.runtime.bar_touch_slot == Some(event.slot) {
        handle_bar_touch_motion(wm, state, root_point(location));
        return;
    }
    if state.runtime.pointer_touch_slot == Some(event.slot) {
        state.runtime.pointer_location = location;
        let hit = focus_at(state, location);
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = state.pointer.clone();
        pointer.motion(
            state,
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
    if state.runtime.bar_touch_slot == Some(slot) {
        state.runtime.bar_touch_slot = None;
        finish_bar_touch(wm, state);
        return;
    }
    let serial = SERIAL_COUNTER.next_serial();
    if state.runtime.pointer_touch_slot == Some(slot) {
        state.runtime.pointer_touch_slot = None;
        let pointer = state.pointer.clone();
        pointer.button(
            state,
            &smithay::input::pointer::ButtonEvent {
                button: TOUCH_BUTTON_CODE,
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
    if state.runtime.bar_touch_slot.take().is_some() {
        cancel_bar_touch(wm, state);
    }
    if state.runtime.pointer_touch_slot.take().is_some() {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = state.pointer.clone();
        pointer.button(
            state,
            &smithay::input::pointer::ButtonEvent {
                button: TOUCH_BUTTON_CODE,
                state: smithay::backend::input::ButtonState::Released,
                serial,
                time: 0,
            },
        );
        pointer.frame(state);
    }
    state.touch.clone().cancel(state);
}

fn root_point(location: Point<f64, Logical>) -> crate::types::Point {
    crate::types::Point::from_f64_round(location.x, location.y)
}

fn clean_modifier_state(state: &WaylandState) -> u32 {
    crate::util::clean_mask(modifiers_to_x11_mask(&state.keyboard.modifier_state()), 0)
}

fn handle_bar_touch_motion(wm: &mut Wm, state: &mut WaylandState, root: crate::types::Point) {
    let mut ctx = wm.ctx();
    if ctx.core().drag_state().tag.active && !crate::mouse::drag_tag_motion(&mut ctx, root) {
        crate::mouse::drag_tag_finish(&mut ctx, clean_modifier_state(state));
    }
    if ctx.core().drag_state().armed_interaction().is_some() {
        crate::mouse::drag::title::title_drag_motion_at(&mut ctx, root, true);
    }
    if let crate::contexts::WmCtx::Wayland(ref mut wayland) = ctx
        && wayland.core.drag_state().active_interaction().is_some()
    {
        crate::wayland::input::pointer::drag::hover_resize_drag_motion(wayland, root);
    }
    if ctx.core().drag_state().sidebar_volume_active() {
        crate::mouse::update_sidebar_gesture(&mut ctx, root.y);
    }
}

fn finish_bar_touch(wm: &mut Wm, state: &mut WaylandState) {
    let modifiers = clean_modifier_state(state);
    let mut ctx = wm.ctx();

    if let crate::contexts::WmCtx::Wayland(ref mut wayland) = ctx
        && crate::wayland::input::pointer::drag::hover_resize_drag_finish(
            wayland,
            MouseButton::Left,
            modifiers,
        )
    {
        return;
    }
    if ctx.core().drag_state().tag.active {
        crate::mouse::drag_tag_finish(&mut ctx, modifiers);
    }
    if ctx.core().drag_state().armed_interaction().is_some() {
        crate::mouse::title_drag_finish(&mut ctx);
    }
    if ctx.core().drag_state().sidebar_volume_active() {
        let _ = crate::mouse::finish_sidebar_gesture(&mut ctx, MouseButton::Left);
    }
}

fn cancel_bar_touch(wm: &mut Wm, _state: &mut WaylandState) {
    let mut ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(ref mut wayland) = ctx {
        crate::mouse::drag::lifecycle::cancel(
            wayland.core.drag_state_mut(),
            wayland.wayland,
            crate::core_state::DragCancelReason::TouchCancelled,
        );
    }
    ctx.core_mut().drag_state_mut().tag = Default::default();
    ctx.core_mut().drag_state_mut().cancel_sidebar_volume();
    ctx.core_mut().bar.hover.clear();
    ctx.set_cursor_style(crate::types::AltCursor::Default);
    ctx.request_bar_update();
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
                .find(|output| output.name() == *name && output_is_active(state, output))?;
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

fn output_is_active(state: &WaylandState, output: &smithay::output::Output) -> bool {
    state
        .runtime
        .output_enabled
        .get(&output.name())
        .copied()
        .unwrap_or(true)
}

fn active_layout_bounds(state: &WaylandState) -> Option<Rectangle<i32, Logical>> {
    layout_bounds(
        state
            .space
            .outputs()
            .filter(|output| output_is_active(state, output))
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
        let (_event_loop, state) =
            crate::wayland::runtime::common::new_wayland_event_loop_and_state();
        assert!(state.seat.get_touch().is_some());
    }
}
