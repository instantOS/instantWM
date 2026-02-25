//! Layout manager — the stateful half of the layout system.

use crate::bar::draw_bar;
use crate::client::{next_tiled, resize, restore_border_width, save_border_width};
use crate::contexts::WmCtx;
use crate::layouts::algo::save_floating;
use crate::layouts::query::{client_count, client_count_mon, get_current_layout};
use crate::types::{Monitor, MonitorId, Rect};
use std::cmp::max;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::LayoutKind;

pub fn arrange(ctx: &mut WmCtx<'_>, mon_id: Option<MonitorId>) {
    crate::mouse::reset_cursor(ctx);

    if let Some(id) = mon_id {
        // First pass: show/hide stack
        let stack = ctx.g.monitors.get(id).map(|m| m.stack);
        if let Some(stack) = stack {
            crate::client::show_hide(ctx, stack);
        }
        // Second pass: arrange and restack
        // Use MonitorId to avoid borrow conflicts
        if let Some(id) = mon_id {
            arrange_monitor(ctx, id);
            restack(ctx, id);
        }
    } else {
        let stacks: Vec<Option<Window>> = ctx.g.monitors.iter().map(|m| m.stack).collect();

        for stack in stacks {
            crate::client::show_hide(ctx, stack);
        }

        // Collect monitor indices first to avoid borrow issues
        let mon_indices: Vec<usize> = (0..ctx.g.monitors.len()).collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
        }
    }
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    // Get client count first
    let clientcount = {
        let m = ctx.g.monitors.get(mon_id).expect("invalid monitor");
        client_count_mon(ctx.g, m) as u32
    };

    // Apply border widths
    apply_border_widths(ctx, mon_id);

    // Run layout
    run_layout(ctx, mon_id);

    // Place overlay
    place_overlay(ctx, mon_id);

    // Update client count
    if let Some(m) = ctx.g.monitors.get_mut(mon_id) {
        m.clientcount = clientcount;
    }
}

fn apply_border_widths(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let m = match ctx.g.monitors.get(mon_id) {
        Some(m) => m,
        None => return,
    };

    let is_tiling = get_current_layout(ctx.g, m).is_tiling();
    let is_monocle = get_current_layout(ctx.g, m).is_monocle();
    let clientcount = m.clientcount;

    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (is_floating, is_fullscreen) = match ctx.g.clients.get(&win) {
            None => break,
            Some(c) => (c.isfloating, c.is_fullscreen),
        };

        let strip_border =
            !is_floating && !is_fullscreen && ((clientcount == 1 && is_tiling) || is_monocle);

        if strip_border {
            save_border_width(win);
            if let Some(c) = ctx.g.clients.get_mut(&win) {
                c.border_width = 0;
            }
        } else {
            restore_border_width(win);
        }

        c_win = ctx.g.clients.get(&win).and_then(|c| next_tiled(c.next));
    }
}

fn run_layout(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    // Clone the layout to avoid borrow issues
    let layout = ctx
        .g
        .monitors
        .get(mon_id)
        .map(|m| get_current_layout(ctx.g, m));
    if let Some(layout) = layout {
        if let Some(m) = ctx.g.monitors.get_mut(mon_id) {
            layout.arrange(ctx, m);
        }
    }
}

fn place_overlay(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let overlay_win = match ctx.g.monitors.get(mon_id).and_then(|m| m.overlay) {
        Some(w) => w,
        None => return,
    };

    if let Some(c) = ctx.g.clients.get_mut(&overlay_win) {
        if c.isfloating {
            save_floating(ctx, overlay_win);
        }
    }

    let bw = ctx
        .g
        .clients
        .get(&overlay_win)
        .map_or(0, |c| c.border_width);

    let geo = if let Some(m) = ctx.g.monitors.get(mon_id) {
        Rect {
            x: m.work_rect.x,
            y: m.work_rect.y,
            w: m.work_rect.w - 2 * bw,
            h: m.work_rect.h - 2 * bw,
        }
    } else {
        return;
    };

    resize(ctx, overlay_win, &geo, false);
}

