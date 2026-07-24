//! Pointer motion handling.

use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::wayland::commands::PointerMotionCommand;
use crate::backend::wayland::compositor::window::hit_test::{PointerContents, SurfaceFocus};
use crate::backend::wayland::compositor::{PointerFocusTarget, WaylandState};
use crate::contexts::{WmCtx, WmCtxWayland};
use crate::mouse::{clear_hover_offer, update_selected_resize_offer_at, update_sidebar_offer_at};
use crate::types::BarPosition;
use crate::types::Point as RootPoint;
use crate::types::Rect;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wayland::input::bar::update_bar_hit_state;
use crate::wayland::input::pointer::constraints::{ActivePointerConstraint, activate_under};
use crate::wayland::input::pointer::drag::{active_drag_window, hover_resize_drag_motion};
use crate::wm::Wm;

fn monitor_bar_visible(wm: &Wm, mon: &crate::types::Monitor) -> bool {
    mon.bar_visible(&wm.core.model.clients)
}

/// Unified pointer motion event that abstracts over input source.
#[derive(Debug, Clone, Copy)]
pub enum MotionEvent {
    /// Absolute position (winit backend, tablets, touch screens)
    Absolute { x: f64, y: f64, time_msec: u32 },
    /// Relative delta (libinput mouse)
    Relative {
        dx: f64,
        dy: f64,
        dx_unaccel: f64,
        dy_unaccel: f64,
        time_msec: u32,
        time_usec: u64,
    },
}

impl MotionEvent {
    /// Compute the new pointer location from the current position.
    pub fn compute_location(
        &self,
        current: Point<f64, smithay::utils::Logical>,
        output_width: i32,
        output_height: i32,
    ) -> Point<f64, smithay::utils::Logical> {
        let max_x = output_width.max(0) as f64;
        let max_y = output_height.max(0) as f64;
        match self {
            MotionEvent::Absolute { x, y, .. } => {
                Point::from((x.clamp(0.0, max_x), y.clamp(0.0, max_y)))
            }
            MotionEvent::Relative { dx, dy, .. } => {
                let x = (current.x + dx).clamp(0.0, max_x);
                let y = (current.y + dy).clamp(0.0, max_y);
                Point::from((x, y))
            }
        }
    }

