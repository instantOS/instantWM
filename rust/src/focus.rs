//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `CoreCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::bar::{draw_bars_wayland, draw_bars_x11};
use crate::client::{set_focus_x11, set_urgent, unfocus_win_x11};
use crate::contexts::{CoreCtx, WaylandCtx, WmCtx, WmCtxWayland, WmCtxX11, X11Ctx};
use crate::mouse::{get_cursor_client_win_x11, warp as mouse_warp};
use crate::tags::view;
use crate::types::*;
use x11rb::protocol::xproto::{AtomEnum, InputFocus, PropMode, Window};
use x11rb::CURRENT_TIME;

/// Set focus to a window, or to the root if None.
///
/// # Errors
/// Returns an error if X11 operations fail (e.g., connection lost).
pub fn focus_x11(core: &mut CoreCtx, x11: &X11Ctx, win: Option<WindowId>) -> anyhow::Result<()> {
    let (sel_mon_id, current_sel, mut target, root, net_active_window) = {
        if core.g.monitors.is_empty() {
            return Ok(());
        }
        let sel_mon_id = core.g.selected_monitor_id();
        let mon = core.g.selected_monitor();

        let selected = mon.selected_tag_mask();

        let mut target = win.filter(|w| {
            core.g
                .clients
                .get(w)
                .map(|c| c.is_visible_on_tags(selected.bits()) && !c.is_hidden)
                .unwrap_or(false)
        });

        if target.is_none() {
            for &c_win in &mon.stack {
                let Some(c) = core.g.clients.get(&c_win) else {
                    continue;
                };
                if c.is_visible_on_tags(selected.bits()) && !c.is_hidden {
                    target = Some(c_win);
                    break;
                }
            }
        }

        (
            sel_mon_id,
            mon.sel,
            target,
            core.g.x11.root,
            core.g.x11.netatom.active_window,
        )
    };

    if current_sel != target {
        if let Some(cur_win) = current_sel {
            unfocus_win_x11(core, x11, cur_win, false);
        }
    }

    let selection_state_changed = current_sel.is_none() != target.is_none();

    if let Some(mon) = core.g.monitor_mut(sel_mon_id) {
        mon.sel = target;
        if !matches!(
            mon.gesture,
            Gesture::None | Gesture::Overlay | Gesture::WinTitle(_)
        ) {
            mon.gesture = Gesture::None;
        }
    }

    if selection_state_changed {
        crate::keyboard::grab_keys_x11(core, x11);
    }

    draw_bars_x11(core, x11);

    if let Some(w) = target.take() {
        let is_urgent = core.g.clients.get(&w).map(|c| c.isurgent).unwrap_or(false);
        if is_urgent {
            set_urgent(core, x11, w, false);
        }
        set_focus_x11(core, x11, w);
        Ok(())
    } else {
        let _ = x11
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, root, CURRENT_TIME);
        let _ = x11.conn.delete_property(root, net_active_window);
        let _ = x11.conn.flush();
        Ok(())
    }
}

/// Wayland focus implementation: pick a target window, update mon.sel,
/// tell the backend, and redraw bars.
pub fn focus_wayland(
    core: &mut CoreCtx,
    wayland: &WaylandCtx,
    win: Option<WindowId>,
) -> anyhow::Result<()> {
    if core.g.monitors.is_empty() {
        return Ok(());
    }
    let sel_mon_id = core.g.selected_monitor_id();
    let mon = core.g.selected_monitor();

    let selected = mon.selected_tag_mask();

    // Resolve target: use the requested window if visible, otherwise walk the
    // stack to find the first visible non-hidden client.
    let mut target = win.filter(|w| {
        core.g
            .clients
            .get(w)
            .map(|c| c.is_visible_on_tags(selected.bits()) && !c.is_hidden)
            .unwrap_or(false)
    });

    if target.is_none() {
        for &c_win in &mon.stack {
            let Some(c) = core.g.clients.get(&c_win) else {
                continue;
            };
            if c.is_visible_on_tags(selected.bits()) && !c.is_hidden {
                target = Some(c_win);
                break;
            }
        }
    }

    let current_sel = core.g.selected_monitor().sel;
    let selection_state_changed = current_sel.is_none() != target.is_none();

    if let Some(mon) = core.g.monitor_mut(sel_mon_id) {
        mon.sel = target;
    }

    if selection_state_changed {
        // Desktop keybinds change based on whether a window is selected.
        // TODO: wayland key grabs not applicable; keep desktop bindings in core
    }

    if let Some(w) = target {
        wayland.backend.set_focus(w);
    }

    draw_bars_wayland(core);
    Ok(())
}

