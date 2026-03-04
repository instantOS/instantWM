//! Layout manager — the stateful half of the layout system.

use crate::bar::draw_bar;
use crate::contexts::WmCtx;
use crate::layouts::algo::save_floating;
use crate::layouts::query::{client_count, client_count_mon, get_current_layout};
use crate::types::{MonitorId, Rect, WindowId};
use std::cmp::max;

use super::LayoutKind;

pub fn arrange(ctx: &mut WmCtx<'_>, mon_id: Option<MonitorId>) {
    crate::mouse::reset_cursor(ctx);

    if let Some(id) = mon_id {
        // First pass: show/hide stack
        if let Some(mon) = ctx.g.monitor(id) {
            crate::client::show_hide(ctx, mon.stack);
        }
        // Second pass: arrange and restack
        arrange_monitor(ctx, id);
        restack(ctx, id);
    } else {
        let stacks: Vec<Option<WindowId>> = ctx.g.monitors_iter().map(|(_i, m)| m.stack).collect();
        for stack in stacks {
            crate::client::show_hide(ctx, stack);
        }

        let mon_indices: Vec<usize> = (0..ctx.g.monitors.count()).collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
            restack(ctx, idx);
        }
    }

    ctx.flush();
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let clientcount = {
        let m = ctx.g.monitor(mon_id).expect("invalid monitor");
        client_count_mon(ctx.g, m) as u32
    };

    if let Some(m) = ctx.g.monitor_mut(mon_id) {
        m.clientcount = clientcount;
    }

    apply_border_widths(ctx, mon_id);
    run_layout(ctx, mon_id);
    place_overlay(ctx, mon_id);
}

fn apply_border_widths(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let m = match ctx.g.monitor(mon_id) {
        Some(m) => m,
        None => return,
    };

    let is_tiling = get_current_layout(ctx.g, m).is_tiling();
    let is_monocle = get_current_layout(ctx.g, m).is_monocle();
    let clientcount = m.clientcount;
    let selected_tags = m.selected_tags();

    let mut c_win = m.clients;
    while let Some(win) = c_win {
        // Resolve all info first to avoid borrow conflicts
        let client_info = ctx.client(win).map(|c| {
            (
                c.isfloating,
                c.is_fullscreen,
                c.is_visible_on_tags(selected_tags) && !c.is_hidden,
                c.old_border_width,
                c.next,
            )
        });

        if let Some((is_floating, is_fs, is_visible, old_bw, next)) = client_info {
            if is_visible {
                let strip_border =
                    !is_floating && !is_fs && ((clientcount == 1 && is_tiling) || is_monocle);
                if strip_border {
                    ctx.set_border(win, 0);
                } else if old_bw != 0 {
                    ctx.set_border(win, old_bw);
                }
            }
            c_win = next;
        } else {
            break;
        }
    }
}

fn run_layout(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let layout = ctx.g.monitor(mon_id).map(|m| get_current_layout(ctx.g, m));
    if let Some(layout) = layout {
        if let Some(mut m) = ctx.g.monitor(mon_id).cloned() {
            layout.arrange(ctx, &mut m);
            ctx.g.monitors.set_monitor(mon_id, m);
        }
    }
}

fn place_overlay(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let (overlay_win, work_rect) = match ctx.g.monitor(mon_id) {
        Some(m) => (m.overlay, m.work_rect),
        None => return,
    };

    let win = match overlay_win {
        Some(w) => w,
        None => return,
    };

    let client_info = ctx.client(win).map(|c| (c.isfloating, c.border_width));

    if let Some((is_floating, bw)) = client_info {
        if is_floating {
            save_floating(ctx, win);
        }
        let geo = Rect {
            x: work_rect.x,
            y: work_rect.y,
            w: work_rect.w - 2 * bw,
            h: work_rect.h - 2 * bw,
        };
        ctx.resize_client(win, geo);
    }
}

