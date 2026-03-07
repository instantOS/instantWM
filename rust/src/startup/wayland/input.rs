//! Input event handlers for the Wayland compositor backends.
//!
//! The keyboard, pointer-button, and pointer-axis handlers are generic over
//! the Smithay `InputBackend` type so that they can be shared between the
//! nested (winit) backend and the standalone DRM/libinput backend.
//!
//! The pointer-motion handler comes in two flavours:
//! - `handle_pointer_motion_absolute` — for winit / tablets / touch screens
//!   that report absolute coordinates.
//! - `handle_pointer_motion_relative` — for real mice under libinput that
//!   report relative (delta) motion.

use smithay::backend::input::{
    AbsolutePositionEvent, InputBackend, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::layer_map_for_output;
use smithay::input::keyboard::{FilterResult, KeyboardHandle};
use smithay::input::pointer::PointerHandle;
use smithay::output::{Mode as OutputMode, Output};
use smithay::utils::{Point, Transform, SERIAL_COUNTER};

use crate::backend::wayland::compositor::{
    KeyboardFocusTarget, PointerFocusTarget, WaylandState, WindowIdMarker,
};
use crate::monitor::update_geom;
use crate::mouse::constants::RESIZE_BORDER_ZONE;
use crate::mouse::{
    set_cursor_default_wayland, set_cursor_move_wayland, set_cursor_resize_wayland,
};
use crate::startup::common_wayland::modifiers_to_x11_mask;
use crate::types::*;
use crate::wm::Wm;

use super::bar::{
    dispatch_wayland_bar_click, dispatch_wayland_bar_scroll, update_wayland_bar_hit_state,
    wayland_button_to_wm_button,
};
use super::init::sanitize_wayland_size;

// ─────────────────────────────────────────────────────────────────────────────
// Pending warp — compositor-side cursor teleport
// ─────────────────────────────────────────────────────────────────────────────

/// Consume any pending warp stored in `WaylandState` and synthesise a full
/// Smithay pointer-motion event so that:
///
/// 1. The external `pointer_location` variable (owned by the event-loop
///    closure) is updated to the new position.
/// 2. `pointer_handle.motion()` is called, which sends `wl_pointer::enter`
///    / `wl_pointer::motion` / `wl_pointer::leave` to the right clients and
///    updates the internal Smithay focus.
/// 3. `pointer_handle.frame()` closes the event batch.
///
/// Call this once per event-loop tick, *before* rendering, so the rendered
/// cursor position matches the pointer protocol state.
///
/// Returns `true` when a warp was applied (callers may wish to mark output
/// dirty so the new cursor position is painted immediately).
pub fn apply_pending_warp(
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    pointer_location: &mut Point<f64, smithay::utils::Logical>,
) -> bool {
    let Some(target) = state.take_pending_warp() else {
        return false;
    };

    *pointer_location = target;

    let focus = state
        .layer_surface_under_pointer(target)
        .or_else(|| state.surface_under_pointer(target))
        .map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));

    let serial = SERIAL_COUNTER.next_serial();
    let time_msec = {
        use smithay::utils::{Clock, Monotonic};
        Clock::<Monotonic>::new().now().as_millis()
    };
    let motion = smithay::input::pointer::MotionEvent {
        location: target,
        serial,
        time: time_msec,
    };

    // We need a mutable borrow of the handle to call motion/frame.
    // Clone the handle (cheap Arc clone) so we can call methods on it while
    // `state` is also borrowed through the focus computation above.
    let mut ph = pointer_handle.clone();
    ph.motion(state, focus, &motion);
    ph.frame(state);

    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Resize helper (winit-only — output size comes from the backend window)
// ─────────────────────────────────────────────────────────────────────────────

pub(super) fn handle_resize(wm: &mut Wm, output: &Output, w: i32, h: i32) {
    let (safe_w, safe_h) = sanitize_wayland_size(w, h);
    let mode = OutputMode {
        size: (safe_w, safe_h).into(),
        refresh: 60_000,
    };
    wm.g.cfg.screen_width = safe_w;
    wm.g.cfg.screen_height = safe_h;
    update_geom(&mut wm.ctx());
    // Transform::Flipped180 is REQUIRED for the winit (nested) backend.
    //
    // Smithay's winit backend renders into an OpenGL framebuffer whose
    // Y-axis points upward (OpenGL convention), but the host Wayland
    // compositor expects the top-left origin (Wayland convention).  The
    // result is that every frame arrives at the host upside-down unless
    // we tell Smithay's output machinery to compensate with a 180° flip.
    //
    // Smithay applies this transform when compositing space elements so
    // that the final pixel layout sent to the host is right-side up.
    //
    // DO NOT replace this with Transform::Normal — the entire compositor
    // output will be rendered upside-down inside the host window.
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    layer_map_for_output(output).arrange();
}

