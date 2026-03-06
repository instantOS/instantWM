use crate::contexts::CoreCtx;
use crate::globals::Globals;
use crate::tags::{get_tag_at_x, get_tag_width};
use crate::types::*;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientBarStats {
    pub occupied_tags: u32,
    pub urgent_tags: u32,
    pub visible_clients: i32,
}

impl ClientBarStats {
    /// Collect bar statistics for the given monitor.
    pub(crate) fn collect(monitor: &Monitor, globals: &Globals) -> Self {
        let mut stats = Self::default();
        let selected = monitor.selected_tags();

        // ── Pass 1: visible_clients via the linked list ───────────────────
        for (_win, client) in monitor.iter_clients(globals.clients.map()) {
            if client.is_visible_on_tags(selected) {
                stats.visible_clients += 1;
            }
        }

        // ── Pass 2: occupied / urgent tag bits from all clients on this monitor
        let monitor_id = monitor.id();
        for client in globals.clients.values() {
            if client.monitor_id != Some(monitor_id) {
                continue;
            }
            stats.occupied_tags |= if client.tags == 255 { 0 } else { client.tags };
            if client.isurgent {
                stats.urgent_tags |= client.tags;
            }
        }

        stats
    }
}

/// Map a `BarPosition` to the `Gesture` used for hover highlighting.
pub fn bar_position_to_gesture(pos: BarPosition) -> Gesture {
    match pos {
        BarPosition::StartMenu => Gesture::StartMenu,
        BarPosition::Tag(idx) => Gesture::Tag(idx),
        BarPosition::CloseButton(_) => Gesture::CloseButton,
        BarPosition::WinTitle(w) => Gesture::WinTitle(w),
        BarPosition::SystrayItem(_) => Gesture::None,
        _ => Gesture::None,
    }
}

/// Compute which logical bar region the cursor's **monitor-local** x coordinate
/// falls in for the given monitor.
pub fn bar_position_at_x(mon: &Monitor, core: &CoreCtx, local_x: i32) -> BarPosition {
    use crate::bar::get_layout_symbol_width;

    let start_menu_size = core.g.cfg.startmenusize;

    if let Some(hit) = core.bar.monitor_hit_cache(mon.id()) {
        if local_x < start_menu_size {
            return BarPosition::StartMenu;
        }

        for r in &hit.tag_ranges {
            if local_x >= r.start && local_x < r.end {
                return BarPosition::Tag(r.tag_index);
            }
        }

        if local_x >= hit.layout_start && local_x < hit.layout_end {
            return BarPosition::LtSymbol;
        }

        if mon.sel.is_none() && local_x < hit.shutdown_end {
            return BarPosition::ShutDown;
        }

        if local_x > hit.status_hit_x {
            return BarPosition::StatusText;
        }

        for r in &hit.title_ranges {
            if local_x >= r.start && local_x < r.end {
                let this_width = (r.end - r.start).max(0);
                let resize_start = r.start + this_width - RESIZE_WIDGET_WIDTH;
                if mon.sel == Some(r.win) && local_x < r.start + CLOSE_BUTTON_HIT_WIDTH {
                    return BarPosition::CloseButton(r.win);
                }
                if mon.sel == Some(r.win) && local_x >= resize_start {
                    return BarPosition::ResizeWidget(r.win);
                }
                return BarPosition::WinTitle(r.win);
            }
        }

        return BarPosition::Root;
    }

    let (tag_end, tag_idx) = (get_tag_width(core), get_tag_at_x(core, local_x));
    let blw = get_layout_symbol_width(core, mon);

    // ── Start menu ────────────────────────────────────────────────────────
    if local_x < start_menu_size {
        return BarPosition::StartMenu;
    }

    // ── Tag buttons ───────────────────────────────────────────────────────
    if tag_idx >= 0 {
        return BarPosition::Tag(tag_idx as usize);
    }

    // ── Layout symbol ─────────────────────────────────────────────────────
    if local_x < tag_end + blw {
        return BarPosition::LtSymbol;
    }

    // ── Shutdown button (only when no client is selected) ─────────────────
    let bar_height = core.g.cfg.bar_height;
    if mon.sel.is_none() && local_x < tag_end + blw + bar_height {
        return BarPosition::ShutDown;
    }

    let is_selmon = core.g.selected_monitor().num == mon.num;
    let x11_present = !core.g.x11.xlibdisplay.0.is_null();
    if core.g.cfg.showsystray && is_selmon && !x11_present {
        if let Some(idx) = crate::wayland_systray::hit_test_wayland_systray_item(core, mon, local_x)
        {
            return BarPosition::SystrayItem(idx);
        }
    }

    // ── Status text ───────────────────────────────────────────────────────
    let systray_w = if core.g.cfg.showsystray && is_selmon {
        crate::systray::get_systray_width_for_bar(core, x11_present)
    } else {
        0
    };
    let status_hit_x =
        mon.work_rect.w - systray_w - core.g.status_text_width + core.g.cfg.horizontal_padding - 2;
    if local_x > status_hit_x {
        return BarPosition::StatusText;
    }

    // ── Window title cells ────────────────────────────────────────────────
    let mut visible_clients: Vec<WindowId> = Vec::new();
    let selected = mon.selected_tags();
    for (c_win, c) in mon.iter_clients(core.g.clients.map()) {
        if c.is_visible_on_tags(selected) {
            visible_clients.push(c_win);
        }
    }

    if !visible_clients.is_empty() {
        let title_area_start = tag_end + blw;
        let total_width = if mon.bar_clients_width > 0 {
            mon.bar_clients_width + 1
        } else {
            (mon.work_rect.w - title_area_start).max(0)
        };

        let n = visible_clients.len() as i32;
        let each_width = total_width / n;
        let mut remainder = total_width % n;

        let mut cell_start = title_area_start;
        for c_win in visible_clients {
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };
            let cell_end = cell_start + this_width;

            if local_x < cell_end {
                let resize_start = cell_start + this_width - RESIZE_WIDGET_WIDTH;
                if mon.sel == Some(c_win) && local_x < cell_start + CLOSE_BUTTON_HIT_WIDTH {
                    return BarPosition::CloseButton(c_win);
                }
                if mon.sel == Some(c_win) && local_x >= resize_start {
                    return BarPosition::ResizeWidget(c_win);
                }
                return BarPosition::WinTitle(c_win);
            }

            cell_start = cell_end;
        }
    }

    BarPosition::Root
}
