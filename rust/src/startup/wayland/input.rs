use smithay::backend::input::{
    AbsolutePositionEvent, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
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
use crate::client::resize_x11;
use crate::monitor::update_geom;
use crate::mouse::{set_cursor_default, set_cursor_move, set_cursor_resize};
use crate::startup::common_wayland::modifiers_to_x11_mask;
use crate::types::*;
use crate::wm::Wm;

use super::bar::{
    dispatch_wayland_bar_click, dispatch_wayland_bar_scroll, update_wayland_bar_hit_state,
    wayland_button_to_wm_button,
};
use super::init::sanitize_wayland_size;

pub(super) fn handle_resize(wm: &mut Wm, output: &Output, w: i32, h: i32) {
    let (safe_w, safe_h) = sanitize_wayland_size(w, h);
    let mode = OutputMode {
        size: (safe_w, safe_h).into(),
        refresh: 60_000,
    };
    wm.g.cfg.screen_width = safe_w;
    wm.g.cfg.screen_height = safe_h;
    update_geom(&mut wm.ctx());
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    layer_map_for_output(output).arrange();
}

pub(super) fn handle_keyboard(
    wm: &mut Wm,
    state: &mut WaylandState,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl KeyboardKeyEvent<smithay::backend::winit::WinitInput>,
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
        event.time() as u32,
        |_data, modifiers, keysym| {
            if wm_shortcuts_allowed && event.state() == smithay::backend::input::KeyState::Pressed {
                let mod_mask = modifiers_to_x11_mask(modifiers);
                let mut ctx = wm.ctx();
                let crate::contexts::WmCtx::Wayland(mut ctx) = ctx else {
                    return FilterResult::Forward;
                };
                if crate::keyboard::handle_keysym(
                    &mut ctx.core,
                    u32::from(keysym.modified_sym()),
                    mod_mask,
                ) {
                    return FilterResult::Intercept(());
                }
            }
            FilterResult::Forward
        },
    );
}

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

    if let Some(lock_win) = wayland_active_drag_window(wm) {
        let mut ctx = wm.ctx();
        if ctx.selected_client() != Some(lock_win) {
            crate::focus::focus_soft(&mut ctx, Some(lock_win));
        }
    } else {
        let hovered_win = find_hovered_window(wm, state, *pointer_location);
        let mut ctx = wm.ctx();
        crate::focus::hover_focus_target(&mut ctx, hovered_win, false);
    }

    let root_x = pointer_location.x.round() as i32;
    let root_y = pointer_location.y.round() as i32;

    if wayland_hover_resize_drag_motion(wm, root_x, root_y) {
        return;
    }

    if wayland_active_drag_window(wm).is_none() {
        let mut ctx = wm.ctx();
        let _ = crate::mouse::handle_floating_resize_hover(&mut ctx, root_x, root_y, false);
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
        time: event.time() as u32,
    };
    pointer_handle.motion(state, focus, &motion);
    pointer_handle.frame(state);
}

pub(super) fn handle_pointer_button(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerButtonEvent<smithay::backend::winit::WinitInput>,
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

        let button = smithay::input::pointer::ButtonEvent {
            serial,
            time: event.time() as u32,
            button: event.button_code(),
            state: event.state(),
        };
        pointer_handle.button(state, &button);

        if let Some(pos) = update_wayland_bar_hit_state(wm, root_x, root_y, true) {
            let clean_state = {
                let ctx = wm.ctx();
                crate::util::clean_mask(
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    ctx.g.x11.numlockmask,
                )
            };
            dispatch_wayland_bar_click(wm, pos, event.button_code(), root_x, root_y, clean_state);
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

        let button = smithay::input::pointer::ButtonEvent {
            serial,
            time: event.time() as u32,
            button: event.button_code(),
            state: event.state(),
        };
        pointer_handle.button(state, &button);
        let released_btn = wm_button;

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

pub(super) fn handle_pointer_axis(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerAxisEvent<smithay::backend::winit::WinitInput>,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let mut frame = smithay::input::pointer::AxisFrame::new(event.time() as u32);
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
                let ctx = wm.ctx();
                crate::util::clean_mask(
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    ctx.g.x11.numlockmask,
                )
            };
            dispatch_wayland_bar_scroll(wm, pos, delta, root_x, root_y, clean_state);
        }
    }

    pointer_handle.axis(state, frame);
    pointer_handle.frame(state);
}

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

fn wayland_hover_resize_drag_begin(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
    btn: MouseButton,
) -> bool {
    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }
    let mut ctx = wm.ctx();
    let Some((win, dir)) = crate::mouse::hover::hover_resize_target_at(&ctx, root_x, root_y) else {
        return false;
    };
    let Some((geo, is_floating, has_tiling)) = ctx.g.clients.get(&win).map(|c| {
        (
            c.geo,
            c.isfloating,
            ctx.g.selected_monitor().is_tiling_layout(),
        )
    }) else {
        return false;
    };
    if !is_floating && has_tiling {
        return false;
    }
    let move_mode = btn == MouseButton::Right
        || crate::mouse::hover::is_at_top_middle_edge(&geo, root_x, root_y);
    ctx.g.drag.hover_resize = crate::globals::HoverResizeDragState {
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
    ctx.g.altcursor = AltCursor::Resize;
    ctx.g.drag.resize_direction = Some(dir);
    if move_mode {
        set_cursor_move(&mut ctx);
    } else {
        set_cursor_resize(&mut ctx, Some(dir));
    }
    crate::focus::focus_soft(&mut ctx, Some(win));
    true
}

fn wayland_hover_resize_drag_motion(wm: &mut Wm, root_x: i32, root_y: i32) -> bool {
    let mut ctx = wm.ctx();
    if !ctx.g.drag.hover_resize.active {
        return false;
    }
    let drag = ctx.g.drag.hover_resize.clone();
    ctx.g.drag.hover_resize.last_root_x = root_x;
    ctx.g.drag.hover_resize.last_root_y = root_y;
    if drag.move_mode {
        let new_x = drag.win_start_geo.x + (root_x - drag.start_x);
        let new_y = drag.win_start_geo.y + (root_y - drag.start_y);
        resize(
            &mut ctx,
            drag.win,
            &Rect {
                x: new_x,
                y: new_y,
                w: drag.win_start_geo.w.max(1),
                h: drag.win_start_geo.h.max(1),
            },
            true,
        );
        if let Some(client) = ctx.g.clients.get_mut(&drag.win) {
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
    } else if affects_bottom {
        (orig_top, (root_y - orig_top + 1).max(1))
    } else {
        (orig_top, drag.win_start_geo.h.max(1))
    };
    resize(
        &mut ctx,
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
    let mut ctx = wm.ctx();
    if !ctx.g.drag.hover_resize.active || ctx.g.drag.hover_resize.button != btn {
        return false;
    }
    let drag = ctx.g.drag.hover_resize.clone();
    ctx.g.drag.hover_resize = crate::globals::HoverResizeDragState::default();
    ctx.g.altcursor = AltCursor::None;
    ctx.g.drag.resize_direction = None;
    set_cursor_default(&mut ctx);
    if drag.move_mode {
        crate::mouse::drag::complete_move_drop(
            &mut ctx,
            drag.win,
            drag.win_start_geo,
            None,
            Some((drag.last_root_x, drag.last_root_y)),
        );
    } else {
        crate::mouse::monitor::handle_client_monitor_switch(&mut ctx, drag.win);
    }
    true
}
