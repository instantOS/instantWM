//! Layout manager — the stateful half of the layout system.

use crate::contexts::WmCtx;
use crate::floating::save_floating_geometry;
use crate::geometry::MoveResizeOptions;
use crate::types::{MonitorId, Rect, WindowId};
use std::cmp::max;

use super::LayoutKind;

pub fn arrange(ctx: &mut WmCtx<'_>, monitor_id: Option<MonitorId>) {
    crate::mouse::reset_cursor(ctx);

    if let Some(id) = monitor_id {
        // First pass: show/hide stack
        crate::client::apply_visibility(ctx);
        // Second pass: arrange and restack
        arrange_monitor(ctx, id);
        restack(ctx, id);
    } else {
        crate::client::apply_visibility(ctx);

        let mon_indices: Vec<MonitorId> = (0..ctx.core().globals().monitors.count())
            .map(MonitorId)
            .collect();
        for idx in mon_indices {
            arrange_monitor(ctx, idx);
            restack(ctx, idx);
        }
    }

    ctx.request_space_sync();
    ctx.flush();
}

pub fn arrange_monitor(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    let clientcount = {
        let m = ctx
            .core()
            .globals()
            .monitor(monitor_id)
            .expect("invalid monitor");
        m.tiled_client_count(ctx.core().globals().clients.map()) as u32
    };

    if let Some(m) = ctx.core_mut().globals_mut().monitor_mut(monitor_id) {
        m.clientcount = clientcount;
    }

    let Some(monitor_before_layout) = ctx.core().globals().monitor(monitor_id).cloned() else {
        return;
    };

    apply_border_widths(ctx, &monitor_before_layout);
    {
        let bar_height = ctx.core().globals().cfg.bar_height;
        let mon = ctx
            .core_mut()
            .globals_mut()
            .monitor_mut(monitor_id)
            .unwrap();
        let (nmaster, mfact) = {
            let pertag = mon.pertag_state();
            (pertag.nmaster, pertag.mfact)
        };
        mon.nmaster = nmaster;
        mon.mfact = mfact;
        mon.update_bar_position(bar_height);
    }
    run_layout(ctx, monitor_id);
    {
        let mon = ctx
            .core_mut()
            .globals_mut()
            .monitor_mut(monitor_id)
            .unwrap();
        let (nmaster, mfact) = (mon.nmaster, mon.mfact);
        let pertag = mon.pertag_state();
        pertag.nmaster = nmaster;
        pertag.mfact = mfact;
    }

    let Some(monitor_after_layout) = ctx.core().globals().monitor(monitor_id).cloned() else {
        return;
    };

    apply_fullscreen(ctx, &monitor_after_layout);
    place_overlay(ctx, &monitor_after_layout);
}

fn apply_fullscreen(ctx: &mut WmCtx<'_>, monitor: &crate::types::Monitor) {
    let mon_rect = monitor.monitor_rect;
    let clients = monitor.clients.clone();
    let selected_tags = monitor.selected_tags();

    let fullscreen_windows: Vec<_> = clients
        .into_iter()
        .filter(|&win| {
            ctx.client(win)
                .is_some_and(|c| c.is_true_fullscreen() && c.is_visible(selected_tags))
        })
        .collect();

    for win in fullscreen_windows {
        ctx.move_resize(win, mon_rect, MoveResizeOptions::immediate());
    }
}

fn apply_border_widths(ctx: &mut WmCtx<'_>, monitor: &crate::types::Monitor) {
    let is_tiling = monitor.current_layout().is_tiling();
    let is_monocle = monitor.current_layout().is_monocle();
    let clientcount = monitor.clientcount;
    let selected_tags = monitor.selected_tags();

    // Collect border changes first to avoid borrow conflicts
    let border_changes: Vec<(WindowId, i32)> = monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let info = ctx.client(win)?;
            let is_visible = info.is_visible(selected_tags);
            if !is_visible {
                return None;
            }

            let strip_border = info.is_true_fullscreen()
                || (!info.is_floating
                    && !info.is_fullscreen
                    && ((clientcount == 1 && is_tiling) || is_monocle));

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
    let layout = ctx
        .core()
        .globals()
        .monitor(monitor_id)
        .map(|m| m.current_layout());
    if let Some(layout) = layout
        && let Some(mut m) = ctx.core().globals().monitor(monitor_id).cloned()
    {
        layout.arrange(ctx, &mut m);
        ctx.core_mut()
            .globals_mut()
            .monitors
            .set_monitor(monitor_id, m);
    }
}

fn place_overlay(ctx: &mut WmCtx<'_>, monitor: &crate::types::Monitor) {
    let overlay_win = monitor.overlay;
    let work_rect = monitor.work_rect;

    let win = match overlay_win {
        Some(w) => w,
        None => return,
    };

    let client_info = ctx.client(win).map(|c| (c.is_floating, c.border_width));

    if let Some((is_floating, bw)) = client_info {
        if is_floating && let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            save_floating_geometry(client);
        }
        let geo = Rect {
            x: work_rect.x,
            y: work_rect.y,
            w: work_rect.w - 2 * bw,
            h: work_rect.h - 2 * bw,
        };
        ctx.move_resize(win, geo, MoveResizeOptions::immediate());
    }
}