    /// Get the event timestamp.
    pub fn time_msec(&self) -> u32 {
        match self {
            MotionEvent::Absolute { time_msec, .. } => *time_msec,
            MotionEvent::Relative { time_msec, .. } => *time_msec,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MotionEvent, PointerMotionSource};
    use crate::types::HoverFocusTrigger;
    use smithay::utils::Point;

    #[test]
    fn relative_motion_reaches_output_right_and_bottom_edges() {
        let event = MotionEvent::Relative {
            dx: 100.0,
            dy: 100.0,
            dx_unaccel: 100.0,
            dy_unaccel: 100.0,
            time_msec: 0,
            time_usec: 0,
        };

        assert_eq!(
            event.compute_location(Point::from((1910.0, 1070.0)), 1920, 1080),
            Point::from((1920.0, 1080.0))
        );
    }

    #[test]
    fn absolute_motion_reaches_output_right_and_bottom_edges() {
        let event = MotionEvent::Absolute {
            x: 1920.0,
            y: 1080.0,
            time_msec: 0,
        };

        assert_eq!(
            event.compute_location(Point::from((0.0, 0.0)), 1920, 1080),
            Point::from((1920.0, 1080.0))
        );
    }

    #[test]
    fn motion_sources_preserve_why_focus_was_recomputed() {
        assert_eq!(
            PointerMotionSource::Device.hover_focus_trigger(),
            HoverFocusTrigger::PointerMotion
        );
        assert_eq!(
            PointerMotionSource::Synthetic.hover_focus_trigger(),
            HoverFocusTrigger::SceneChange
        );
    }
}

/// Process a queued backend pointer command through the single Wayland pointer
/// transaction path.
pub fn process_pointer_motion_command(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    command: PointerMotionCommand,
) {
    match command {
        PointerMotionCommand::Relative {
            dx,
            dy,
            dx_unaccel,
            dy_unaccel,
            time_msec,
            time_usec,
        } => handle_pointer_motion(
            wm,
            state,
            pointer_handle,
            keyboard_handle,
            MotionEvent::Relative {
                dx,
                dy,
                dx_unaccel,
                dy_unaccel,
                time_msec,
                time_usec,
            },
            PointerMotionSource::Device,
        ),
        PointerMotionCommand::Absolute { x, y, time_msec } => {
            handle_pointer_motion(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                MotionEvent::Absolute { x, y, time_msec },
                PointerMotionSource::Device,
            );
        }
        PointerMotionCommand::Warp { x, y, time_msec } => {
            handle_pointer_motion(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                MotionEvent::Absolute { x, y, time_msec },
                PointerMotionSource::Synthetic,
            );
        }
        PointerMotionCommand::Refresh { time_msec } => {
            let location = state.runtime.pointer_location;
            handle_pointer_motion(
                wm,
                state,
                pointer_handle,
                keyboard_handle,
                MotionEvent::Absolute {
                    x: location.x,
                    y: location.y,
                    time_msec,
                },
                PointerMotionSource::Synthetic,
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerMotionSource {
    Device,
    Synthetic,
}

impl PointerMotionSource {
    fn hover_focus_trigger(self) -> crate::types::HoverFocusTrigger {
        match self {
            Self::Device => crate::types::HoverFocusTrigger::PointerMotion,
            Self::Synthetic => crate::types::HoverFocusTrigger::SceneChange,
        }
    }
}

fn handle_pointer_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: MotionEvent,
    source: PointerMotionSource,
) {
    state.runtime.cursor_hidden_by_touch = false;

    let output_width = wm.core.config.derived.display.width;
    let output_height = wm.core.config.derived.display.height;

    let current_location = state.runtime.pointer_location;

    let potential_location = event.compute_location(current_location, output_width, output_height);

    let current_hit = state.contents_under_pointer(current_location);
    let constraint = ActivePointerConstraint::under(
        pointer_handle,
        current_hit.surface.as_ref(),
        current_location,
    );

    // Always emit relative motion if this is a relative event. Locked pointers
    // consume only this relative stream and do not advance absolute location.
    if let MotionEvent::Relative {
        dx,
        dy,
        dx_unaccel,
        dy_unaccel,
        time_msec: _,
        time_usec,
    } = event
    {
        let rel_event = smithay::input::pointer::RelativeMotionEvent {
            delta: (dx, dy).into(),
            delta_unaccel: (dx_unaccel, dy_unaccel).into(),
            utime: time_usec,
        };
        let focus = current_hit
            .surface
            .as_ref()
            .map(|(s, loc)| (PointerFocusTarget::WlSurface(s.clone()), loc.to_f64()));
        pointer_handle.relative_motion(state, focus, &rel_event);
    }

    if constraint.is_locked() {
        pointer_handle.frame(state);
        return;
    }

    let final_location = potential_location;
    let candidate_hit = state.contents_under_pointer(final_location);

    if !constraint.allows_motion_to(candidate_hit.surface.as_ref(), final_location) {
        pointer_handle.frame(state);
        return;
    }

    state.runtime.pointer_location = final_location;

    // Hot corners are compositor-owned pointer interactions. Evaluate them
    // before the final hit test so this motion is dispatched against the
    // newly shown or hidden overlay. Session-locked input must never trigger
    // WM UI.
    if source == PointerMotionSource::Device && !state.is_locked() {
        let root = RootPoint::from_f64_round(final_location.x, final_location.y);
        let mut ctx = wm.ctx();
        crate::mouse::update_overlay_hot_corner(&mut ctx, root);
    }

    let final_hit = state.contents_under_pointer(final_location);

    // Activate any pending constraints BEFORE dispatch so they're active for this event
    activate_under(pointer_handle, final_hit.surface.as_ref(), final_location);

    dispatch_pointer_motion(
        wm,
        state,
        pointer_handle,
        keyboard_handle,
        final_hit,
        event.time_msec(),
        source.hover_focus_trigger(),
    );
}

/// Unified pointer motion: update WM hover focus, propagate to clients, handle drags.
fn dispatch_pointer_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    hit_test: PointerContents,
    time_msec: u32,
    hover_focus_trigger: crate::types::HoverFocusTrigger,
) {
    let pointer_location = state.runtime.pointer_location;
    let root = RootPoint::from_f64_round(pointer_location.x, pointer_location.y);

    // Get active drag window once - used in multiple phases
    let active_drag_window = active_drag_window(wm);

    // Phase 1: Compute bar/guard band hit detection
    let (in_bar_band, in_bar_guard_band) = compute_bar_hit(wm, root);

    // Phase 2: Resolve pointer focus and hovered window
    let (pointer_focus, hovered_win) =
        resolve_pointer_focus_from_hit(state, hit_test, in_bar_band, in_bar_guard_band);

    // Phase 3: Handle resize drag motion (early return path)
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx
        && handle_resize_drag_motion(
            &mut ctx,
            state,
            pointer_handle,
            pointer_focus.clone(),
            time_msec,
        )
    {
        return;
    }

    // Phase 4: Handle bar interaction (early return path)
    // An armed/active tag drag owns the bar hover until release. Running the
    // ordinary hover path as well makes the two states alternate every motion
    // frame, which is visible as flicker on Wayland.
    let bar_pos = if wm.core.drag.tag.active {
        None
    } else {
        update_bar_hit_state(wm, root, false)
    };
    if handle_bar_motion(
        wm,
        state,
        pointer_handle,
        pointer_focus.clone(),
        in_bar_band,
        bar_pos,
        time_msec,
    ) {
        return;
    }

    // Cheap shared sidebar hover path: monitor lookup + rectangle test, no
    // client scans and no button binding dispatch on motion.
    // Only check when no window is under the cursor — a window covering the
    // sidebar area must receive events normally.
    if !wm.core.drag.any_drag_active() {
        if hovered_win.is_none() {
            let ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx
                && update_sidebar_offer_at(&mut WmCtx::Wayland(ctx.reborrow()), root)
                    .affects_pointer_handling()
            {
                return;
            }
        } else if wm.core.drag.hover_offer.is_sidebar() {
            let ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
                clear_hover_offer(&mut WmCtx::Wayland(ctx.reborrow()));
            }
        }
    }

    // Phase 5: Update hover resize state for floating windows
    let suppress_hover_focus =
        update_hover_resize_state(wm, root, hovered_win, !wm.core.drag.any_drag_active());

    // Phase 6: Update pointer focus based on drag state. An exclusive layer
    // surface (for example slurp) temporarily owns keyboard focus; moving the
    // pointer while it is active must not select/reorder managed windows below
    // the overlay.
    if !state.exclusive_layer_has_keyboard_focus() {
        update_pointer_focus(
            wm,
            active_drag_window,
            hovered_win,
            suppress_hover_focus,
            root,
            hover_focus_trigger,
        );
    }

    // Phase 7: Handle tag/title drag motion
    if hover_focus_trigger == crate::types::HoverFocusTrigger::PointerMotion {
        handle_wm_drag_motion(wm, keyboard_handle, root);
    }

    // Phase 8: Dispatch final motion event to Smithay
    let focus =
        pointer_focus.map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));