// ─────────────────────────────────────────────────────────────────────────────
// Keyboard — generic over InputBackend B
// ─────────────────────────────────────────────────────────────────────────────

pub fn handle_keyboard<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl KeyboardKeyEvent<B>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    if matches!(
        keyboard_handle.current_focus(),
        None | Some(KeyboardFocusTarget::Window(_))
    ) {
        if let Some(layer_surface) = state.keyboard_focus_layer_surface() {
            keyboard_handle.set_focus(
                state,
                Some(KeyboardFocusTarget::WlSurface(layer_surface)),
                serial,
            );
        }
    }
    let wm_shortcuts_allowed = matches!(
        keyboard_handle.current_focus(),
        None | Some(KeyboardFocusTarget::Window(_))
    );
    keyboard_handle.input(
        state,
        event.key_code(),
        event.state(),
        serial,
        event.time_msec(),
        |_data, modifiers, keysym| {
            if wm_shortcuts_allowed && event.state() == smithay::backend::input::KeyState::Pressed {
                let mod_mask = modifiers_to_x11_mask(modifiers);
                let ctx = wm.ctx();
                let crate::contexts::WmCtx::Wayland(ctx) = ctx else {
                    return FilterResult::Forward;
                };
                let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx);
                if crate::keyboard::handle_keysym(
                    &mut wm_ctx,
                    keysym.raw_syms().first().map_or(0, |ks| ks.raw()),
                    mod_mask,
                ) {
                    return FilterResult::Intercept(());
                }
            }
            FilterResult::Forward
        },
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Pointer motion — absolute (winit / tablets)
// ─────────────────────────────────────────────────────────────────────────────

pub(super) fn handle_pointer_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    backend: &WinitGraphicsBackend<GlesRenderer>,
    event: impl AbsolutePositionEvent<smithay::backend::winit::WinitInput>,
    pointer_location: &mut Point<f64, smithay::utils::Logical>,
) {
    let size = backend.window_size();
    let x = event.x_transformed(size.w);
    let y = event.y_transformed(size.h);
    *pointer_location = Point::from((x, y));
    let time_msec = event.time_msec();
    dispatch_pointer_motion(
        wm,
        state,
        pointer_handle,
        keyboard_handle,
        pointer_location,
        time_msec,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Pointer motion — relative (real mouse via libinput)
// ─────────────────────────────────────────────────────────────────────────────

/// Handle a relative pointer-motion event as produced by libinput for a
/// standard mouse.  The caller must pass the output bounding box so that the
/// accumulated pointer location can be clamped to the visible area.
pub fn handle_pointer_motion_relative<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerMotionEvent<B>,
    pointer_location: &mut Point<f64, smithay::utils::Logical>,
    output_width: i32,
    output_height: i32,
) {
    let dx = event.delta_x();
    let dy = event.delta_y();
    pointer_location.x = (pointer_location.x + dx).clamp(0.0, output_width as f64);
    pointer_location.y = (pointer_location.y + dy).clamp(0.0, output_height as f64);
    let time_msec = event.time_msec();
    dispatch_pointer_motion(
        wm,
        state,
        pointer_handle,
        keyboard_handle,
        pointer_location,
        time_msec,
    );
}

/// Handle an absolute pointer-motion event coming from libinput (tablet /
/// touch screen).  The caller provides the output dimensions so that the
/// normalised [0,1] absolute position can be converted to logical pixels.
pub fn handle_pointer_motion_absolute<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl AbsolutePositionEvent<B>,
    pointer_location: &mut Point<f64, smithay::utils::Logical>,
    output_width: i32,
    output_height: i32,
) {
    let x = event.x_transformed(output_width);
    let y = event.y_transformed(output_height);
    *pointer_location = Point::from((x, y));
    let time_msec = event.time_msec();
    dispatch_pointer_motion(
        wm,
        state,
        pointer_handle,
        keyboard_handle,
        pointer_location,
        time_msec,
    );
}