pub fn restack(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    if ctx
        .g
        .monitor(mon_id)
        .map_or(false, |m| get_current_layout(ctx.g, m).is_overview())
    {
        return;
    }
    draw_bar(ctx, mon_id);

    let m = ctx.g.monitor(mon_id).expect("invalid monitor");
    let sel_win = match m.sel {
        Some(w) => w,
        None => return,
    };
    let is_tiling = get_current_layout(ctx.g, m).is_tiling();
    let selected_tags = m.selected_tags();
    let barwin = m.barwin;
    let stack_head = m.stack;

    let is_floating = ctx.client(sel_win).map_or(false, |c| c.isfloating);

    if is_floating {
        ctx.raise(sel_win);
    }

    if !is_tiling {
        ctx.raise(sel_win);
        ctx.flush();
        return;
    }

    let mut tiled_stack = Vec::new();
    let mut floating_stack = Vec::new();
    let mut s_win = stack_head;
    while let Some(win) = s_win {
        if let Some(c) = ctx.client(win) {
            if c.is_visible_on_tags(selected_tags) {
                if c.isfloating {
                    floating_stack.push(win);
                } else {
                    tiled_stack.push(win);
                }
            }
            s_win = c.snext;
        } else {
            break;
        }
    }

    let mut stack = tiled_stack;
    stack.push(barwin);
    if is_floating {
        stack.retain(|w| *w != sel_win);
        stack.push(sel_win);
    }
    stack.extend(floating_stack);
    ctx.restack(&stack);
    ctx.flush();
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: LayoutKind) {
    if ctx.g.tags.prefix {
        for (_i, mon) in ctx.g.monitors_iter_mut() {
            for tag in mon.tags.iter_mut() {
                tag.layouts.set_layout(layout);
            }
        }
        ctx.g.tags.prefix = false;
    } else if let Some(m) = ctx.g.selmon_mut() {
        let tag = m.current_tag;
        if tag > 0 && tag <= m.tags.len() {
            m.tags[tag - 1].layouts.set_layout(layout);
        }
    }
    finish_layout_change(ctx);
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    if ctx.g.tags.prefix {
        for (_i, mon) in ctx.g.monitors_iter_mut() {
            for tag in mon.tags.iter_mut() {
                tag.layouts.toggle_slot();
            }
        }
        ctx.g.tags.prefix = false;
    } else if let Some(m) = ctx.g.selmon_mut() {
        let tag = m.current_tag;
        if tag > 0 && tag <= m.tags.len() {
            m.tags[tag - 1].layouts.toggle_slot();
        }
    }
    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selmon = ctx.g.selmon_id();
    if ctx.g.selmon().and_then(|m| m.sel).is_some() {
        arrange(ctx, Some(selmon));
    } else {
        draw_bar(ctx, selmon);
    }
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = ctx.g.selmon().map(|m| get_current_layout(ctx.g, m));
    let all_layouts = LayoutKind::all();
    let layouts_len = all_layouts.len();
    let current_idx = current_layout
        .map(|l| all_layouts.iter().position(|&x| x == l).unwrap_or(0))
        .unwrap_or(0);

    let candidate = if forward {
        (current_idx + 1) % layouts_len
    } else {
        if current_idx == 0 {
            layouts_len - 1
        } else {
            current_idx - 1
        }
    };
    let final_layout = if all_layouts[candidate].is_overview() {
        let final_idx = if forward {
            (candidate + 1) % layouts_len
        } else {
            if candidate == 0 {
                layouts_len - 1
            } else {
                candidate - 1
            }
        };
        all_layouts[final_idx]
    } else {
        all_layouts[candidate]
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
    if let Some(m) = ctx.g.selmon_mut() {
        if delta > 0 && m.nmaster >= ccount {
            m.nmaster = ccount;
        } else {
            let new_nmaster = max(m.nmaster + delta, 0);
            m.nmaster = new_nmaster;
            let tag = m.current_tag;
            if tag > 0 && tag <= m.tags.len() {
                m.tags[tag - 1].nmaster = new_nmaster;
            }
        }
    }
    let selmon = ctx.g.selmon_id();
    arrange(ctx, Some(selmon));
}

pub fn set_mfact(ctx: &mut WmCtx<'_>, mfact_val: f32) {
    if mfact_val == 0.0 {
        return;
    }
    let is_tiling = ctx
        .g
        .selmon()
        .map_or(false, |m| get_current_layout(ctx.g, m).is_tiling());
    if !is_tiling {
        return;
    }

    let current_mfact = ctx.g.selmon().map_or(0.55, |m| m.mfact);
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

    if let Some(m) = ctx.g.selmon_mut() {
        m.mfact = new_mfact;
        let tag = m.current_tag;
        if tag > 0 && tag <= m.tags.len() {
            m.tags[tag - 1].mfact = new_mfact;
        }
    }

    let selmon = ctx.g.selmon_id();
    arrange(ctx, Some(selmon));
    if animation_on {
        ctx.g.animated = true;
    }
}
