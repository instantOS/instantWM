//! Layout manager — the stateful half of the layout system.

use crate::bar::draw_bar;
use crate::client::{next_tiled, resize, restore_border_width, save_border_width};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::algo::save_floating;
use crate::layouts::query::{client_count, client_count_mon, get_current_layout};
use crate::types::{Monitor, MonitorId, Rect};
use std::cmp::max;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::LayoutKind;

pub fn arrange(mon_id: Option<MonitorId>) {
    crate::mouse::reset_cursor();

    if let Some(id) = mon_id {
        {
            let g = get_globals_mut();
            if let Some(m) = g.monitors.get_mut(id) {
                let stack = m.stack;
                crate::client::show_hide(stack);
            }
        }
        {
            let g = get_globals_mut();
            if let Some(m) = g.monitors.get_mut(id) {
                arrange_monitor(m);
                restack(m);
            }
        }
    } else {
        let stacks: Vec<Option<Window>> = {
            let g = get_globals();
            g.monitors.iter().map(|m| m.stack).collect()
        };

        for stack in stacks {
            crate::client::show_hide(stack);
        }

        let g = get_globals_mut();
        for m in g.monitors.iter_mut() {
            arrange_monitor(m);
        }
    }
}

pub fn arrange_monitor(m: &mut Monitor) {
    m.clientcount = client_count_mon(m) as u32;
    apply_border_widths(m);
    run_layout(m);
    place_overlay(m);
}

fn apply_border_widths(m: &Monitor) {
    let is_tiling = get_current_layout(m).is_tiling();
    let is_monocle = get_current_layout(m).is_monocle();
    let clientcount = m.clientcount;

    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (is_floating, is_fullscreen) = {
            let g = get_globals();
            match g.clients.get(&win) {
                None => break,
                Some(c) => (c.isfloating, c.is_fullscreen),
            }
        };

        let strip_border =
            !is_floating && !is_fullscreen && ((clientcount == 1 && is_tiling) || is_monocle);

        if strip_border {
            save_border_width(win);
            if let Some(c) = get_globals_mut().clients.get_mut(&win) {
                c.border_width = 0;
            }
        } else {
            restore_border_width(win);
        }

        c_win = get_globals()
            .clients
            .get(&win)
            .and_then(|c| next_tiled(c.next));
    }
}

fn run_layout(m: &mut Monitor) {
    get_current_layout(m).arrange(m);
}

fn place_overlay(m: &mut Monitor) {
    let overlay_win = match m.overlay {
        Some(w) => w,
        None => return,
    };

    let g = get_globals_mut();
    if let Some(c) = g.clients.get_mut(&overlay_win) {
        if c.isfloating {
            save_floating(overlay_win);
        }
    }

    let bw = g.clients.get(&overlay_win).map_or(0, |c| c.border_width);
    let geo = Rect {
        x: m.work_rect.x,
        y: m.work_rect.y,
        w: m.work_rect.w - 2 * bw,
        h: m.work_rect.h - 2 * bw,
    };

    resize(overlay_win, &geo, false);
}

pub fn restack(m: &mut Monitor) {
    if get_current_layout(m).is_overview() {
        return;
    }

    draw_bar(m);

    let sel_win = match m.sel {
        Some(w) => w,
        None => return,
    };

    let is_tiling = get_current_layout(m).is_tiling();

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let is_floating = get_globals()
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
                let g = get_globals();
                match g.clients.get(&win) {
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

pub fn set_layout(layout: LayoutKind) {
    let tagprefix = get_globals().tags.prefix;

    if tagprefix {
        {
            let g = get_globals_mut();
            for tag in g.tags.tags.iter_mut() {
                tag.layouts.set_layout(layout);
            }
            g.tags.prefix = false;
        }
        finish_layout_change();
        return;
    }

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            let current_tag = m.current_tag;
            if current_tag > 0 && current_tag <= g.tags.tags.len() {
                let tag = &mut g.tags.tags[current_tag - 1];
                tag.layouts.set_layout(layout);
            }
        }
    }

    finish_layout_change();
}

pub fn toggle_layout() {
    let tagprefix = get_globals().tags.prefix;

    if tagprefix {
        {
            let g = get_globals_mut();
            for tag in g.tags.tags.iter_mut() {
                tag.layouts.toggle_slot();
            }
            g.tags.prefix = false;
        }
        finish_layout_change();
        return;
    }

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            let current_tag = m.current_tag;
            if current_tag > 0 && current_tag <= g.tags.tags.len() {
                let tag = &mut g.tags.tags[current_tag - 1];
                tag.layouts.toggle_slot();
            }
        }
    }

    finish_layout_change();
}

