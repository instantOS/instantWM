use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::systray::get_systray_width;
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
    ///
    /// * `visible_clients` — counted by walking the intrusive linked list so
    ///   the number exactly matches what `draw_window_titles` will draw and
    ///   what `bar_position_at_x` uses for hit-testing.  The draw/hit-test
    ///   code skips clients that fail `is_visible_on_tags()`, so we apply the same
    ///   predicate here.
    ///
    /// * `occupied_tags` / `urgent_tags` — accumulated from all clients on the
    ///   monitor regardless of list order; order does not matter for bitmasks.
    pub(crate) fn collect(monitor: &Monitor, globals: &Globals) -> Self {
        let mut stats = Self::default();
        let selected = monitor.selected_tags();

        // ── Pass 1: visible_clients via the linked list ───────────────────
        // Walking the linked list (monitor.clients → client.next) gives the
        // same iteration order as draw_window_titles and bar_position_at_x,
        // so the count is guaranteed to be consistent with what is drawn and
        // what click regions are calculated for.
        for (_win, client) in monitor.iter_clients(&globals.clients) {
            if client.is_visible_on_tags(selected) {
                stats.visible_clients += 1;
            }
        }

        // ── Pass 2: occupied / urgent tag bits from all clients on this monitor
        // Use monitor.id() (the Vec index stored as MonitorId) for matching
        // because Client::mon_id holds the Vec index, not monitor.num (which is
        // the Xinerama display number and can diverge from the Vec index).
        let monitor_id = monitor.id();
        for client in globals.clients.values() {
            if client.mon_id != Some(monitor_id) {
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
///
/// Not every `BarPosition` maps to a meaningful gesture; those that don't
/// map to `Gesture::None`.
pub fn bar_position_to_gesture(pos: BarPosition) -> Gesture {
    match pos {
        BarPosition::StartMenu => Gesture::StartMenu,
        BarPosition::Tag(idx) => Gesture::Tag(idx),
        BarPosition::CloseButton(_) => Gesture::CloseButton,
        BarPosition::WinTitle(w) => Gesture::WinTitle(w),
        // Most positions don't map to a gesture.
        _ => Gesture::None,
    }
}

/// Compute which logical bar region the cursor's **monitor-local** x coordinate
/// falls in for the given monitor.
///
/// `local_x` must already be relative to the left edge of the monitor
/// (i.e. `root_x − monitor.monitor_rect.x` for root-window coordinates, or
/// `event_x` for bar-window coordinates which are already monitor-local).
///
/// This function is the **single canonical hit-test** for the bar. Both click
/// handling and hover-gesture detection should call it rather than reimplementing
/// the geometry themselves.
///
/// Outside-bar cases (`ClientWin`, `SideBar`, `Root`) are set directly by
/// `button_press`; this function only returns bar-interior variants.
pub fn bar_position_at_x(mon: &Monitor, ctx: &WmCtx, local_x: i32) -> BarPosition {
    use crate::bar::get_layout_symbol_width;

    let start_menu_size = ctx.g.cfg.startmenusize;
    let (tag_end, tag_idx) = (get_tag_width(ctx), get_tag_at_x(ctx, local_x));
    let blw = get_layout_symbol_width(ctx, mon);

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
    let bh = ctx.g.cfg.bar_height;
    if mon.sel.is_none() && local_x < tag_end + blw + bh {
        return BarPosition::ShutDown;
    }

    // ── Status text ───────────────────────────────────────────────────────
    let systray_w = if ctx.backend_kind() == crate::backend::BackendKind::Wayland {
        0
    } else {
        get_systray_width(ctx) as i32
    };
    let status_hit_x =
        mon.work_rect.w - systray_w - ctx.g.status_text_width + ctx.g.cfg.horizontal_padding - 2;
    if local_x > status_hit_x {
        return BarPosition::StatusText;
    }

    // ── Window title cells ────────────────────────────────────────────────
    // Build the ordered list of visible clients exactly as draw_window_titles
    // does (intrusive linked list walk). draw_window_titles and all click/hover
    // consumers delegate to this function, so the order is always consistent.
    let mut visible_clients: Vec<WindowId> = Vec::new();
    let selected = mon.selected_tags();
    for (c_win, c) in mon.iter_clients(&ctx.g.clients) {
        if c.is_visible_on_tags(selected) {
            visible_clients.push(c_win);
        }
    }

    if !visible_clients.is_empty() {
        // Total width of the title area.
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

            if local_x <= cell_end {
                // Cursor is inside this cell.
                let resize_start = cell_start + this_width - RESIZE_WIDGET_WIDTH;
                if mon.sel == Some(c_win) && local_x < cell_start + CLOSE_BUTTON_HIT_WIDTH {
                    return BarPosition::CloseButton(c_win);
                }
                if mon.sel == Some(c_win) && local_x > resize_start {
                    return BarPosition::ResizeWidget(c_win);
                }
                return BarPosition::WinTitle(c_win);
            }

            cell_start = cell_end;
        }
    }

    BarPosition::Root
}
