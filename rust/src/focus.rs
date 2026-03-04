//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `WmCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::backend::{BackendKind, BackendOps};
use crate::bar::draw_bars;
use crate::client::{set_focus, set_urgent, unfocus_win};
use crate::contexts::WmCtx;
use crate::mouse::warp as mouse_warp;
use crate::tags::view;
use crate::types::*;
use crate::util::X11ConnExt;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

/// Set focus to a window, or to the root if None.
///
/// # Errors
/// Returns an error if X11 operations fail (e.g., connection lost).
pub fn focus(ctx: &mut WmCtx, win: Option<WindowId>) -> anyhow::Result<()> {
    if ctx.backend_kind() == BackendKind::Wayland {
        return focus_wayland(ctx, win);
    }
    let (sel_mon_id, current_sel, mut target, root, net_active_window) = {
        if ctx.g.monitors.is_empty() {
            return Ok(());
        }
        let sel_mon_id = ctx.g.selected_monitor_id();
        let mon = ctx.g.selected_monitor();

        let selected = mon.selected_tag_mask();

        let mut target = win.filter(|w| {
            ctx.g
                .clients
                .get(w)
                .map(|c| c.is_visible_on_tags(selected.bits()) && !c.is_hidden)
                .unwrap_or(false)
        });

        if target.is_none() {
            for &c_win in &mon.stack {
                let Some(c) = ctx.g.clients.get(&c_win) else {
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
            ctx.g.x11.root,
            ctx.g.x11.netatom.active_window,
        )
    };

    if current_sel != target {
        if let Some(cur_win) = current_sel {
            unfocus_win(ctx, cur_win, false);
        }
    }

    let selection_state_changed = current_sel.is_none() != target.is_none();

    if let Some(mon) = ctx.g.monitor_mut(sel_mon_id) {
        mon.sel = target;
        if !matches!(
            mon.gesture,
            Gesture::None | Gesture::Overlay | Gesture::WinTitle(_)
        ) {
            mon.gesture = Gesture::None;
        }
    }

    if selection_state_changed {
        crate::keyboard::grab_keys(ctx);
    }

    draw_bars(ctx);

    if let Some(w) = target.take() {
        let is_urgent = ctx.g.clients.get(&w).map(|c| c.isurgent).unwrap_or(false);
        if is_urgent {
            set_urgent(ctx, w, false);
        }
        set_focus(ctx, w);
        Ok(())
    } else {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            return Ok(());
        };
        // Use the _ctx methods for operations that should report errors
        conn.set_input_focus_ctx(InputFocus::POINTER_ROOT, root, CURRENT_TIME)?;
        conn.delete_property_ctx(root, net_active_window)?;
        conn.flush_ctx()?;
        Ok(())
    }
}

/// Wayland focus implementation: pick a target window, update mon.sel,
/// tell the backend, and redraw bars.
fn focus_wayland(ctx: &mut WmCtx, win: Option<WindowId>) -> anyhow::Result<()> {
    if ctx.g.monitors.is_empty() {
        return Ok(());
    }
    let sel_mon_id = ctx.g.selected_monitor_id();
    let mon = ctx.g.selected_monitor();

    let selected = mon.selected_tag_mask();

    // Resolve target: use the requested window if visible, otherwise walk the
    // stack to find the first visible non-hidden client.
    let mut target = win.filter(|w| {
        ctx.g
            .clients
            .get(w)
            .map(|c| c.is_visible_on_tags(selected.bits()) && !c.is_hidden)
            .unwrap_or(false)
    });

    if target.is_none() {
        for &c_win in &mon.stack {
            let Some(c) = ctx.g.clients.get(&c_win) else {
                continue;
            };
            if c.is_visible_on_tags(selected.bits()) && !c.is_hidden {
                target = Some(c_win);
                break;
            }
        }
    }

    let current_sel = ctx.g.selected_monitor().sel;
    let selection_state_changed = current_sel.is_none() != target.is_none();

    if let Some(mon) = ctx.g.monitor_mut(sel_mon_id) {
        mon.sel = target;
    }

    if selection_state_changed {
        // Desktop keybinds change based on whether a window is selected.
        crate::keyboard::grab_keys(ctx);
    }

    if let Some(w) = target {
        ctx.backend.set_focus(w);
    }

    draw_bars(ctx);
    Ok(())
}

/// Best-effort focus.
///
/// Focus failures typically mean the X11 connection is in a bad state; callers
/// in event handlers usually can't recover, but we should not silently drop the
/// error.
pub fn focus_soft(ctx: &mut WmCtx, win: Option<WindowId>) {
    if let Err(e) = focus(ctx, win) {
        log::warn!("focus({:?}) failed: {}", win, e);
    }
}

/// Shared hover-focus behavior used by both X11 and Wayland pointer paths.
pub fn hover_focus_target(ctx: &mut WmCtx, hovered_win: Option<WindowId>, entering_root: bool) {
    let Some(hovered_win) = hovered_win else {
        return;
    };
    if !ctx.g.focusfollowsmouse {
        return;
    }

    if let Some(mid) = ctx.g.clients.get(&hovered_win).and_then(|c| c.monitor_id) {
        if mid != ctx.g.selected_monitor_id() {
            ctx.g.set_selected_monitor(mid);
        }
    }

    let hovered_is_floating = ctx
        .g
        .clients
        .get(&hovered_win)
        .map(|c| c.isfloating)
        .unwrap_or(false);
    let has_tiling = ctx.g.selected_monitor().is_tiling_layout();
    if !ctx.g.focusfollowsfloatmouse && hovered_is_floating && has_tiling && !entering_root {
        return;
    }

    if ctx.selected_client() == Some(hovered_win) {
        return;
    }

    if ctx.backend_kind() == BackendKind::Wayland {
        ctx.set_selected_client(Some(hovered_win));
        ctx.backend.set_focus(hovered_win);
        draw_bars(ctx);
    } else {
        focus_soft(ctx, Some(hovered_win));
    }
}

pub fn set_focus_win(ctx: &WmCtx, win: WindowId) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };
    let x11_win: Window = win.into();
    if let Some(c) = ctx.g.clients.get(&win) {
        if !c.neverfocus {
            let _ = conn.set_input_focus_ctx(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
            let _ = conn.change_property32_ctx(
                PropMode::REPLACE,
                ctx.g.x11.root,
                ctx.g.x11.netatom.active_window,
                AtomEnum::WINDOW,
                &[x11_win],
            );
        }
        let _ = conn.flush_ctx();
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
pub fn focus_direction<F>(ctx: &WmCtx, direction: Direction, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let mon = ctx.g.selected_monitor();

    let selected = mon.selected_tag_mask();

    let Some(source_win) = mon.sel else {
        focus_fn(None);
        return;
    };

    let Some(source_client) = ctx.g.clients.get(&source_win) else {
        focus_fn(None);
        return;
    };

    let (source_center_x, source_center_y) = source_client.geo.center();

    let candidates = get_directional_candidates(
        &mon.clients,
        &*ctx.g.clients,
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

pub fn direction_focus(ctx: &mut WmCtx, direction: Direction) {
    let candidates = {
        if ctx.g.monitors.is_empty() {
            return;
        }
        let mon = ctx.g.selected_monitor();
        let Some(source_win) = mon.sel else {
            return;
        };
        let Some(source_client) = ctx.g.clients.get(&source_win) else {
            return;
        };
        let (source_center_x, source_center_y) = source_client.geo.center();

        let selected = mon.selected_tag_mask();

        get_directional_candidates(
            &mon.clients,
            &*ctx.g.clients,
            selected,
            source_win,
            source_center_x,
            source_center_y,
            direction,
        )
    };

    if let Some(target) = candidates {
        focus_soft(ctx, Some(target));
    }
}

pub fn focus_last_client(ctx: &mut WmCtx) {
    let last_client_win = ctx.focus.last_client;
    if last_client_win == WindowId::default() {
        return;
    }
    let last_win = last_client_win;

    let last_client = match ctx.g.clients.get(&last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        crate::scratchpad::scratchpad_show_name(ctx, &last_client.scratchpad_name);
        return;
    }

    let tags = last_client.tags;
    let last_mon_id = last_client.monitor_id;

    if let Some(last_mid) = last_mon_id {
        let sel_mon_id = ctx.g.selected_monitor_id();
        if !ctx.g.monitors.is_empty() && sel_mon_id != last_mid {
            if let Some(sel) = ctx.g.monitor(sel_mon_id).and_then(|m| m.sel) {
                unfocus_win(ctx, sel, false);
                ctx.g.set_selected_monitor(last_mid);
            }
        }
    }

    if let Some(cur) = ctx.selected_client() {
        ctx.focus.last_client = cur;
    }

    view(ctx, TagMask::from_bits(tags));
    focus_soft(ctx, Some(last_win));

    let monitor_id = ctx.g.selected_monitor_id();
    crate::layouts::arrange(ctx, Some(monitor_id));
}

pub fn warp_cursor_to_client(ctx: &WmCtx, c_win: WindowId) {
    mouse_warp::warp_impl(ctx, c_win);
}

pub fn warp_to_focus(ctx: &WmCtx) {
    if let Some(win) = ctx.selected_client() {
        warp_cursor_to_client(ctx, win);
    }
}

/// Focus the next or previous client in the stack.
pub fn focus_stack_direction<F>(ctx: &WmCtx, forward: bool, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let mon = ctx.g.selected_monitor();

    let selected_window = mon.sel;
    let stack = get_visible_stack(mon, &*ctx.g.clients);

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

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    let selected_window = ctx.selected_client();

    let stack = {
        if ctx.g.monitors.is_empty() {
            return;
        }
        let mon = ctx.g.selected_monitor();
        get_visible_stack(mon, &*ctx.g.clients)
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

    focus_soft(ctx, Some(stack[next_idx]));
}