/// Shared body for both absolute and relative motion: update WM hover focus,
/// propagate the motion event to clients, handle drag states.
fn dispatch_pointer_motion(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    pointer_location: &mut Point<f64, smithay::utils::Logical>,
    time_msec: u32,
) {
    let root_x = pointer_location.x.round() as i32;
    let root_y = pointer_location.y.round() as i32;

    if wayland_hover_resize_drag_motion(wm, root_x, root_y) {
        // During an active resize drag, still forward motion to Smithay so
        // the pointer protocol stays in sync, but skip focus updates.
        let serial = SERIAL_COUNTER.next_serial();
        let motion = smithay::input::pointer::MotionEvent {
            location: *pointer_location,
            serial,
            time: time_msec,
        };
        let focus = state
            .surface_under_pointer(*pointer_location)
            .map(|(s, l)| (PointerFocusTarget::WlSurface(s), l.to_f64()));
        pointer_handle.motion(state, focus, &motion);
        pointer_handle.frame(state);
        return;
    }

    let active_drag_window = wayland_active_drag_window(wm);
    let resize_offer_window = if active_drag_window.is_none() {
        update_wayland_selected_resize_offer(wm, root_x, root_y)
    } else {
        None
    };
    let focus_lock_window = active_drag_window.or(resize_offer_window);

    if let Some(lock_win) = focus_lock_window {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
            return;
        };
        if ctx.core.selected_client() != Some(lock_win) {
            let _ = crate::focus::focus_wayland(&mut ctx.core, &ctx.wayland, Some(lock_win));
        }
    } else {
        let hovered_win = find_hovered_window(wm, state, *pointer_location);
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::Wayland(ctx) = ctx else {
            return;
        };
        let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx);
        crate::focus::hover_focus_target(&mut wm_ctx, hovered_win, false);
    }

    let _ = update_wayland_bar_hit_state(wm, root_x, root_y, false);

    if wm.g.drag.tag.active {
        let mut ctx = wm.ctx();
        if !crate::mouse::drag_tag_motion(&mut ctx, root_x, root_y) {
            let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
            crate::mouse::drag_tag_finish(&mut ctx, mod_state);
        }
    }

    if wm.g.drag.title.active {
        let mut ctx = wm.ctx();
        crate::mouse::title_drag_motion(&mut ctx, root_x, root_y);
    }

    let focus = state
        .layer_surface_under_pointer(*pointer_location)
        .or_else(|| state.surface_under_pointer(*pointer_location))
        .map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc.to_f64()));

    let serial = SERIAL_COUNTER.next_serial();
    let motion = smithay::input::pointer::MotionEvent {
        location: *pointer_location,
        serial,
        time: time_msec,
    };
    pointer_handle.motion(state, focus, &motion);
    pointer_handle.frame(state);
}

// ─────────────────────────────────────────────────────────────────────────────
// Pointer button — generic over InputBackend B
// ─────────────────────────────────────────────────────────────────────────────