/// Best-effort focus.
///
/// Focus failures typically mean the X11 connection is in a bad state; callers
/// in event handlers usually can't recover, but we should not silently drop the
/// error.
pub fn focus_soft_x11(core: &mut CoreCtx, x11: &X11Ctx, win: Option<WindowId>) {
    if let Err(e) = focus_x11(core, x11, win) {
        log::warn!("focus({:?}) failed: {}", win, e);
    }
}

/// Backend-agnostic soft focus - does match internally.
///
/// For X11: calls focus_soft_x11 which logs but doesn't propagate errors.
/// For Wayland: calls focus_wayland which logs but doesn't propagate errors.
pub fn focus_soft(ctx: &mut crate::contexts::WmCtx, win: Option<WindowId>) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 { core, x11, .. }) => {
            focus_soft_x11(core, x11, win);
        }
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            if let Err(e) = focus_wayland(core, wayland, win) {
                log::warn!("focus_wayland({:?}) failed: {}", win, e);
            }
        }
    }
}

/// Backend-agnostic unfocus - does match internally.
///
/// For X11: calls unfocus_win_x11 (resets border, releases buttons, clears focus).
/// For Wayland: currently just tracks last_client (border/focus handled differently).
pub fn unfocus_win(ctx: &mut crate::contexts::WmCtx, win: WindowId, redirect_to_root: bool) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 { core, x11, .. }) => {
            unfocus_win_x11(core, x11, win, redirect_to_root);
        }
        Wayland(WmCtxWayland { core, .. }) => {
            core.focus.last_client = win;
        }
    }
}

/// Backend-agnostic hover-focus entry point.
pub fn hover_focus_target(
    ctx: &mut crate::contexts::WmCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 { core, x11, .. }) => {
            hover_focus_target_x11(core, x11, hovered_win, entering_root);
        }
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            hover_focus_target_wayland(core, wayland, hovered_win, entering_root);
        }
    }
}

/// Backend-agnostic cursor query for hover logic.
pub fn cursor_client(ctx: &crate::contexts::WmCtx) -> Option<WindowId> {
    use crate::contexts::{WmCtx::*, WmCtxX11};
    match ctx {
        X11(WmCtxX11 { core, x11, .. }) => get_cursor_client_win_x11(core, x11),
        Wayland(_) => None,
    }
}

/// X11 hover-focus implementation matching the enter-notify focus path.
pub fn hover_focus_target_x11(
    core: &mut CoreCtx,
    x11: &X11Ctx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    if !core.g.focusfollowsmouse {
        return;
    }

    if let Some(win) = hovered_win {
        if let Some(mid) = core.g.clients.get(&win).and_then(|c| c.monitor_id) {
            if mid != core.g.selected_monitor_id() {
                core.g.set_selected_monitor(mid);
                focus_soft_x11(core, x11, None);
                return;
            }
        }

        let hovered_is_floating = core
            .g
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = core.g.selected_monitor().is_tiling_layout();
        if !core.g.focusfollowsfloatmouse && hovered_is_floating && has_tiling && !entering_root {
            return;
        }
    } else {
        let event_win = WindowId::from(core.g.x11.root);
        if let Some(new_mon_id) = core.g.monitors.win_to_mon(
            event_win,
            core.g.x11.root,
            &*core.g.clients,
            Some(crate::globals::X11Conn { conn: x11.conn }),
        ) {
            if new_mon_id != core.g.selected_monitor_id() {
                core.g.set_selected_monitor(new_mon_id);
                focus_soft_x11(core, x11, None);
                return;
            }
        }
    }

    focus_soft_x11(core, x11, hovered_win);
}

/// Shared hover-focus behavior used by both X11 and Wayland pointer paths.
pub fn hover_focus_target_wayland(
    core: &mut CoreCtx,
    wayland: &WaylandCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    let Some(hovered_win) = hovered_win else {
        return;
    };
    if !core.g.focusfollowsmouse {
        return;
    }

    if let Some(mid) = core.g.clients.get(&hovered_win).and_then(|c| c.monitor_id) {
        if mid != core.g.selected_monitor_id() {
            core.g.set_selected_monitor(mid);
        }
    }

    let hovered_is_floating = core
        .g
        .clients
        .get(&hovered_win)
        .map(|c| c.isfloating)
        .unwrap_or(false);
    let has_tiling = core.g.selected_monitor().is_tiling_layout();
    if !core.g.focusfollowsfloatmouse && hovered_is_floating && has_tiling && !entering_root {
        return;
    }

    if core.selected_client() == Some(hovered_win) {
        return;
    }

    core.set_selected_client(Some(hovered_win));
    wayland.backend.set_focus(hovered_win);
    draw_bars_wayland(core);
}

