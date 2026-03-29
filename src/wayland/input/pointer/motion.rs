#![allow(clippy::type_complexity)]
//! Pointer motion handling.

use smithay::backend::input::{AbsolutePositionEvent, InputBackend, PointerMotionEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::wayland::compositor::{PointerFocusTarget, WaylandState};
use crate::contexts::WmCtxWayland;
use crate::mouse::hover::selected_hover_resize_target_at;
use crate::mouse::set_cursor_style;
use crate::types::AltCursor;
use crate::types::BarPosition;
use crate::types::Rect;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wayland::input::bar::update_wayland_bar_hit_state;
use crate::wayland::input::pointer::drag::{
    update_wayland_selected_resize_offer, wayland_active_drag_window,
    wayland_hover_resize_drag_motion,
};
use crate::wm::Wm;

fn wayland_monitor_bar_visible(wm: &Wm, mon: &crate::types::Monitor) -> bool {
    if !mon.shows_bar() {
        return false;
    }
    let selected_tags = mon.selected_tags();
    !mon.fullscreen
        .and_then(|win| wm.g.clients.get(&win))
        .is_some_and(|client| client.is_true_fullscreen() && client.is_visible(selected_tags))
}

/// Unified pointer motion event that abstracts over input source.
#[derive(Debug, Clone, Copy)]
pub enum MotionEvent {
    /// Absolute position (winit backend, tablets, touch screens)
    Absolute { x: f64, y: f64, time_msec: u32 },
    /// Relative delta (libinput mouse)
    Relative { dx: f64, dy: f64, time_msec: u32 },
}

impl MotionEvent {
    /// Compute the new pointer location from the current position.
    pub fn compute_location(
        &self,
        current: Point<f64, smithay::utils::Logical>,
        output_width: i32,
        output_height: i32,
    ) -> Point<f64, smithay::utils::Logical> {
        match self {
            MotionEvent::Absolute { x, y, .. } => Point::from((*x, *y)),
            MotionEvent::Relative { dx, dy, .. } => {
                let x = (current.x + dx).clamp(0.0, output_width as f64);
                let y = (current.y + dy).clamp(0.0, output_height as f64);
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

/// Construct a `MotionEvent` from a libinput relative motion event.
pub fn motion_event_from_libinput_relative<B: InputBackend>(
    event: impl PointerMotionEvent<B>,
) -> MotionEvent {
    MotionEvent::Relative {
        dx: event.delta_x(),
        dy: event.delta_y(),
        time_msec: event.time_msec(),
    }
}

/// Construct a `MotionEvent` from a libinput absolute motion event.
pub fn motion_event_from_libinput_absolute<B: InputBackend>(
    event: impl AbsolutePositionEvent<B>,
    output_width: i32,
    output_height: i32,
) -> MotionEvent {
    let x = event.x_transformed(output_width);
    let y = event.y_transformed(output_height);
    MotionEvent::Absolute {
        x,
        y,
        time_msec: event.time_msec(),
    }
}

/// Construct a `MotionEvent` from a winit event.
pub fn motion_event_from_winit(
    event: impl smithay::backend::input::AbsolutePositionEvent<smithay::backend::winit::WinitInput>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
) -> MotionEvent {
    let x = event.x_transformed(size.w);
    let y = event.y_transformed(size.h);
    MotionEvent::Absolute {
        x,
        y,
        time_msec: event.time_msec(),
    }
}

/// Handle pointer motion from any source (absolute, relative, or warp).
///
/// This is the single entry point for all pointer motion. The motion source
/// is abstracted via the `MotionEvent` type.
pub fn handle_pointer_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: MotionEvent,
) {
    let output_width = wm.g.cfg.screen_width;
    let output_height = wm.g.cfg.screen_height;

    // Compute and update pointer location
    state.runtime.pointer_location =
        event.compute_location(state.runtime.pointer_location, output_width, output_height);
    // Dispatch to focus/drag handling logic
    dispatch_pointer_motion(
        wm,
        state,
        pointer_handle,
        keyboard_handle,
        event.time_msec(),
    );
}

/// Unified pointer motion: update WM hover focus, propagate to clients, handle drags.
pub fn dispatch_pointer_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    time_msec: u32,
) {
    let pointer_location = state.runtime.pointer_location;
    let root_x = pointer_location.x.round() as i32;
    let root_y = pointer_location.y.round() as i32;

    // Get active drag window once - used in multiple phases
    let active_drag_window = wayland_active_drag_window(wm);

    // Phase 1: Compute bar/guard band hit detection
    let (in_bar_band, in_bar_guard_band) = compute_bar_hit(wm, root_x, root_y, active_drag_window);

    // Phase 2: Resolve pointer focus and hovered window
    let (pointer_focus, hovered_win) =
        resolve_pointer_focus(wm, state, in_bar_band, in_bar_guard_band);

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
    let bar_pos = update_wayland_bar_hit_state(wm, root_x, root_y, false);
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

    // Phase 5: Update hover resize state for floating windows
    let suppress_hover_focus = update_hover_resize_state(
        wm,
        root_x,
        root_y,
        hovered_win,
        active_drag_window.is_none(),
    );

    // Phase 6: Update pointer focus based on drag state
    update_pointer_focus(
        wm,
        active_drag_window,
        hovered_win,
        suppress_hover_focus,
        root_x,
        root_y,
    );

    let _ = update_wayland_bar_hit_state(wm, root_x, root_y, false);

    // Phase 7: Handle tag/title drag motion
    handle_wm_drag_motion(wm, keyboard_handle, root_x, root_y);

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
fn compute_bar_hit(
    wm: &Wm,
    root_x: i32,
    root_y: i32,
    active_drag_window: Option<crate::types::WindowId>,
) -> (bool, bool) {
    crate::types::find_monitor_by_rect(
        wm.g.monitors.monitors(),
        &Rect {
            x: root_x,
            y: root_y,
            w: 1,
            h: 1,
        },
    )
    .and_then(|mid| wm.g.monitor(mid))
    .map(|mon| {
        let bar_h = mon.bar_height.max(1);
        let bar_visible = wayland_monitor_bar_visible(wm, mon);
        // 4-pixel guard band below the bar: pointer must move this many pixels
        // past the bar bottom before a window drag is allowed to start.
        let guard_h = 4;
        let drag_active =
            active_drag_window.is_some() || wm.g.drag.interactive.active || wm.g.drag.tag.active;
        let in_bar = bar_visible && root_y >= mon.bar_y && root_y < mon.bar_y + bar_h;
        let in_guard = bar_visible
            && !drag_active
            && root_y >= mon.bar_y + bar_h
            && root_y < mon.bar_y + bar_h + guard_h;
        (in_bar, in_guard)
    })
    .unwrap_or((false, false))
}

/// Resolve pointer focus and hovered window based on bar hit state.
fn resolve_pointer_focus(
    wm: &Wm,
    state: &WaylandState,
    in_bar_band: bool,
    in_bar_guard_band: bool,
) -> (
    Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, smithay::utils::Logical>,
    )>,
    Option<crate::types::WindowId>,
) {
    let pointer_location = state.runtime.pointer_location;
    let mut pointer_focus = if in_bar_band || in_bar_guard_band {
        state.layer_surface_under_pointer(pointer_location)
    } else {
        state
            .layer_surface_under_pointer(pointer_location)
            .or_else(|| state.surface_under_pointer(pointer_location))
    };

    let hovered_win = if in_bar_band || in_bar_guard_band {
        None
    } else if let Some((surface, _)) = state.layer_surface_under_pointer(pointer_location) {
        find_hovered_window_for_surface(wm, &surface)
    } else {
        state.logical_window_under_pointer(pointer_location)
    };

    // If the logical window differs from the surface Smithay found,
    // clear pointer focus so events don't fall through to the background.
    if !in_bar_band
        && !in_bar_guard_band
        && let Some(logical) = hovered_win
        && let Some((surf, _)) = &pointer_focus
        && let Some(actual_win) = find_hovered_window_for_surface(wm, surf)
        && actual_win != logical
    {
        pointer_focus = None;
    }

    (pointer_focus, hovered_win)
}

/// Handle resize drag motion. Returns true if handled (early return).
fn handle_resize_drag_motion(
    ctx: &mut WmCtxWayland<'_>,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    pointer_focus: Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, smithay::utils::Logical>,
    )>,
    time_msec: u32,
) -> bool {
    let pointer_location = state.runtime.pointer_location;
    if !wayland_hover_resize_drag_motion(
        ctx,
        pointer_location.x.round() as i32,
        pointer_location.y.round() as i32,
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
    pointer_focus: Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<i32, smithay::utils::Logical>,
    )>,
    in_bar_band: bool,
    bar_pos: Option<BarPosition>,
    time_msec: u32,
) -> bool {
    let pointer_location = state.runtime.pointer_location;
    let is_drag = wm.g.drag.interactive.active || wm.g.drag.tag.active;
    if (in_bar_band || bar_pos.is_some()) && !is_drag {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
            return true;
        };
        if matches!(
            ctx.core.globals().behavior.cursor_icon,
            AltCursor::Resize(_)
        ) {
            set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                AltCursor::Default,
            );
        }
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
    root_x: i32,
    root_y: i32,
    hovered_win: Option<crate::types::WindowId>,
    no_active_drag: bool,
) -> bool {
    if !no_active_drag {
        return false;
    }

    let selected_floating =
        wm.g.selected_win()
            .and_then(|win| wm.g.clients.get(&win).map(|c| (win, c.is_floating)))
            .is_some_and(|(_, is_floating)| is_floating);
    let hovered_is_selected = hovered_win.is_some_and(|win| Some(win) == wm.g.selected_win());

    let ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
        return false;
    };

    if !selected_floating {
        let _ = update_wayland_selected_resize_offer(&mut ctx, root_x, root_y);
        return false;
    }

    let mut suppress_hover_focus = !hovered_is_selected;
    let selected_offer = update_wayland_selected_resize_offer(&mut ctx, root_x, root_y).is_some();
    if selected_offer {
        suppress_hover_focus = true;
    } else if !hovered_is_selected {
        if let Some((_, dir)) = selected_hover_resize_target_at(
            &crate::contexts::WmCtx::Wayland(ctx.reborrow()),
            root_x,
            root_y,
        ) {
            set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                AltCursor::Resize(dir),
            );
            suppress_hover_focus = true;
        } else if matches!(
            ctx.core.globals().behavior.cursor_icon,
            AltCursor::Resize(_)
        ) {
            set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                AltCursor::Default,
            );
        }
    }

    suppress_hover_focus
}

