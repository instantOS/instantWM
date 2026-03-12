//! Layout manager — the stateful half of the layout system.

use crate::contexts::WmCtx;
use crate::layouts::algo::save_floating;
use crate::types::{MonitorId, Rect, WindowId};
use std::cmp::max;

use super::LayoutKind;

pub fn arrange(ctx: &mut WmCtx<'_>, monitor_id: Option<MonitorId>) {
    crate::mouse::reset_cursor(ctx);

    if let Some(id) = monitor_id {
        // First pass: show/hide stack
        crate::client::show_hide(ctx);
        // Second pass: arrange and restack
        arrange_monitor(ctx, id);
        restack(ctx, id);
    } else {
        crate::client::show_hide(ctx);

        let mon_indices: Vec<usize> = (0..ctx.g().monitors.count()).collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
            restack(ctx, idx);
        }
    }

    ctx.g_mut().layout_dirty = false;
    ctx.g_mut().space_dirty = true;
    ctx.flush();
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let clientcount = {
        let m = ctx.g().monitor(monitor_id).expect("invalid monitor");
        m.tiled_client_count(&*ctx.g().clients) as u32
    };

    if let Some(m) = ctx.g_mut().monitor_mut(monitor_id) {
        m.clientcount = clientcount;
    }

    apply_border_widths(ctx, monitor_id);
    run_layout(ctx, monitor_id);
    place_overlay(ctx, monitor_id);
}

fn apply_border_widths(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let m = match ctx.g().monitor(monitor_id) {
        Some(m) => m,
        None => return,
    };

    let is_tiling = m.current_layout().is_tiling();
    let is_monocle = m.current_layout().is_monocle();
    let clientcount = m.clientcount;
    let selected_tags = m.selected_tags();

    // Collect border changes first to avoid borrow conflicts
    let border_changes: Vec<(WindowId, i32)> = m
        .clients
        .iter()
        .filter_map(|&win| {
            let info = ctx.client(win)?;
            let is_visible = info.is_visible_on_tags(selected_tags) && !info.is_hidden;
            if !is_visible {
                return None;
            }

            let strip_border = !info.is_floating
                && !info.is_fullscreen
                && ((clientcount == 1 && is_tiling) || is_monocle);

            let new_border = if strip_border {
                0
            } else {
                info.old_border_width
            };
            Some((win, new_border))
        })
        .collect();

    // Apply border changes
    for (win, border) in border_changes {
        ctx.set_border(win, border);
    }
}

fn run_layout(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let layout = ctx.g().monitor(monitor_id).map(|m| m.current_layout());
    if let Some(layout) = layout {
        if let Some(mut m) = ctx.g().monitor(monitor_id).cloned() {
            layout.arrange(ctx, &mut m);
            ctx.g_mut().monitors.set_monitor(monitor_id, m);
        }
    }
}

