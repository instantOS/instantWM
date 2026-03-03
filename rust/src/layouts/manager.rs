//! Layout manager — the stateful half of the layout system.

use crate::backend::BackendOps;
use crate::bar::draw_bar;
use crate::client::{resize, restore_border_width_ctx, save_border_width_ctx};
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
        let stack = ctx.g.monitor(id).map(|m| m.stack);
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
        let stacks: Vec<Option<WindowId>> = ctx.g.monitors.iter().map(|m| m.stack).collect();

        for stack in stacks {
            crate::client::show_hide(ctx, stack);
        }

        let mon_indices: Vec<usize> = (0..ctx.g.monitors.len()).collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
            restack(ctx, idx);
        }
        // NOTE: arrange(None) intentionally uses a raw index loop because it
        // needs to call mutable methods between iterations; monitors_iter_mut
        // cannot be used here without borrow conflicts.
    }

    // Batch-flush all backend operations from this layout pass.
    ctx.backend.flush();
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    // Get client count first
    let clientcount = {
        let m = ctx.g.monitor(mon_id).expect("invalid monitor");
        client_count_mon(ctx.g, m) as u32
    };

    // Update client count
    if let Some(m) = ctx.g.monitor_mut(mon_id) {
        m.clientcount = clientcount;
    }

    // Apply border widths
    apply_border_widths(ctx, mon_id);

    // Run layout
    run_layout(ctx, mon_id);

    // Place overlay
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
        let (is_floating, is_fullscreen, is_visible, is_hidden) = match ctx.g.clients.get(&win) {
            None => break,
            Some(c) => (
                c.isfloating,
                c.is_fullscreen,
                c.is_visible_on_tags(selected_tags),
                c.is_hidden,
            ),
        };

        if is_visible && !is_hidden {
            let strip_border =
                !is_floating && !is_fullscreen && ((clientcount == 1 && is_tiling) || is_monocle);

            if strip_border {
                save_border_width_ctx(ctx, win);
                if let Some(c) = ctx.g.clients.get_mut(&win) {
                    c.border_width = 0;
                }
            } else {
                let old_bw = ctx.g.clients.get(&win).map(|c| c.border_width).unwrap_or(0);
                restore_border_width_ctx(ctx, win);
                let new_bw = ctx.g.clients.get(&win).map(|c| c.border_width).unwrap_or(0);

                if old_bw != new_bw {
                    ctx.backend.set_border_width(win, new_bw);
                }
            }
        }

        c_win = ctx.g.clients.get(&win).and_then(|c| c.next);
    }
}

fn run_layout(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let layout = ctx.g.monitor(mon_id).map(|m| get_current_layout(ctx.g, m));
    if let Some(layout) = layout {
        // Clone the monitor so that `layout.arrange` can receive both
        // `&mut WmCtx` and `&mut Monitor` without a split-borrow conflict.
        // Layout algorithms only modify clients via `ctx.g.clients` and read
        // monitor data from `m`; restack is handled by the caller.
        if let Some(mut m) = ctx.g.monitor(mon_id).cloned() {
            layout.arrange(ctx, &mut m);
            ctx.g.monitors[mon_id] = m;
        }
    }
}

fn place_overlay(ctx: &mut WmCtx<'_>, mon_id: MonitorId) {
    let overlay_win = match ctx.g.monitor(mon_id).and_then(|m| m.overlay) {
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

    let geo = if let Some(m) = ctx.g.monitor(mon_id) {
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
        .monitor(mon_id)
        .map(|m| get_current_layout(ctx.g, m).is_overview())
        .unwrap_or(false);
    if is_overview {
        return;
    }
    draw_bar(ctx, mon_id);

    let m = ctx.g.monitor_mut(mon_id).expect("invalid monitor");
    let sel_win = match m.sel {
        Some(w) => w,
        None => return,
    };

    let is_tiling = ctx
        .g
        .monitor(mon_id)
        .map(|mon| get_current_layout(ctx.g, mon).is_tiling())
        .unwrap_or(true);

    // Extract the fields we need from the monitor before we start borrowing
    // `ctx.g.clients`, since holding a `&mut Monitor` and a `&clients` at the
    // same time would be a simultaneous mutable + immutable borrow of `ctx.g`.
    let (barwin, stack_head, selected_tags) = {
        let m = ctx.g.monitor_mut(mon_id).expect("invalid monitor");
        (m.barwin, m.stack, m.selected_tags())
    };
    let is_floating = ctx
        .g
        .clients
        .get(&sel_win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        ctx.backend.raise_window(sel_win);
    }

    if !is_tiling {
        ctx.backend.raise_window(sel_win);
        ctx.backend.flush();
        return;
    }

    let mut stack: Vec<WindowId> = Vec::new();
    let mut s_win = stack_head;
    while let Some(win) = s_win {
        match ctx.g.clients.get(&win) {
            None => break,
            Some(c) => {
                let is_win_floating = c.isfloating;
                let visible = c.is_visible_on_tags(selected_tags);
                let snext = c.snext;

                if !is_win_floating && visible {
                    stack.push(win);
                }

                s_win = snext;
            }
        }
    }

    stack.push(barwin);
    if is_floating {
        stack.push(sel_win);
    }
    ctx.backend.restack(&stack);
    ctx.backend.flush();
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: LayoutKind) {
    let tagprefix = ctx.g.tags.prefix;

    if tagprefix {
        for mon in ctx.g.monitors.iter_mut() {
            for tag in mon.tags.iter_mut() {
                tag.layouts.set_layout(layout);
            }
        }
        ctx.g.tags.prefix = false;
        finish_layout_change(ctx);
        return;
    }

    if let Some(m) = ctx.g.selmon_mut() {
        let current_tag = m.current_tag;
        if current_tag > 0 && current_tag <= m.tags.len() {
            m.tags[current_tag - 1].layouts.set_layout(layout);
        }
    }

    finish_layout_change(ctx);
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    let tagprefix = ctx.g.tags.prefix;

    if tagprefix {
        for mon in ctx.g.monitors.iter_mut() {
            for tag in mon.tags.iter_mut() {
                tag.layouts.toggle_slot();
            }
        }
        ctx.g.tags.prefix = false;
        finish_layout_change(ctx);
        return;
    }

    if let Some(m) = ctx.g.selmon_mut() {
        let current_tag = m.current_tag;
        if current_tag > 0 && current_tag <= m.tags.len() {
            m.tags[current_tag - 1].layouts.toggle_slot();
        }
    }

    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selmon = ctx.g.selmon_id();
    let sel = ctx.g.selmon().and_then(|m| m.sel);

    if sel.is_some() {
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
        if let Some(m) = ctx.g.selmon_mut() {
            if delta > 0 && m.nmaster >= ccount {
                m.nmaster = ccount;
                return;
            }

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
        .map(|m| get_current_layout(ctx.g, m).is_tiling())
        .unwrap_or(false);

    if !is_tiling {
        return;
    }

    let current_mfact = ctx.g.selmon().map(|m| m.mfact).unwrap_or(0.55);

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
        if let Some(m) = ctx.g.selmon_mut() {
            m.mfact = new_mfact;
            let tag = m.current_tag;
            if tag > 0 && tag <= m.tags.len() {
                m.tags[tag - 1].mfact = new_mfact;
            }
        }
    }

    let selmon = ctx.g.selmon_id();
    arrange(ctx, Some(selmon));

    if animation_on {
        ctx.g.animated = true;
    }
}