    let serial = SERIAL_COUNTER.next_serial();
    let motion = smithay::input::pointer::MotionEvent {
        location: pointer_location,
        serial,
        time: time_msec,
    };
    pointer_handle.motion(state, focus, &motion);
    pointer_handle.frame(state);
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions for dispatch_pointer_motion
// ─────────────────────────────────────────────────────────────────────────────

/// Compute whether the pointer is in the bar area or guard band below it.
fn compute_bar_hit(wm: &Wm, root: RootPoint) -> (bool, bool) {
    wm.core
        .model
        .monitors
        .id_intersecting_rect(Rect {
            x: root.x,
            y: root.y,
            w: 1,
            h: 1,
        })
        .and_then(|mid| wm.core.monitor(mid))
        .map(|mon| {
            let bar_visible = monitor_bar_visible(wm, mon);
            let in_bar = bar_visible && mon.y_in_bar(root.y);
            let in_guard =
                bar_visible && !wm.core.drag.any_drag_active() && mon.y_in_guard_band(root.y);
            (in_bar, in_guard)
        })
        .unwrap_or((false, false))
}

/// Resolve pointer focus and hovered window based on bar hit state.
///
/// Uses a single-pass hit test (`contents_under_pointer`) to avoid repeated
/// window-list allocations and layer-surface scans on every motion event.
fn resolve_pointer_focus_from_hit(
    state: &WaylandState,
    contents: PointerContents,
    in_bar_band: bool,
    in_bar_guard_band: bool,
) -> (Option<SurfaceFocus>, Option<crate::types::WindowId>) {
    let pointer_location = state.runtime.pointer_location;

    // When the session is locked, only the lock surface should receive pointer events.
    if state.is_locked() {
        let pointer_focus = state.lock_surface_under_pointer(pointer_location);
        return (pointer_focus, None);
    }

    // In the bar or guard band, only layer surfaces matter (no window hit testing).
    if in_bar_band || in_bar_guard_band {
        let pointer_focus = state.layer_surface_under_pointer(pointer_location);
        return (pointer_focus, None);
    }

    (contents.surface, contents.hovered_win)
}

/// Handle resize drag motion. Returns true if handled (early return).
fn handle_resize_drag_motion(
    ctx: &mut WmCtxWayland<'_>,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    pointer_focus: Option<SurfaceFocus>,
    time_msec: u32,
) -> bool {
    let pointer_location = state.runtime.pointer_location;
    if !hover_resize_drag_motion(
        ctx,
        RootPoint::from_f64_round(pointer_location.x, pointer_location.y),
    ) {
        return false;
    }

    // During an active resize drag, forward motion to Smithay to keep
    // the pointer protocol in sync, but skip focus updates.
    let serial = SERIAL_COUNTER.next_serial();
    let motion = smithay::input::pointer::MotionEvent {
        location: pointer_location,
        serial,
        time: time_msec,
    };
    let focus =
        pointer_focus.map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));
    pointer_handle.motion(state, focus, &motion);
    pointer_handle.frame(state);
    true
}