pub fn handle_pointer_button<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerButtonEvent<B>,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let root_x = pointer_location.x.round() as i32;
    let root_y = pointer_location.y.round() as i32;
    let wm_button = wayland_button_to_wm_button(event.button_code()).and_then(MouseButton::from_u8);

    if event.state() == smithay::backend::input::ButtonState::Pressed {
        if let Some(btn) = wm_button {
            if wayland_hover_resize_drag_begin(wm, root_x, root_y, btn) {
                return;
            }
        }

        // Check ClientWin bindings (e.g. Super+Left to move) before forwarding
        // the button event to the client, so modifier-based WM actions consume
        // the press instead of passing it through.
        let mut consumed = false;
        if update_wayland_bar_hit_state(wm, root_x, root_y, false).is_none() {
            if let Some(btn) = wm_button {
                let clean_state = crate::util::clean_mask(
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    wm.x11_runtime.numlockmask,
                );
                consumed = dispatch_wayland_client_button(wm, btn, root_x, root_y, clean_state);
            }
        }

        if !consumed {
            let button = smithay::input::pointer::ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: event.state(),
            };
            pointer_handle.button(state, &button);
        }

        if let Some(pos) = update_wayland_bar_hit_state(wm, root_x, root_y, true) {
            let clean_state = {
                crate::util::clean_mask(
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    wm.x11_runtime.numlockmask,
                )
            };
            dispatch_wayland_bar_click(wm, pos, event.button_code(), root_x, root_y, clean_state);
        } else if !consumed {
            let maybe_close = {
                let core = crate::contexts::CoreCtx::new(
                    &mut wm.g,
                    &mut wm.running,
                    &mut wm.bar,
                    &mut wm.focus,
                );
                let mon = core.g.selected_monitor().clone();
                let local_x = root_x - mon.work_rect.x;
                wm.wayland_systray_menu.is_some()
                    && crate::wayland_systray::hit_test_wayland_systray_menu_item_with_state(
                        &core,
                        &wm.wayland_systray,
                        wm.wayland_systray_menu.as_ref(),
                        &mon,
                        local_x,
                    )
                    .is_none()
            };
            if maybe_close {
                wm.wayland_systray_menu = None;
                wm.bar.mark_dirty();
            }
        }

        let keyboard_focus = state
            .layer_surface_under_pointer(pointer_location)
            .map(|(surface, _)| KeyboardFocusTarget::WlSurface(surface))
            .or_else(|| {
                state
                    .space
                    .element_under(pointer_location)
                    .map(|(window, _)| KeyboardFocusTarget::Window(window.clone()))
            });
        keyboard_handle.set_focus(state, keyboard_focus, serial);
    } else if event.state() == smithay::backend::input::ButtonState::Released {
        if let Some(btn) = wm_button {
            if wayland_hover_resize_drag_finish(wm, btn) {
                return;
            }
        }

        let released_btn = wm_button;
        let is_wm_drag = (wm.g.drag.title.active && released_btn == Some(wm.g.drag.title.button))
            || (wm.g.drag.tag.active && released_btn == Some(wm.g.drag.tag.button));

        if !is_wm_drag {
            let button = smithay::input::pointer::ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: event.state(),
            };
            pointer_handle.button(state, &button);
        }

        if wm.g.drag.tag.active && released_btn == Some(wm.g.drag.tag.button) {
            let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
            let mut ctx = wm.ctx();
            crate::mouse::drag_tag_finish(&mut ctx, mod_state);
        }

        if wm.g.drag.title.active && released_btn == Some(wm.g.drag.title.button) {
            let mut ctx = wm.ctx();
            crate::mouse::title_drag_finish(&mut ctx);
        }
    }

    pointer_handle.frame(state);
}

// ─────────────────────────────────────────────────────────────────────────────
// Pointer axis (scroll) — generic over InputBackend B
// ─────────────────────────────────────────────────────────────────────────────

pub fn handle_pointer_axis<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerAxisEvent<B>,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let mut frame = smithay::input::pointer::AxisFrame::new(event.time_msec());
    frame = frame.source(event.source());

    if let Some(amount) = event.amount(smithay::backend::input::Axis::Vertical) {
        frame = frame.value(smithay::backend::input::Axis::Vertical, amount);
    }
    if let Some(amount) = event.amount(smithay::backend::input::Axis::Horizontal) {
        frame = frame.value(smithay::backend::input::Axis::Horizontal, amount);
    }
    if let Some(steps) = event.amount_v120(smithay::backend::input::Axis::Vertical) {
        frame = frame.v120(smithay::backend::input::Axis::Vertical, steps as i32);
    }
    if let Some(steps) = event.amount_v120(smithay::backend::input::Axis::Horizontal) {
        frame = frame.v120(smithay::backend::input::Axis::Horizontal, steps as i32);
    }

    let scroll_delta = event
        .amount_v120(smithay::backend::input::Axis::Vertical)
        .map(|s| s as f64)
        .or_else(|| event.amount(smithay::backend::input::Axis::Vertical));
    if let Some(delta) = scroll_delta.filter(|d| *d != 0.0) {
        let root_x = pointer_location.x.round() as i32;
        let root_y = pointer_location.y.round() as i32;
        if let Some(pos) = update_wayland_bar_hit_state(wm, root_x, root_y, true) {
            let clean_state = {
                crate::util::clean_mask(
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    wm.x11_runtime.numlockmask,
                )
            };
            dispatch_wayland_bar_scroll(wm, pos, delta, root_x, root_y, clean_state);
        }
    }

    pointer_handle.axis(state, frame);
    pointer_handle.frame(state);
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers (unchanged from original)
// ─────────────────────────────────────────────────────────────────────────────

