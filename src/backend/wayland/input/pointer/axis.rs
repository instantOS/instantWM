//! Pointer axis (scroll) handling.

use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::Point;

use crate::backend::wayland::commands::PointerAxisCommand;
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::input::modifiers_to_x11_mask;
use crate::types::Point as RootPoint;
use crate::util::clean_mask;
use crate::wm::Wm;

use crate::backend::wayland::input::bar::{handle_bar_scroll, update_bar_hit_state};

/// Resolve the effective scroll factor from input configuration.
///
/// Checks `type:pointer`, `type:touchpad`, then `*` (wildcard) entries,
/// returning the first `scroll_factor` found, or `1.0` if none is set.
fn resolve_scroll_factor(
    input_config: &std::collections::HashMap<String, crate::config::config_toml::InputConfig>,
) -> f64 {
    for key in &["type:pointer", "type:touchpad", "*"] {
        if let Some(cfg) = input_config.get(*key)
            && let Some(factor) = cfg.scroll_factor
        {
            return factor.max(0.0);
        }
    }
    1.0
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PointerAxisInput {
    pub event: PointerAxisCommand,
    pub location: Point<f64, smithay::utils::Logical>,
}

pub(crate) fn handle_pointer_axis(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer: &PointerHandle<WaylandState>,
    keyboard: &KeyboardHandle<WaylandState>,
    input: PointerAxisInput,
) {
    let scroll_factor = resolve_scroll_factor(&wm.core.config.input);

    let root = RootPoint::from_f64_round(input.location.x, input.location.y);

    // Check if the pointer is in the bar area; if so, dispatch bar scroll.
    // A captured gesture that owns the bar hover suppresses ordinary hover
    // updates here as well: scrolling mid-drag would race the gesture's
    // highlight state exactly like the motion path does.
    let scroll_delta = input.event.vertical.v120.or(input.event.vertical.amount);
    let bar_pos = if wm.core.interaction.drag.owns_bar_hover() {
        None
    } else {
        update_bar_hit_state(wm, root, true)
    };
    if let Some(delta) = scroll_delta.filter(|d| *d != 0.0)
        && let Some(pos) = bar_pos
    {
        let clean_state = clean_mask(modifiers_to_x11_mask(&keyboard.modifier_state()), 0);
        handle_bar_scroll(wm, pos, delta, root, clean_state);
    }

    let mut frame =
        smithay::input::pointer::AxisFrame::new(input.event.time_msec).source(input.event.source);
    let mut has_axis_content = false;

    for (axis, axis_input) in [
        (
            smithay::backend::input::Axis::Horizontal,
            input.event.horizontal,
        ),
        (
            smithay::backend::input::Axis::Vertical,
            input.event.vertical,
        ),
    ] {
        if let Some(amount) = axis_input.amount {
            if amount.abs() >= f64::EPSILON {
                frame = frame.relative_direction(axis, axis_input.relative_direction);
                frame = frame.value(axis, amount * scroll_factor);
                has_axis_content = true;
                if let Some(steps) = axis_input.v120 {
                    frame = frame.v120(axis, (steps * scroll_factor) as i32);
                }
            } else if matches!(
                input.event.source,
                smithay::backend::input::AxisSource::Finger
            ) {
                // Finger scrolling must send axis_stop when libinput ends the sequence.
                frame = frame.stop(axis);
                has_axis_content = true;
            }
        }
    }

    if has_axis_content {
        pointer.axis(state, frame);
        pointer.frame(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::wayland::commands::{PointerAxis, PointerAxisCommand};
    use crate::types::{
        Client, ClientMode, Gesture, Monitor, MonitorId, Point, Rect, TagMask, WindowId,
    };

    /// A monitor with a visible bar and two tiled clients whose titles the
    /// strip presents in order `[first, second]`.
    fn wm_with_title_strip() -> (crate::wm::Wm, MonitorId, WindowId, WindowId) {
        use crate::backend::{Backend, wayland::WaylandBackend};

        let mut wm = crate::wm::Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 9;
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            bar_height: 30,
            show_bar: true,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let windows = [WindowId(41), WindowId(42)];
        for win in windows {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags,
                mode: ClientMode::tiled(),
                geo: Rect::new(0, 30, 600, 770),
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = windows.to_vec();
        (wm, monitor_id, windows[0], windows[1])
    }

    fn wheel_scroll_at(root: Point) -> PointerAxisInput {
        let axis = || PointerAxis {
            amount: None,
            v120: None,
            relative_direction: smithay::backend::input::AxisRelativeDirection::Identical,
        };
        PointerAxisInput {
            event: PointerAxisCommand {
                source: smithay::backend::input::AxisSource::Wheel,
                horizontal: axis(),
                vertical: PointerAxis {
                    amount: Some(-120.0),
                    v120: Some(-120.0),
                    ..axis()
                },
                time_msec: 0,
            },
            location: smithay::utils::Point::from((root.x as f64, root.y as f64)),
        }
    }

    /// Root-space center x of `win`'s title cell, scanned through the shared
    /// hit-test so the test cannot drift from the renderer's layout.
    fn title_cell_center(wm: &mut crate::wm::Wm, win: WindowId) -> i32 {
        let mut span: Option<(i32, i32)> = None;
        let mut core = crate::contexts::CoreCtx::new(
            &mut wm.core,
            &mut wm.work,
            &mut wm.running,
            &mut wm.bar,
            &mut wm.focus,
        );
        for x in 0..1200 {
            if let Some((_, crate::types::BarPosition::WinTitle(hit))) =
                crate::bar::resolve_bar_position_at_root(&mut core, Point::new(x, 10))
                && hit == win
            {
                match span {
                    Some((start, _)) => span = Some((start, x)),
                    None => span = Some((x, x)),
                }
            }
        }
        let (start, end) = span.expect("window must own a title cell");
        (start + end + 1) / 2
    }

    /// Scrolling while a captured gesture owns the bar hover must not run the
    /// ordinary hover update: the wheel would race the gesture's highlight
    /// state exactly like pointer motion did before the suppression.
    #[test]
    fn scroll_during_a_captured_gesture_leaves_bar_hover_untouched() {
        let (mut wm, monitor_id, _first, second) = wm_with_title_strip();
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let (Some(pointer), Some(keyboard)) = (state.seat.get_pointer(), state.seat.get_keyboard())
        else {
            panic!("test seat must provide pointer and keyboard handles");
        };

        let scroll_root = Point::new(title_cell_center(&mut wm, second), 10);

        // An active tag drag owns the bar hover for its whole capture.
        wm.core
            .interaction
            .drag
            .begin_tag_drag(crate::core_state::TagDragState {
                initial_tag: TagMask::single(1).unwrap(),
                start: scroll_root,
                dragging: true,
                monitor_id,
                last_tag: Some(1),
                cursor_on_bar: true,
                last_motion: None,
                button: crate::types::MouseButton::Left,
                source: crate::types::InteractionSource::Pointer,
            })
            .unwrap();
        // The gesture's own highlight, as maintained by the drag path.
        wm.bar.hover.set(monitor_id, Gesture::Tag(0), true);

        handle_pointer_axis(
            &mut wm,
            &mut state,
            &pointer,
            &keyboard,
            wheel_scroll_at(scroll_root),
        );
        assert_eq!(
            (wm.bar.hover.monitor_id, wm.bar.hover.gesture),
            (Some(monitor_id), Gesture::Tag(0)),
            "scroll during a hover-owning capture must not touch bar hover"
        );

        // Control: without the capture the same scroll runs the ordinary hover
        // update and highlights the scrolled-over title.
        assert!(
            wm.core
                .interaction
                .drag
                .finish_tag_drag(crate::types::MouseButton::Left)
                .is_some()
        );
        handle_pointer_axis(
            &mut wm,
            &mut state,
            &pointer,
            &keyboard,
            wheel_scroll_at(scroll_root),
        );
        assert_eq!(
            (wm.bar.hover.monitor_id, wm.bar.hover.gesture),
            (Some(monitor_id), Gesture::WinTitle(second))
        );
    }
}