pub fn restack(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let is_overview = ctx
        .g
        .monitors
        .get(mon_id)
        .map(|m| get_current_layout(ctx.g, m).is_overview())
        .unwrap_or(false);
    if is_overview {
        return;
    }
    let m = ctx.g.monitors.get_mut(mon_id).expect("invalid monitor");

    draw_bar(m);

    let sel_win = match m.sel {
        Some(w) => w,
        None => return,
    };

    let is_tiling = ctx
        .g
        .monitors
        .get(mon_id)
        .map(|mon| get_current_layout(ctx.g, mon).is_tiling())
        .unwrap_or(true);

    let m = ctx.g.monitors.get_mut(mon_id).expect("invalid monitor");
    if let Some(ref conn) = ctx.x11.conn {
        let is_floating = ctx
            .g
            .clients
            .get(&sel_win)
            .map(|c| c.isfloating)
            .unwrap_or(false);

        if is_floating || !is_tiling {
            let _ = configure_window(
                conn,
                sel_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
        }

        if is_tiling {
            let mut wc = ConfigureWindowAux::new()
                .stack_mode(StackMode::BELOW)
                .sibling(m.barwin);

            let mut s_win = m.stack;
            while let Some(win) = s_win {
                match ctx.g.clients.get(&win) {
                    None => break,
                    Some(c) => {
                        let is_win_floating = c.isfloating;
                        let visible = c.is_visible();
                        let snext = c.snext;

                        if !is_win_floating && visible {
                            let _ = configure_window(conn, win, &wc);
                            wc = ConfigureWindowAux::new()
                                .stack_mode(StackMode::ABOVE)
                                .sibling(win);
                        }

                        s_win = snext;
                    }
                }
            }
        }

        let _ = conn.flush();
    }
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: LayoutKind) {
    let tagprefix = ctx.g.tags.prefix;

    if tagprefix {
        for tag in ctx.g.tags.tags.iter_mut() {
            tag.layouts.set_layout(layout);
        }
        ctx.g.tags.prefix = false;
        finish_layout_change(ctx);
        return;
    }

    if let Some(m) = ctx.g.monitors.get_mut(ctx.g.selmon) {
        let current_tag = m.current_tag;
        if current_tag > 0 && current_tag <= ctx.g.tags.tags.len() {
            let tag = &mut ctx.g.tags.tags[current_tag - 1];
            tag.layouts.set_layout(layout);
        }
    }

    finish_layout_change(ctx);
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    let tagprefix = ctx.g.tags.prefix;

    if tagprefix {
        for tag in ctx.g.tags.tags.iter_mut() {
            tag.layouts.toggle_slot();
        }
        ctx.g.tags.prefix = false;
        finish_layout_change(ctx);
        return;
    }

    if let Some(m) = ctx.g.monitors.get_mut(ctx.g.selmon) {
        let current_tag = m.current_tag;
        if current_tag > 0 && current_tag <= ctx.g.tags.tags.len() {
            let tag = &mut ctx.g.tags.tags[current_tag - 1];
            tag.layouts.toggle_slot();
        }
    }

    finish_layout_change(ctx);
}

pub fn restore_last_layout(ctx: &mut WmCtx<'_>) {
    let tagprefix = ctx.g.tags.prefix;

    if tagprefix {
        for tag in ctx.g.tags.tags.iter_mut() {
            tag.layouts.restore_last_layout();
        }
        ctx.g.tags.prefix = false;
        finish_layout_change(ctx);
        return;
    }

    if let Some(m) = ctx.g.monitors.get_mut(ctx.g.selmon) {
        let current_tag = m.current_tag;
        if current_tag > 0 && current_tag <= ctx.g.tags.tags.len() {
            let tag = &mut ctx.g.tags.tags[current_tag - 1];
            tag.layouts.restore_last_layout();
        }
    }

    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selmon = ctx.g.selmon;
    let sel = ctx.g.monitors.get(selmon).and_then(|m| m.sel);

    if sel.is_some() {
        arrange(ctx, Some(selmon));
    } else {
        if let Some(m) = ctx.g.monitors.get_mut(selmon) {
            draw_bar(m);
        }
    }
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| get_current_layout(ctx.g, m));

    let all_layouts = LayoutKind::all();
    let layouts_len = all_layouts.len();

    let current_idx = current_layout
        .map(|l| all_layouts.iter().position(|&x| x == l).unwrap_or(0))
        .unwrap_or(0);

    let candidate = if forward {
        (current_idx + 1) % layouts_len
    } else if current_idx == 0 {
        layouts_len - 1
    } else {
        current_idx - 1
    };

    let candidate_layout = all_layouts[candidate];
    let final_layout = if candidate_layout.is_overview() {
        let final_idx = if forward {
            (candidate + 1) % layouts_len
        } else if candidate == 0 {
            layouts_len - 1
        } else {
            candidate - 1
        };
        all_layouts[final_idx]
    } else {
        candidate_layout
    };

    set_layout(ctx, final_layout);
}

pub fn command_layout(ctx: &mut WmCtx<'_>, layout_idx: u32) {
    let all_layouts = LayoutKind::all();
    let idx = if layout_idx > 0 && (layout_idx as usize) < all_layouts.len() {
        layout_idx as usize
    } else {
        0
    };

    set_layout(ctx, all_layouts[idx]);
}

pub fn inc_nmaster_by(ctx: &mut WmCtx<'_>, delta: i32) {
    let ccount = client_count(ctx.g);

    {
        if let Some(m) = ctx.g.monitors.get_mut(ctx.g.selmon) {
            if delta > 0 && m.nmaster >= ccount {
                m.nmaster = ccount;
                return;
            }

            let new_nmaster = max(m.nmaster + delta, 0);
            m.nmaster = new_nmaster;

            let tag = m.current_tag;
            if tag > 0 && tag <= ctx.g.tags.tags.len() {
                ctx.g.tags.tags[tag - 1].nmaster = new_nmaster;
            }
        }
    }

    let selmon = ctx.g.selmon;
    arrange(ctx, Some(selmon));
}

pub fn set_mfact(ctx: &mut WmCtx<'_>, mfact_val: f32) {
    if mfact_val == 0.0 {
        return;
    }

    let is_tiling = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| get_current_layout(ctx.g, m).is_tiling())
        .unwrap_or(false);

    if !is_tiling {
        return;
    }

    let current_mfact = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| m.mfact)
        .unwrap_or(0.55);

    let new_mfact = if mfact_val < 1.0 {
        mfact_val + current_mfact
    } else {
        mfact_val - 1.0
    };

    if !(0.05..=0.95).contains(&new_mfact) {
        return;
    }

    let animation_on = ctx.g.animated && client_count(ctx.g) > 2;
    if animation_on {
        ctx.g.animated = false;
    }

    {
        if let Some(m) = ctx.g.monitors.get_mut(ctx.g.selmon) {
            m.mfact = new_mfact;
            let tag = m.current_tag;
            if tag > 0 && tag <= ctx.g.tags.tags.len() {
                ctx.g.tags.tags[tag - 1].mfact = new_mfact;
            }
        }
    }

    let selmon = ctx.g.selmon;
    arrange(ctx, Some(selmon));

    if animation_on {
        ctx.g.animated = true;
    }
}