fn place_overlay(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let (overlay_win, work_rect) = match ctx.g().monitor(monitor_id) {
        Some(m) => (m.overlay, m.work_rect),
        None => return,
    };

    let win = match overlay_win {
        Some(w) => w,
        None => return,
    };

    let client_info = ctx.client(win).map(|c| (c.is_floating, c.border_width));

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

pub fn restack(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    ctx.request_bar_update(Some(monitor_id));

    if ctx
        .g()
        .monitor(monitor_id)
        .map_or(false, |m| m.current_layout().is_overview())
    {
        return;
    }

    // Extract data from monitor first to avoid borrow conflicts
    let (selected_window, is_tiling, selected_tags, bar_win, is_floating) = {
        let m = ctx.g().monitor(monitor_id).expect("invalid monitor");
        let selected_window = match m.sel {
            Some(w) => w,
            None => return,
        };
        let is_tiling = m.current_layout().is_tiling();
        let selected_tags = m.selected_tags();
        let bar_win = m.bar_win;
        let is_floating = ctx.client(selected_window).map_or(false, |c| c.is_floating);
        (
            selected_window,
            is_tiling,
            selected_tags,
            bar_win,
            is_floating,
        )
    };

    if is_floating {
        ctx.raise(selected_window);
    }

    if !is_tiling {
        ctx.raise(selected_window);
        ctx.flush();
        return;
    }

    let mut tiled_stack = Vec::new();
    let mut floating_stack = Vec::new();
    if let Some(m) = ctx.g().monitor(monitor_id) {
        for &win in &m.stack {
            if let Some(c) = ctx.client(win) {
                if c.is_visible_on_tags(selected_tags) {
                    if c.is_floating {
                        floating_stack.push(win);
                    } else {
                        tiled_stack.push(win);
                    }
                }
            }
        }
    }

    let mut stack = tiled_stack;
    stack.push(bar_win);
    stack.extend(floating_stack);
    ctx.restack(&stack);
    ctx.flush();
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: LayoutKind) {
    if ctx.g().tags.prefix {
        for mon in ctx.g_mut().monitors_iter_all_mut() {
            for tag in mon.tags.iter_mut() {
                tag.layouts.set_layout(layout);
            }
        }
        ctx.g_mut().tags.prefix = false;
    } else {
        let m = ctx.g_mut().selected_monitor_mut();
        let tag = m.current_tag;
        if tag > 0 && tag <= m.tags.len() {
            m.tags[tag - 1].layouts.set_layout(layout);
        }
    }
    finish_layout_change(ctx);
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    if ctx.g().tags.prefix {
        for mon in ctx.g_mut().monitors_iter_all_mut() {
            for tag in mon.tags.iter_mut() {
                tag.layouts.toggle_slot();
            }
        }
        ctx.g_mut().tags.prefix = false;
    } else {
        let m = ctx.g_mut().selected_monitor_mut();
        let tag = m.current_tag;
        if tag > 0 && tag <= m.tags.len() {
            m.tags[tag - 1].layouts.toggle_slot();
        }
    }
    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.g().selected_monitor_id();
    if ctx.g().selected_monitor().sel.is_some() {
        arrange(ctx, Some(selected_monitor_id));
    } else {
        ctx.request_bar_update(Some(selected_monitor_id));
    }
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = ctx.g().selected_monitor().current_layout();
    let all_layouts = LayoutKind::all();
    let layouts_len = all_layouts.len();
    let current_idx = all_layouts
        .iter()
        .position(|&x| x == current_layout)
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
    let ccount = ctx
        .g()
        .selected_monitor()
        .tiled_client_count(&*ctx.g().clients) as i32;
    let m = ctx.g_mut().selected_monitor_mut();
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
    let selected_monitor_id = ctx.g().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn set_mfact(ctx: &mut WmCtx<'_>, mfact_val: f32) {
    if mfact_val == 0.0 {
        return;
    }
    let is_tiling = ctx.g().selected_monitor().current_layout().is_tiling();
    if !is_tiling {
        return;
    }

    let current_mfact = ctx.g().selected_monitor().mfact;
    let new_mfact = if mfact_val < 1.0 {
        mfact_val + current_mfact
    } else {
        mfact_val - 1.0
    };
    if !(0.05..=0.95).contains(&new_mfact) {
        return;
    }

    let animation_on = ctx.g().animated
        && ctx
            .g()
            .selected_monitor()
            .tiled_client_count(&*ctx.g().clients)
            > 2;
    if animation_on {
        ctx.g_mut().animated = false;
    }

    let m = ctx.g_mut().selected_monitor_mut();
    m.mfact = new_mfact;
    let tag = m.current_tag;
    if tag > 0 && tag <= m.tags.len() {
        m.tags[tag - 1].mfact = new_mfact;
    }

    let selected_monitor_id = ctx.g().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
    if animation_on {
        ctx.g_mut().animated = true;
    }
}