fn find_hovered_window(
    wm: &Wm,
    state: &WaylandState,
    pointer_location: Point<f64, smithay::utils::Logical>,
) -> Option<WindowId> {
    let pointer_x = pointer_location.x;
    let pointer_y = pointer_location.y;
    for window in state.space.elements().rev() {
        let Some(w) = window.user_data().get::<WindowIdMarker>().map(|m| m.id) else {
            continue;
        };
        let Some(c) = wm.g.clients.get(&w) else {
            continue;
        };
        if c.is_hidden {
            continue;
        }
        let is_visible = c
            .monitor_id
            .and_then(|mid| wm.g.monitor(mid))
            .map(|m| c.is_visible_on_tags(m.selected_tags()))
            .unwrap_or(false);
        if !is_visible {
            continue;
        }
        let bw = c.border_width.max(0) as f64;
        let ox = c.geo.x as f64;
        let oy = c.geo.y as f64;
        let ow = c.geo.w as f64 + 2.0 * bw;
        let oh = c.geo.h as f64 + 2.0 * bw;
        if pointer_x >= ox && pointer_x < ox + ow && pointer_y >= oy && pointer_y < oy + oh {
            return Some(w);
        }
    }

    state
        .space
        .element_under(pointer_location)
        .and_then(|(window, _)| window.user_data().get::<WindowIdMarker>().map(|m| m.id))
}

fn wayland_active_drag_window(wm: &Wm) -> Option<WindowId> {
    if wm.g.drag.hover_resize.active {
        return Some(wm.g.drag.hover_resize.win);
    }
    if wm.g.drag.title.active {
        return Some(wm.g.drag.title.win);
    }
    None
}

fn dispatch_wayland_client_button(
    wm: &mut Wm,
    btn: MouseButton,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) -> bool {
    let buttons = wm.g.cfg.buttons.clone();
    for b in &buttons {
        if !b.matches(BarPosition::ClientWin) || b.button != btn {
            continue;
        }
        if crate::util::clean_mask(b.mask, 0) != clean_state {
            continue;
        }
        let mut ctx = wm.ctx();
        (b.action)(
            &mut ctx,
            ButtonArg {
                pos: BarPosition::ClientWin,
                btn: b.button,
                rx: root_x,
                ry: root_y,
            },
        );
        return true;
    }
    false
}

fn wayland_hover_resize_drag_begin(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
    btn: MouseButton,
) -> bool {
    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
        return false;
    };
    let Some((win, dir, geo)) = wayland_selected_resize_target_at(&ctx, root_x, root_y) else {
        return false;
    };
    let move_mode = btn == MouseButton::Right
        || crate::mouse::hover::is_at_top_middle_edge(&geo, root_x, root_y);
    ctx.core.g.drag.hover_resize = crate::globals::HoverResizeDragState {
        active: true,
        win,
        button: btn,
        direction: dir,
        move_mode,
        start_x: root_x,
        start_y: root_y,
        win_start_geo: geo,
        last_root_x: root_x,
        last_root_y: root_y,
    };
    ctx.core.g.altcursor = AltCursor::Resize;
    ctx.core.g.drag.resize_direction = Some(dir);
    if move_mode {
        set_cursor_move_wayland(&mut ctx);
    } else {
        set_cursor_resize_wayland(&mut ctx, Some(dir));
    }
    let _ = crate::focus::focus_wayland(&mut ctx.core, &ctx.wayland, Some(win));
    crate::contexts::WmCtx::Wayland(ctx.reborrow()).raise_interactive(win);
    true
}

fn wayland_selected_resize_target_at(
    ctx: &crate::contexts::WmCtxWayland<'_>,
    root_x: i32,
    root_y: i32,
) -> Option<(WindowId, ResizeDirection, Rect)> {
    let win = ctx.core.selected_client()?;
    let mon = ctx.core.g.selected_monitor();
    if mon.showbar && root_y < mon.monitor_rect.y + ctx.core.g.cfg.bar_height {
        return None;
    }
    let selected_tags = mon.selected_tags();
    let c = ctx.core.g.clients.get(&win)?;
    if c.is_hidden || !c.is_visible_on_tags(selected_tags) {
        return None;
    }
    let has_tiling = mon.is_tiling_layout();
    if !c.isfloating && has_tiling {
        return None;
    }
    if !is_point_in_resize_border(c.geo, root_x, root_y) {
        return None;
    }
    let hit_x = root_x - c.geo.x;
    let hit_y = root_y - c.geo.y;
    let dir = get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y);
    Some((win, dir, c.geo))
}

fn update_wayland_selected_resize_offer(wm: &mut Wm, root_x: i32, root_y: i32) -> Option<WindowId> {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
        return None;
    };
    let Some((win, dir, _)) = wayland_selected_resize_target_at(&ctx, root_x, root_y) else {
        if ctx.core.g.altcursor == AltCursor::Resize {
            clear_wayland_hover_resize_offer(&mut ctx);
        }
        return None;
    };
    set_cursor_resize_wayland(&mut ctx, Some(dir));
    ctx.core.g.altcursor = AltCursor::Resize;
    ctx.core.g.drag.resize_direction = Some(dir);
    Some(win)
}

