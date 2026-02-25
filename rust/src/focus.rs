//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `WmCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::bar::draw_bars;
use crate::client::{set_focus, set_urgent, unfocus_win};
use crate::contexts::WmCtx;
use crate::tags::view;
use crate::types::*;
use crate::util::X11ConnExt;
use std::sync::atomic::Ordering;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

/// Set focus to a window, or to the root if None.
///
/// # Errors
/// Returns an error if X11 operations fail (e.g., connection lost).
pub fn focus(ctx: &mut WmCtx, win: Option<Window>) -> anyhow::Result<()> {
    let (sel_mon_id, current_sel, mut target, root, net_active_window) = {
        if ctx.g.monitors.is_empty() {
            return Ok(());
        }
        let sel_mon_id = ctx.g.selmon_id();
        let Some(mon) = ctx.g.selmon() else {
            return Ok(());
        };

        let selected = mon.selected_tags();

        let mut target = win.filter(|w| {
            ctx.g
                .clients
                .get(w)
                .map(|c| c.is_visible_on_tags(selected) && !c.is_hidden)
                .unwrap_or(false)
        });

        if target.is_none() {
            let mut stack = mon.stack;
            while let Some(c_win) = stack {
                let Some(c) = ctx.g.clients.get(&c_win) else {
                    break;
                };
                if c.is_visible_on_tags(selected) && !c.is_hidden {
                    target = Some(c_win);
                    break;
                }
                stack = c.snext;
            }
        }

        (
            sel_mon_id,
            mon.sel,
            target,
            ctx.g.cfg.root,
            ctx.g.cfg.netatom.active_window,
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
        if !matches!(mon.gesture, Gesture::None | Gesture::Overlay) {
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
        let conn = ctx.x11.conn;
        // Use the _ctx methods for operations that should report errors
        conn.set_input_focus_ctx(InputFocus::POINTER_ROOT, root, CURRENT_TIME)?;
        conn.delete_property_ctx(root, net_active_window)?;
        conn.flush_ctx()?;
        Ok(())
    }
}

pub fn set_focus_win(ctx: &WmCtx, win: Window) {
    let conn = ctx.x11.conn;
    if let Some(c) = ctx.g.clients.get(&win) {
        if !c.neverfocus {
            let _ = conn.set_input_focus_ctx(InputFocus::POINTER_ROOT, win, CURRENT_TIME);
            let _ = conn.change_property32_ctx(
                PropMode::REPLACE,
                ctx.g.cfg.root,
                ctx.g.cfg.netatom.active_window,
                AtomEnum::WINDOW,
                &[win],
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
    F: FnOnce(Option<Window>),
{
    let Some(mon) = ctx.g.selmon() else {
        focus_fn(None);
        return;
    };

    let selected = mon.selected_tags();

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
        mon.clients,
        &ctx.g.clients,
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    );

    focus_fn(candidates);
}

fn get_directional_candidates(
    head: Option<Window>,
    globals_map: &std::collections::HashMap<Window, Client>,
    //TODO: there is a struct/enum which abstracts this better, use it
    selected_tags: u32,
    source_win: Window,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> Option<Window> {
    let mut out_client: Option<Window> = None;
    let mut min_score: i32 = 0;

    for (c_win, c) in crate::types::ClientListIter::new(head, globals_map) {
        if !c.is_visible_on_tags(selected_tags) {
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
    c_win: Window,
    source_win: Window,
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
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        let Some(source_win) = mon.sel else {
            return;
        };
        let Some(source_client) = ctx.g.clients.get(&source_win) else {
            return;
        };
        let (source_center_x, source_center_y) = source_client.geo.center();

        let selected = mon.selected_tags();

        get_directional_candidates(
            mon.clients,
            &ctx.g.clients,
            selected,
            source_win,
            source_center_x,
            source_center_y,
            direction,
        )
    };

    if let Some(target) = candidates {
        focus(ctx, Some(target));
    }
}

pub fn focus_last_client(ctx: &mut WmCtx) {
    let last_client_win = crate::client::LAST_CLIENT.load(Ordering::Relaxed);
    if last_client_win == 0 {
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
    let last_mon_id = last_client.mon_id;

    if let Some(last_mid) = last_mon_id {
        let sel_mon_id = ctx.g.selmon_id();
        if !ctx.g.monitors.is_empty() && sel_mon_id != last_mid {
            if let Some(sel) = ctx.g.monitor(sel_mon_id).and_then(|m| m.sel) {
                unfocus_win(ctx, sel, false);
                ctx.g.set_selmon(last_mid);
            }
        }
    }

    if let Some(cur) = get_selected_window(ctx) {
        crate::client::LAST_CLIENT.store(cur, Ordering::Relaxed);
    }

    view(ctx, TagMask::from_bits(tags));
    //TODO: do error propagation
    focus(ctx, Some(last_win));

    let mon_id = ctx.g.selmon_id();
    crate::layouts::arrange(ctx, Some(mon_id));
}

//TODO: is this duplicated? Look for other warp functions in this and the C
//codebase and do what's best
pub fn warp(ctx: &WmCtx, c_win: Window) {
    let conn = ctx.x11.conn;
    if let Some(c) = ctx.g.clients.get(&c_win) {
        if let Some(_cursor_x) = get_root_ptr(ctx) {
            let _ = conn.warp_pointer(
                CURRENT_TIME,
                c.win,
                0,
                0,
                0,
                0,
                (c.geo.w / 2) as i16,
                (c.geo.h / 2) as i16,
            );
            let _ = conn.flush();
        }
    }
}

//TODO: is this duplicated? Look for other warp functions in this and the C
//codebase and do what's best
pub fn force_warp(ctx: &WmCtx, c_win: Window) {
    let conn = ctx.x11.conn;
    if let Some(c) = ctx.g.clients.get(&c_win) {
        let _ = conn.warp_pointer(
            CURRENT_TIME,
            c.win,
            0,
            0,
            0,
            0,
            (c.geo.w / 2) as i16,
            10_i16,
        );
        let _ = conn.flush();
    }
}

pub fn warp_cursor_to_client(ctx: &WmCtx, c_win: Window) {
    let conn = ctx.x11.conn;
    let root = ctx.g.cfg.root;
    let bh = ctx.g.cfg.bar_height;

    //TODO: get rid of magic number
    if c_win == 0 {
        if !ctx.g.monitors.is_empty() {
            if let Some(mon) = ctx.g.selmon() {
                let _ = conn.warp_pointer(
                    CURRENT_TIME,
                    root,
                    0,
                    0,
                    0,
                    0,
                    (mon.work_rect.x + mon.work_rect.w / 2) as i16,
                    (mon.work_rect.y + mon.work_rect.h / 2) as i16,
                );
                let _ = conn.flush();
            }
        }
        return;
    }

    if let Some(c) = ctx.g.clients.get(&c_win) {
        if let Some((x, y)) = get_root_ptr(ctx) {
            let in_window = c.geo.contains_point(x, y)
                || (x > c.geo.x - c.border_width
                    && y > c.geo.y - c.border_width
                    && x < c.geo.x + c.geo.w + c.border_width * 2
                    && y < c.geo.y + c.geo.h + c.border_width * 2);

            let on_bar = if let Some(mon_id) = c.mon_id {
                if let Some(mon) = ctx.g.monitor(mon_id) {
                    (y > mon.by && y < mon.by + bh) || (mon.topbar && y == 0)
                } else {
                    false
                }
            } else {
                false
            };

            if in_window || on_bar {
                return;
            }

            let _ = conn.warp_pointer(
                CURRENT_TIME,
                c.win,
                0,
                0,
                0,
                0,
                (c.geo.w / 2) as i16,
                (c.geo.h / 2) as i16,
            );
            let _ = conn.flush();
        }
    }
}

pub fn warp_into(ctx: &WmCtx, c_win: Window) {
    let conn = ctx.x11.conn;
    let root = ctx.g.cfg.root;

    if let Some(c) = ctx.g.clients.get(&c_win) {
        if let Some((mut x, mut y)) = get_root_ptr(ctx) {
            if x < c.geo.x {
                x = c.geo.x + 10;
            } else if x > c.geo.x + c.geo.w {
                x = c.geo.x + c.geo.w - 10;
            }

            if y < c.geo.y {
                y = c.geo.y + 10;
            } else if y > c.geo.y + c.geo.h {
                y = c.geo.y + c.geo.h - 10;
            }

            let _ = conn.warp_pointer(CURRENT_TIME, root, 0, 0, 0, 0, x as i16, y as i16);
            let _ = conn.flush();
        }
    }
}

pub fn warp_to_focus(ctx: &WmCtx) {
    if let Some(win) = get_selected_window(ctx) {
        warp_cursor_to_client(ctx, win);
    }
}

fn get_root_ptr(ctx: &WmCtx) -> Option<(i32, i32)> {
    let conn = ctx.x11.conn;
    if let Ok(cookie) = query_pointer(conn, ctx.g.cfg.root) {
        if let Ok(reply) = cookie.reply() {
            return Some((reply.root_x as i32, reply.root_y as i32));
        }
    }
    None
}

/// Focus the next or previous client in the stack.
//TODO: check super + up/down keybinds, shouldn't these use this? Or is this function duplicated? Do what's best
pub fn focus_stack_direction<F>(ctx: &WmCtx, forward: bool, focus_fn: F)
where
    F: FnOnce(Option<Window>),
{
    let Some(mon) = ctx.g.selmon() else {
        focus_fn(None);
        return;
    };

    let sel_win = mon.sel;
    let stack = get_visible_stack(mon, &ctx.g.clients);

    if stack.is_empty() {
        focus_fn(None);
        return;
    }

    let current_idx = match sel_win {
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
    clients: &std::collections::HashMap<Window, Client>,
) -> Vec<Window> {
    let mut stack = Vec::new();
    let selected = mon.selected_tags();

    for (c_win, c) in mon.iter_stack(clients) {
        if c.is_visible_on_tags(selected) {
            stack.push(c_win);
        }
    }

    stack
}

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    let sel_win = get_selected_window(ctx);

    let stack = {
        if ctx.g.monitors.is_empty() {
            return;
        }
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        get_visible_stack(mon, &ctx.g.clients)
    };

    if stack.is_empty() {
        return;
    }

    let current_idx = match sel_win {
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

    //TODO: proper error propagation and handling
    focus(ctx, Some(stack[next_idx]));
}

fn get_selected_window(ctx: &WmCtx) -> Option<Window> {
    ctx.g.selected_win()
}