pub fn restack(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    ctx.request_bar_update(Some(monitor_id));

    let Some(monitor) = ctx.core().globals().monitor(monitor_id) else {
        return;
    };
    if monitor.current_layout().is_overview() {
        return;
    }

    let selected_window = match monitor.sel {
        Some(win) => win,
        None => return,
    };
    let layout = monitor.current_layout();
    let is_tiling = layout.is_tiling();
    let is_monocle = layout.is_monocle();
    let selected_tags = monitor.selected_tags();
    let bar_win = monitor.bar_win;

    if !is_tiling {
        ctx.raise(selected_window);
        ctx.flush();
        return;
    }

    let mut tiled_stack = Vec::new();
    let mut floating_stack = Vec::new();
    let mut fullscreen_stack = Vec::new();
    for &win in &monitor.stack {
        if let Some(c) = ctx.client(win)
            && c.is_visible(selected_tags)
        {
            if c.is_true_fullscreen() {
                fullscreen_stack.push(win);
            } else if c.is_floating {
                floating_stack.push(win);
            } else {
                tiled_stack.push(win);
            }
        }
    }

    if let Some(idx) = fullscreen_stack
        .iter()
        .position(|&win| win == selected_window)
    {
        let selected = fullscreen_stack.remove(idx);
        fullscreen_stack.push(selected);
    } else if let Some(idx) = floating_stack
        .iter()
        .position(|&win| win == selected_window)
    {
        let selected = floating_stack.remove(idx);
        floating_stack.push(selected);
    } else {
        // In monocle every tiled client occupies the full work area, so the
        // focused tiled client must be the last tiled element in z-order.
        // Keeping this explicit also makes the generic tiled case easier to read.
        if let Some(idx) = tiled_stack.iter().position(|&win| win == selected_window) {
            let selected = tiled_stack.remove(idx);
            tiled_stack.push(selected);
        }
        if is_monocle && tiled_stack.last().copied() != Some(selected_window) {
            tiled_stack.retain(|&win| win != selected_window);
            tiled_stack.push(selected_window);
        }
    }

    // Final z-order: tiled clients, then the bar, then floating clients,
    // and finally fullscreen clients.
    // This keeps every floating window above tiled content while still
    // keeping the selected window topmost within its own class, and guarantees
    // fullscreen windows sit above everything else.
    let mut stack = tiled_stack;
    stack.push(bar_win);
    stack.extend(floating_stack);
    stack.extend(fullscreen_stack);
    ctx.restack(&stack);
    ctx.flush();
}

pub fn set_layout(ctx: &mut WmCtx<'_>, layout: LayoutKind) {
    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    m.pertag_state().layouts.set_layout(layout);
    finish_layout_change(ctx);
}

pub fn toggle_layout(ctx: &mut WmCtx<'_>) {
    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    m.pertag_state().layouts.toggle_slot();
    finish_layout_change(ctx);
}

fn finish_layout_change(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn cycle_layout_direction(ctx: &mut WmCtx<'_>, forward: bool) {
    let current_layout = ctx.core().globals().selected_monitor().current_layout();
    let all_layouts = LayoutKind::all();
    let layouts_len = all_layouts.len();
    let current_idx = all_layouts
        .iter()
        .position(|&x| x == current_layout)
        .unwrap_or(0);

    let candidate = if forward {
        (current_idx + 1) % layouts_len
    } else if current_idx == 0 {
        layouts_len - 1
    } else {
        current_idx - 1
    };
    let final_layout = if all_layouts[candidate].is_overview() {
        let final_idx = if forward {
            (candidate + 1) % layouts_len
        } else if candidate == 0 {
            layouts_len - 1
        } else {
            candidate - 1
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
        .core()
        .globals()
        .selected_monitor()
        .tiled_client_count(ctx.core().globals().clients.map()) as i32;
    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    if delta > 0 && m.nmaster >= ccount {
        m.nmaster = ccount;
    } else {
        let new_nmaster = max(m.nmaster + delta, 0);
        m.nmaster = new_nmaster;
    }
    m.pertag_state().nmaster = m.nmaster;
    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
}

pub fn set_mfact(ctx: &mut WmCtx<'_>, mfact_val: f32) {
    if mfact_val == 0.0 {
        return;
    }
    let is_tiling = ctx
        .core()
        .globals()
        .selected_monitor()
        .current_layout()
        .is_tiling();
    if !is_tiling {
        return;
    }

    let current_mfact = ctx.core().globals().selected_monitor().mfact;
    let new_mfact = if mfact_val < 1.0 {
        mfact_val + current_mfact
    } else {
        mfact_val - 1.0
    };
    if !(0.05..=0.95).contains(&new_mfact) {
        return;
    }

    let animation_on = ctx.core().globals().behavior.animated
        && ctx
            .core()
            .globals()
            .selected_monitor()
            .tiled_client_count(ctx.core().globals().clients.map())
            > 1;
    if animation_on {
        ctx.core_mut().globals_mut().behavior.animated = false;
    }

    let m = ctx.core_mut().globals_mut().selected_monitor_mut();
    m.mfact = new_mfact;
    m.pertag_state().mfact = new_mfact;

    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selected_monitor_id));
    if animation_on {
        ctx.core_mut().globals_mut().behavior.animated = true;
    }
}