fn clear_wayland_hover_resize_offer(ctx: &mut crate::contexts::WmCtxWayland<'_>) {
    ctx.core.g.altcursor = AltCursor::None;
    ctx.core.g.drag.resize_direction = None;
    set_cursor_default_wayland(ctx);
}

fn is_point_in_resize_border(geo: Rect, x: i32, y: i32) -> bool {
    if x > geo.x && x < geo.x + geo.w && y > geo.y && y < geo.y + geo.h {
        return false;
    }
    if y < geo.y - RESIZE_BORDER_ZONE
        || x < geo.x - RESIZE_BORDER_ZONE
        || y > geo.y + geo.h + RESIZE_BORDER_ZONE
        || x > geo.x + geo.w + RESIZE_BORDER_ZONE
    {
        return false;
    }
    true
}

fn wayland_hover_resize_drag_motion(wm: &mut Wm, root_x: i32, root_y: i32) -> bool {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
        return false;
    };
    if !ctx.core.g.drag.hover_resize.active {
        return false;
    }
    let drag = ctx.core.g.drag.hover_resize.clone();
    ctx.core.g.drag.hover_resize.last_root_x = root_x;
    ctx.core.g.drag.hover_resize.last_root_y = root_y;
    if drag.move_mode {
        let mut new_x = drag.win_start_geo.x + (root_x - drag.start_x);
        let mut new_y = drag.win_start_geo.y + (root_y - drag.start_y);
        {
            let wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
            crate::mouse::drag::snap_window_to_monitor_edges(
                &wm_ctx,
                drag.win,
                drag.win_start_geo.w.max(1),
                drag.win_start_geo.h.max(1),
                &mut new_x,
                &mut new_y,
            );
        }
        crate::client::resize(
            &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
            drag.win,
            &Rect {
                x: new_x,
                y: new_y,
                w: drag.win_start_geo.w.max(1),
                h: drag.win_start_geo.h.max(1),
            },
            true,
        );
        if let Some(client) = ctx.core.g.clients.get_mut(&drag.win) {
            client.float_geo.x = new_x;
            client.float_geo.y = new_y;
        }
        return true;
    }

    let orig_left = drag.win_start_geo.x;
    let orig_top = drag.win_start_geo.y;
    let orig_right = drag.win_start_geo.x + drag.win_start_geo.w;
    let orig_bottom = drag.win_start_geo.y + drag.win_start_geo.h;
    let (affects_left, affects_right, affects_top, affects_bottom) =
        drag.direction.affected_edges();
    let (new_x, new_w) = if affects_left {
        (root_x, (orig_right - root_x).max(1))
    } else if affects_right {
        (orig_left, (root_x - orig_left + 1).max(1))
    } else {
        (orig_left, drag.win_start_geo.w.max(1))
    };
    let (new_y, new_h) = if affects_top {
        (root_y, (orig_bottom - root_y).max(1))
    } else if !affects_top && affects_bottom {
        (orig_top, (root_y - orig_top + 1).max(1))
    } else {
        (orig_top, drag.win_start_geo.h.max(1))
    };
    crate::client::resize(
        &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
        drag.win,
        &Rect {
            x: new_x,
            y: new_y,
            w: new_w,
            h: new_h,
        },
        true,
    );
    true
}

fn wayland_hover_resize_drag_finish(wm: &mut Wm, btn: MouseButton) -> bool {
    let ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
        return false;
    };
    if !ctx.core.g.drag.hover_resize.active || ctx.core.g.drag.hover_resize.button != btn {
        return false;
    }
    let drag = ctx.core.g.drag.hover_resize.clone();
    ctx.core.g.drag.hover_resize = crate::globals::HoverResizeDragState::default();
    ctx.core.g.altcursor = AltCursor::None;
    ctx.core.g.drag.resize_direction = None;
    set_cursor_default_wayland(&mut ctx);
    if drag.move_mode {
        crate::mouse::drag::complete_move_drop(
            &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
            drag.win,
            drag.win_start_geo,
            None,
            Some((drag.last_root_x, drag.last_root_y)),
        );
    } else {
        crate::mouse::monitor::handle_client_monitor_switch(
            &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
            drag.win,
        );
    }
    true
}