pub fn set_focus_win_x11(core: &CoreCtx, x11: &X11Ctx, win: WindowId) {
    let x11_win: Window = win.into();
    if let Some(c) = core.g.clients.get(&win) {
        if !c.neverfocus {
            let _ = x11
                .conn
                .set_input_focus(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
            let _ = x11.conn.change_property32(
                PropMode::REPLACE,
                core.g.x11.root,
                core.g.x11.netatom.active_window,
                AtomEnum::WINDOW,
                &[x11_win],
            );
        }
        let _ = x11.conn.flush();
    }
}

/// Focus a client in the given direction.
///
/// This function uses dependency injection by accepting explicit parameters
/// instead of accessing global state directly.
///
/// # Arguments
/// * `monitors` - Slice of all monitors
/// * `sel_mon_id` - Currently selected monitor ID
/// * `clients` - Reference to all clients
/// * `direction` - Direction to search for a client
/// * `focus_fn` - Function to call with the target window
pub fn focus_direction<F>(core: &CoreCtx, direction: Direction, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let mon = core.g.selected_monitor();

    let selected = mon.selected_tag_mask();

    let Some(source_win) = mon.sel else {
        focus_fn(None);
        return;
    };

    let Some(source_client) = core.g.clients.get(&source_win) else {
        focus_fn(None);
        return;
    };

    let (source_center_x, source_center_y) = source_client.geo.center();

    let candidates = get_directional_candidates(
        &mon.clients,
        &*core.g.clients,
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    );

    focus_fn(candidates);
}

fn get_directional_candidates(
    clients: &Vec<WindowId>,
    globals_map: &std::collections::HashMap<WindowId, Client>,
    selected_tags: TagMask,
    source_win: WindowId,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> Option<WindowId> {
    let mut out_client: Option<WindowId> = None;
    let mut min_score: i32 = 0;

    for (c_win, c) in crate::types::ClientListIter::new(clients, globals_map) {
        if !c.is_visible_on_tags(selected_tags.bits()) {
            continue;
        }

        let center_x = c.geo.x + c.geo.w / 2;
        let center_y = c.geo.y + c.geo.h / 2;

        if is_client_in_direction(
            c_win,
            source_win,
            center_x,
            center_y,
            source_center_x,
            source_center_y,
            direction,
        ) {
            let score = calculate_direction_score(
                center_x,
                center_y,
                source_center_x,
                source_center_y,
                direction,
            );
            if score < min_score || min_score == 0 {
                out_client = Some(c_win);
                min_score = score;
            }
        }
    }

    out_client
}

fn is_client_in_direction(
    c_win: WindowId,
    source_win: WindowId,
    center_x: i32,
    center_y: i32,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> bool {
    if c_win == source_win {
        return false;
    }

    match direction {
        Direction::Up => center_y < source_center_y,
        Direction::Down => center_y > source_center_y,
        Direction::Left => center_x < source_center_x,
        Direction::Right => center_x > source_center_x,
    }
}

fn calculate_direction_score(
    center_x: i32,
    center_y: i32,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> i32 {
    let dist_x = (source_center_x - center_x).abs();
    let dist_y = (source_center_y - center_y).abs();

    match direction {
        Direction::Up | Direction::Down => {
            if dist_x > dist_y {
                return i32::MAX;
            }
            dist_x + dist_y / 4
        }
        Direction::Left | Direction::Right => {
            if dist_y > dist_x {
                return i32::MAX;
            }
            dist_y + dist_x / 4
        }
    }
}

pub fn direction_focus_x11(core: &mut CoreCtx, x11: &X11Ctx, direction: Direction) {
    let candidates = {
        if core.g.monitors.is_empty() {
            return;
        }
        let mon = core.g.selected_monitor();
        let Some(source_win) = mon.sel else {
            return;
        };
        let Some(source_client) = core.g.clients.get(&source_win) else {
            return;
        };
        let (source_center_x, source_center_y) = source_client.geo.center();

        let selected = mon.selected_tag_mask();

        get_directional_candidates(
            &mon.clients,
            &*core.g.clients,
            selected,
            source_win,
            source_center_x,
            source_center_y,
            direction,
        )
    };

    if let Some(target) = candidates {
        focus_soft_x11(core, x11, Some(target));
    }
}

pub fn focus_last_client_x11(core: &mut CoreCtx, x11: &X11Ctx) {
    let last_client_win = core.focus.last_client;
    if last_client_win == WindowId::default() {
        return;
    }
    let last_win = last_client_win;

    let last_client = match core.g.clients.get(&last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        crate::scratchpad::scratchpad_show_name(core, &last_client.scratchpad_name);
        return;
    }

    let tags = last_client.tags;
    let last_mon_id = last_client.monitor_id;

    if let Some(last_mid) = last_mon_id {
        let sel_mon_id = core.g.selected_monitor_id();
        if !core.g.monitors.is_empty() && sel_mon_id != last_mid {
            if let Some(sel) = core.g.monitor(sel_mon_id).and_then(|m| m.sel) {
                unfocus_win_x11(core, x11, sel, false);
                core.g.set_selected_monitor(last_mid);
            }
        }
    }

    if let Some(cur) = core.selected_client() {
        core.focus.last_client = cur;
    }

    view(core, x11, TagMask::from_bits(tags));
    focus_soft_x11(core, x11, Some(last_win));

    let monitor_id = core.g.selected_monitor_id();
    crate::layouts::arrange(core, Some(monitor_id));
}

pub fn warp_cursor_to_client_x11(core: &CoreCtx, x11: &X11Ctx, c_win: WindowId) {
    mouse_warp::warp_impl_x11(core, x11, c_win);
}

pub fn warp_to_focus_x11(core: &CoreCtx, x11: &X11Ctx) {
    if let Some(win) = core.selected_client() {
        warp_cursor_to_client_x11(core, x11, win);
    }
}

/// Focus the next or previous client in the stack.
pub fn focus_stack_direction<F>(core: &CoreCtx, forward: bool, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let mon = core.g.selected_monitor();

    let selected_window = mon.sel;
    let stack = get_visible_stack(mon, &*core.g.clients);

    if stack.is_empty() {
        focus_fn(None);
        return;
    }

    let current_idx = match selected_window {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if forward {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    focus_fn(Some(stack[next_idx]));
}

fn get_visible_stack(
    mon: &Monitor,
    clients: &std::collections::HashMap<WindowId, Client>,
) -> Vec<WindowId> {
    let mut stack = Vec::new();
    let selected = mon.selected_tag_mask();

    for (c_win, c) in mon.iter_stack(clients) {
        if c.is_visible_on_tags(selected.bits()) {
            stack.push(c_win);
        }
    }

    stack
}

pub fn focus_stack_x11(core: &mut CoreCtx, x11: &X11Ctx, direction: StackDirection) {
    let selected_window = core.selected_client();

    let stack = {
        if core.g.monitors.is_empty() {
            return;
        }
        let mon = core.g.selected_monitor();
        get_visible_stack(mon, &*core.g.clients)
    };

    if stack.is_empty() {
        return;
    }

    let current_idx = match selected_window {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if direction.is_forward() {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    focus_soft_x11(core, x11, Some(stack[next_idx]));
}

pub fn direction_focus_wayland(core: &mut CoreCtx, wayland: &WaylandCtx, direction: Direction) {
    if core.g.monitors.is_empty() {
        return;
    }
    let mon = core.g.selected_monitor();
    let Some(source_win) = mon.sel else {
        return;
    };
    let Some(source_client) = core.g.clients.get(&source_win) else {
        return;
    };
    let (source_center_x, source_center_y) = source_client.geo.center();

    let selected = mon.selected_tag_mask();

    let candidates = get_directional_candidates(
        &mon.clients,
        &*core.g.clients,
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    );

    if let Some(target) = candidates {
        if let Err(e) = focus_wayland(core, wayland, Some(target)) {
            log::warn!("focus_wayland({:?}) failed: {}", target, e);
        }
    }
}

pub fn focus_stack_wayland(core: &mut CoreCtx, wayland: &WaylandCtx, direction: StackDirection) {
    let selected_window = core.selected_client();

    let stack = {
        if core.g.monitors.is_empty() {
            return;
        }
        let mon = core.g.selected_monitor();
        get_visible_stack(mon, &*core.g.clients)
    };

    if stack.is_empty() {
        return;
    }

    let current_idx = match selected_window {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if direction.is_forward() {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    if let Err(e) = focus_wayland(core, wayland, Some(stack[next_idx])) {
        log::warn!("focus_wayland({:?}) failed: {}", stack[next_idx], e);
    }
}

pub fn direction_focus(ctx: &mut WmCtx, direction: Direction) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 { core, x11, .. }) => direction_focus_x11(core, x11, direction),
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            direction_focus_wayland(core, wayland, direction)
        }
    }
}

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 { core, x11, .. }) => focus_stack_x11(core, x11, direction),
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            focus_stack_wayland(core, wayland, direction)
        }
    }
}