pub fn restore_last_layout() {
    let tagprefix = get_globals().tags.prefix;

    if tagprefix {
        {
            let g = get_globals_mut();
            for tag in g.tags.tags.iter_mut() {
                tag.layouts.restore_last_layout();
            }
            g.tags.prefix = false;
        }
        finish_layout_change();
        return;
    }

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            let current_tag = m.current_tag;
            if current_tag > 0 && current_tag <= g.tags.tags.len() {
                let tag = &mut g.tags.tags[current_tag - 1];
                tag.layouts.restore_last_layout();
            }
        }
    }

    finish_layout_change();
}

fn finish_layout_change() {
    let (selmon, sel) = {
        let g = get_globals();
        let sel = g.monitors.get(g.selmon).and_then(|m| m.sel);
        (g.selmon, sel)
    };

    if sel.is_some() {
        arrange(Some(selmon));
    } else {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(selmon) {
            draw_bar(m);
        }
    }
}

pub fn cycle_layout_direction(forward: bool) {
    let current_layout = {
        let g = get_globals();
        g.monitors.get(g.selmon).map(|m| get_current_layout(m))
    };

    let layouts_len = get_globals().layouts.len();
    if layouts_len == 0 {
        return;
    }

    let current_symbol = current_layout.map(|l| l.symbol());

    let g = get_globals();
    let current_idx = current_symbol
        .and_then(|sym| g.layouts.iter().position(|l| l.symbol() == sym))
        .unwrap_or(0);

    let candidate = if forward {
        (current_idx + 1) % layouts_len
    } else if current_idx == 0 {
        layouts_len - 1
    } else {
        current_idx - 1
    };

    let skip = {
        let g = get_globals();
        g.layouts.get(candidate).is_some_and(|l| l.is_overview())
    };

    let final_idx = if skip {
        if forward {
            (candidate + 1) % layouts_len
        } else if candidate == 0 {
            layouts_len - 1
        } else {
            candidate - 1
        }
    } else {
        candidate
    };

    if let Some(&layout) = get_globals().layouts.get(final_idx) {
        set_layout(layout);
    }
}

pub fn command_layout(layout_idx: u32) {
    let g = get_globals();
    let layouts_len = g.layouts.len();
    let idx = if layout_idx > 0 && (layout_idx as usize) < layouts_len {
        layout_idx as usize
    } else {
        0
    };

    if let Some(&layout) = g.layouts.get(idx) {
        set_layout(layout);
    }
}

pub fn inc_nmaster_by(delta: i32) {
    let ccount = client_count();

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            if delta > 0 && m.nmaster >= ccount {
                m.nmaster = ccount;
                return;
            }

            let new_nmaster = max(m.nmaster + delta, 0);
            m.nmaster = new_nmaster;

            let tag = m.current_tag;
            if tag > 0 && tag <= g.tags.tags.len() {
                g.tags.tags[tag - 1].nmaster = new_nmaster;
            }
        }
    }

    let selmon = get_globals().selmon;
    arrange(Some(selmon));
}

pub fn set_mfact(mfact_val: f32) {
    if mfact_val == 0.0 {
        return;
    }

    let is_tiling = {
        let g = get_globals();
        g.monitors
            .get(g.selmon)
            .map(|m| get_current_layout(m).is_tiling())
            .unwrap_or(false)
    };

    if !is_tiling {
        return;
    }

    let current_mfact = {
        let g = get_globals();
        g.monitors.get(g.selmon).map(|m| m.mfact).unwrap_or(0.55)
    };

    let new_mfact = if mfact_val < 1.0 {
        mfact_val + current_mfact
    } else {
        mfact_val - 1.0
    };

    if !(0.05..=0.95).contains(&new_mfact) {
        return;
    }

    let animation_on = get_globals().animated && client_count() > 2;
    if animation_on {
        get_globals_mut().animated = false;
    }

    {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(g.selmon) {
            m.mfact = new_mfact;
            let tag = m.current_tag;
            if tag > 0 && tag <= g.tags.tags.len() {
                g.tags.tags[tag - 1].mfact = new_mfact;
            }
        }
    }

    let selmon = get_globals().selmon;
    arrange(Some(selmon));

    if animation_on {
        get_globals_mut().animated = true;
    }
}