/// Handle bar motion. Returns true if handled (early return).
fn handle_bar_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    pointer_focus: Option<SurfaceFocus>,
    in_bar_band: bool,
    bar_pos: Option<BarPosition>,
    time_msec: u32,
) -> bool {
    let pointer_location = state.runtime.pointer_location;
    let is_drag = wm.core.drag.any_drag_active();
    if (in_bar_band || bar_pos.is_some()) && !is_drag {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
            return true;
        };
        clear_hover_offer(&mut WmCtx::Wayland(ctx.reborrow()));
        let focus = pointer_focus
            .map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));
        let serial = SERIAL_COUNTER.next_serial();
        let motion = smithay::input::pointer::MotionEvent {
            location: pointer_location,
            serial,
            time: time_msec,
        };
        pointer_handle.motion(state, focus, &motion);
        pointer_handle.frame(state);
        return true;
    }
    false
}

/// Update hover resize state for floating windows.
/// Returns whether to suppress hover focus.
fn update_hover_resize_state(
    wm: &mut Wm,
    root: RootPoint,
    hovered_win: Option<crate::types::WindowId>,
    no_active_drag: bool,
) -> bool {
    if wm.core.model.is_overview_active() {
        let mut ctx = wm.ctx();
        clear_hover_offer(&mut ctx);
        return false;
    }
    if !no_active_drag {
        return false;
    }

    let selected_floating = wm
        .core
        .selected_win()
        .and_then(|win| {
            wm.core
                .model
                .client(win)
                .map(|c| (win, c.mode().is_floating()))
        })
        .is_some_and(|(_, is_floating)| is_floating);
    let hovered_is_selected = hovered_win.is_some_and(|win| Some(win) == wm.core.selected_win());

    let ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
        return false;
    };

    if !selected_floating {
        let _ = update_selected_resize_offer_at(&mut WmCtx::Wayland(ctx.reborrow()), root);
        return false;
    }

    let mut suppress_hover_focus = !hovered_is_selected;
    let selected_offer =
        update_selected_resize_offer_at(&mut WmCtx::Wayland(ctx.reborrow()), root).is_some();
    if selected_offer {
        suppress_hover_focus = true;
    } else if !hovered_is_selected {
        suppress_hover_focus = false;
    }

    suppress_hover_focus
}

/// Update pointer focus based on drag state.
fn update_pointer_focus(
    wm: &mut Wm,
    active_drag_window: Option<crate::types::WindowId>,
    hovered_win: Option<crate::types::WindowId>,
    suppress_hover_focus: bool,
    root: RootPoint,
    trigger: crate::types::HoverFocusTrigger,
) {
    if wm.core.model.is_overview_active() {
        let mut ctx = wm.ctx();
        crate::focus::apply_hover_focus(&mut ctx, hovered_win, false, Some(root), trigger);
        return;
    }
    if let Some(lock_win) = active_drag_window {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
            return;
        };
        if ctx.core.model().selected_win() != Some(lock_win) {
            crate::focus::focus(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                Some(lock_win),
            );
        }
    } else if !suppress_hover_focus {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(ctx) = ctx else {
            return;
        };
        let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx);
        crate::focus::apply_hover_focus(&mut wm_ctx, hovered_win, false, Some(root), trigger);
    }
}

/// Handle tag and title drag motion.
fn handle_wm_drag_motion(
    wm: &mut Wm,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    root: RootPoint,
) {
    let mut ctx = wm.ctx();
    if ctx.core().drag_state().tag.active && !crate::mouse::drag_tag_motion(&mut ctx, root) {
        let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
        crate::mouse::drag_tag_finish(&mut ctx, mod_state);
    }
    if ctx.core().drag_state().armed_interaction().is_some() {
        crate::mouse::title_drag_motion(&mut ctx, root);
    }
    if ctx.core().drag_state().sidebar_volume_active() {
        crate::mouse::update_sidebar_gesture(&mut ctx, root.y);
    }
}
