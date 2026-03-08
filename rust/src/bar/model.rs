use crate::bar::{MonitorHitCache, TagHitRange, TitleHitRange};
use crate::contexts::CoreCtx;
use crate::globals::Globals;
use crate::tags::get_tag_width;
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
        BarPosition::SystrayMenuItem(_) => Gesture::None,
        _ => Gesture::None,
    }
}

/// Walk a `MonitorHitCache` to resolve a local-x coordinate into a `BarPosition`.
/// This is the single source of truth for hit-testing; both the cached and the
/// fallback paths go through here.
pub(crate) fn hit_test(
    hit: &MonitorHitCache,
    mon: &Monitor,
    core: &CoreCtx,
    is_selmon: bool,
    local_x: i32,
) -> BarPosition {
    if local_x < core.g.cfg.startmenusize {
        return BarPosition::StartMenu;
    }

    if core.g.cfg.show_systray && is_selmon && !hit.x11_bar {
        // Check systray menu items first (they appear to the left of tray items)
        for slot in &hit.systray_menu_slots {
            if local_x >= slot.start && local_x < slot.end {
                return BarPosition::SystrayMenuItem(slot.idx);
            }
        }
        // Check systray tray items
        for slot in &hit.systray_slots {
            if local_x >= slot.start && local_x < slot.end {
                return BarPosition::SystrayItem(slot.idx);
            }
        }
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

    if core.g.status_text_width > 0 && local_x > hit.status_hit_x {
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

    BarPosition::Root
}

/// Build a `MonitorHitCache` from scratch using the same utility functions that
/// the renderer uses, for when the render-time cache is not yet available.
pub(crate) fn build_fallback_hit_cache(mon: &Monitor, core: &CoreCtx) -> MonitorHitCache {
    use crate::bar::get_layout_symbol_width;

    let is_selmon = core.g.selected_monitor().num == mon.num;
    let tag_end = get_tag_width(core);
    let blw = get_layout_symbol_width(core, mon);
    let bar_height = core.g.cfg.bar_height;

    // ── Tag ranges ────────────────────────────────────────────────────────
    let mut tag_ranges: Vec<TagHitRange> = Vec::new();
    let mut acc = core.g.cfg.startmenusize;
    for (slot, &w) in core.bar.tag_widths.iter().enumerate() {
        tag_ranges.push(TagHitRange {
            start: acc,
            end: acc + w,
            tag_index: slot,
        });
        acc += w;
    }

    // ── Layout symbol ─────────────────────────────────────────────────────
    let layout_start = tag_end;
    let layout_end = tag_end + blw;

    // ── Shutdown button ───────────────────────────────────────────────────
    let shutdown_end = layout_end + bar_height;

    // ── Status text ───────────────────────────────────────────────────────
    let systray_w = if core.g.cfg.show_systray && is_selmon {
        crate::systray::get_systray_width_for_bar(core, true, None)
            .max(crate::systray::get_systray_width_for_bar(core, false, None))
    } else {
        0
    };
    let status_hit_x =
        mon.work_rect.w - systray_w - core.g.status_text_width + core.g.cfg.horizontal_padding - 2;

    // ── Window title ranges ───────────────────────────────────────────────
    let selected = mon.selected_tags();
    let visible_clients: Vec<WindowId> = mon
        .iter_clients(core.g.clients.map())
        .filter_map(|(win, c)| c.is_visible_on_tags(selected).then_some(win))
        .collect();

    let mut title_ranges: Vec<TitleHitRange> = Vec::new();
    if !visible_clients.is_empty() {
        let title_area_start = layout_end;
        let total_width = if mon.bar_clients_width > 0 {
            mon.bar_clients_width + 1
        } else {
            (mon.work_rect.w - title_area_start).max(0)
        };
        let n = visible_clients.len() as i32;
        let each_width = total_width / n;
        let mut remainder = total_width % n;
        let mut cell_start = title_area_start;
        for win in visible_clients {
            let this_width = if remainder > 0 {
                remainder -= 1;
                each_width + 1
            } else {
                each_width
            };
            title_ranges.push(TitleHitRange {
                start: cell_start,
                end: cell_start + this_width,
                win,
            });
            cell_start += this_width;
        }
    }

    MonitorHitCache {
        tag_ranges,
        title_ranges,
        layout_start,
        layout_end,
        shutdown_end,
        status_hit_x,
        x11_bar: false,
        systray_slots: Vec::new(),
        systray_menu_slots: Vec::new(),
    }
}
