//! Backend-independent touchscreen event handling.
//!
//! Input backends provide normalized absolute coordinates. This module maps
//! them into compositor space, applies output transforms, resolves the target
//! surface, updates keyboard focus on touch-down, and emits native `wl_touch`
//! events through Smithay.

use smithay::backend::input::TouchSlot;
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Transform};

use crate::backend::wayland::compositor::{KeyboardFocusTarget, PointerFocusTarget, WaylandState};
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

    let serial = SERIAL_COUNTER.next_serial();
    let hit = focus_at(state, location);

    if !state.is_locked() {
        if hit.is_layer {
            if let Some((PointerFocusTarget::WlSurface(surface), _)) = hit.focus.as_ref()
                && crate::backend::wayland::compositor::layer_shell::layer_surface_accepts_keyboard_focus(
                    surface,
                )
            {
                state.keyboard.clone().set_focus(
                    state,
                    Some(KeyboardFocusTarget::WlSurface(surface.clone())),
                    serial,
                );
            }
        } else if !state.is_pointer_over_overlay(location) {
            crate::wayland::input::pointer::button::focus_button_target(
                wm,
                hit.hovered_window,
                Some(MouseButton::Left),
            );
        }
    }

    state.touch.clone().down(
        state,
        hit.focus,
        &DownEvent {
            slot: event.slot,
            location,
            serial,
            time: event.time_msec,
        },
    );
}

/// Deliver movement for an existing touch point.
pub fn handle_touch_motion(
    state: &mut WaylandState,
    event: TouchPointEvent,
    mapping: &TouchMappingTarget,
) {
    let Some(location) = event_location(state, event.position, mapping) else {
        return;
    };
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
pub fn handle_touch_up(state: &mut WaylandState, slot: TouchSlot, time_msec: u32) {
    state.touch.clone().up(
        state,
        &UpEvent {
            slot,
            serial: SERIAL_COUNTER.next_serial(),
            time: time_msec,
        },
    );
}

/// Finish a backend-provided touch frame.
pub fn handle_touch_frame(state: &mut WaylandState) {
    state.touch.clone().frame(state);
}

/// Cancel every active touch point.
pub fn handle_touch_cancel(state: &mut WaylandState) {
    state.touch.clone().cancel(state);
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