/// Update pointer focus based on drag state.
fn update_pointer_focus(
    wm: &mut Wm,
    active_drag_window: Option<crate::types::WindowId>,
    hovered_win: Option<crate::types::WindowId>,
    suppress_hover_focus: bool,
    root_x: i32,
    root_y: i32,
) {
    if let Some(lock_win) = active_drag_window {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
            return;
        };
        if ctx.core.selected_client() != Some(lock_win) {
            let _ = crate::focus::focus_wayland(&mut ctx.core, &ctx.wayland, Some(lock_win));
        }
    } else if !suppress_hover_focus {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(ctx) = ctx else {
            return;
        };
        let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx);
        crate::focus::hover_focus_target(&mut wm_ctx, hovered_win, false, Some((root_x, root_y)));
    }
}

/// Handle tag and title drag motion.
fn handle_wm_drag_motion(
    wm: &mut Wm,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    root_x: i32,
    root_y: i32,
) {
    if wm.g.drag.tag.active {
        let mut ctx = wm.ctx();
        if !crate::mouse::drag_tag_motion(&mut ctx, root_x, root_y) {
            let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
            crate::mouse::drag_tag_finish(&mut ctx, mod_state);
        }
    }

    if wm.g.drag.interactive.active {
        let mut ctx = wm.ctx();
        crate::mouse::title_drag_motion(&mut ctx, root_x, root_y);
    }
}

/// Find the hovered window for a given surface.
fn find_hovered_window_for_surface(
    wm: &Wm,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> Option<crate::types::WindowId> {
    use smithay::wayland::compositor::with_states;

    if let Some(win) = with_states(surface, |states| {
        states
            .data_map
            .get::<crate::backend::wayland::compositor::WindowIdMarker>()
            .map(|marker| marker.id)
    }) {
        return Some(win);
    }

    let backend = match &wm.backend {
        crate::backend::Backend::Wayland(data) => &data.backend,
        _ => return None,
    };

    backend
        .with_state(|state| state.window_id_for_surface(surface))
        .flatten()
}
