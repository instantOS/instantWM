use crate::bar::{BarOverlayHit, MonitorHitCache, TagHitRange, TitleHitRange};
use crate::contexts::CoreCtx;
use crate::model::WmModel;
use crate::types::*;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientBarStats {
    pub occupied_tags: TagMask,
    pub urgent_tags: TagMask,
}

impl ClientBarStats {
    /// Collect bar statistics for the given monitor.
    pub(crate) fn collect(monitor: &Monitor, model: &WmModel) -> Self {
        let mut stats = Self::default();

        // Occupied / urgent tag bits from all clients on this monitor.
        let mut occupied = TagMask::EMPTY;
        for (_win, client) in monitor.iter_clients(&model.clients) {
            occupied = occupied | client.tags;
            if client.is_urgent {
                stats.urgent_tags = stats.urgent_tags | client.tags;
            }
        }
        stats.occupied_tags = occupied.without_scratchpad();

        stats
    }
}

/// Split `total` into `n` cell widths that differ by at most one pixel.
///
/// The remainder of `total / n` is distributed one pixel at a time over the
/// leading cells, keeping every boundary on whole pixels. Returns an empty
/// `Vec` when `n` is not positive.
///
/// `total` must be non-negative; with a negative `total` the truncated
/// division leaves the cells no longer summing back to it.
pub(crate) fn distribute_cells(total: i32, n: i32) -> Vec<i32> {
    if n <= 0 {
        return Vec::new();
    }
    let each = total / n;
    let mut remainder = total % n;
    (0..n)
        .map(|_| {
            if remainder > 0 {
                remainder -= 1;
                each + 1
            } else {
                each
            }
        })
        .collect()
}

/// Walk a `MonitorHitCache` to resolve a local-x coordinate into a `BarPosition`.
/// This is the single source of truth for hit-testing; both the cached and the
/// fallback paths go through here.
pub(crate) fn hit_test(
    hit: &MonitorHitCache,
    monitor: &Monitor,
    systray_show: bool,
    is_selected_monitor: bool,
    local_x: i32,
) -> BarPosition {
    if is_selected_monitor
        && let Some(BarOverlayHit::TrayMenu { start, end, slots }) = &hit.overlay
        && local_x >= *start
        && local_x < *end
    {
        for slot in slots {
            if local_x >= slot.start && local_x < slot.end {
                return BarPosition::SystrayMenuItem(slot.idx);
            }
        }
        return BarPosition::Root;
    }

    if local_x < monitor.startmenu_size {
        return BarPosition::StartMenu;
    }

    if systray_show && is_selected_monitor {
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
        return BarPosition::LayoutSymbol;
    }

    if monitor.selected.is_none() && local_x < hit.shutdown_end {
        return BarPosition::ShutDown;
    }

    if is_selected_monitor && local_x > hit.status_hit_x {
        return BarPosition::StatusText;
    }

    for r in &hit.title_ranges {
        if local_x >= r.start && local_x < r.end {
            let this_width = (r.end - r.start).max(0);
            let resize_start = r.start + this_width - RESIZE_WIDGET_WIDTH;
            if monitor.selected == Some(r.win) && local_x < r.start + CLOSE_BUTTON_HIT_WIDTH {
                return BarPosition::CloseButton(r.win);
            }
            if monitor.selected == Some(r.win) && local_x >= resize_start {
                return BarPosition::ResizeWidget(r.win);
            }
            return BarPosition::WinTitle(r.win);
        }
    }

    BarPosition::Root
}

/// Resolve the logical bar region for `local_x` on `monitor`.
///
/// Prefers the pre-built hit cache populated during rendering; falls back to
/// computing a temporary one from the same utility functions.
pub(crate) fn bar_position_at_x(monitor: &Monitor, core: &CoreCtx, local_x: i32) -> BarPosition {
    let is_selmon = core.model().expect_selected_monitor().num == monitor.num;
    let owned;
    let hit: &MonitorHitCache = match core.bar.monitor_hit_cache(monitor.id()) {
        Some(h) => h,
        None => {
            owned = build_fallback_hit_cache(monitor, core);
            &owned
        }
    };

    hit_test(hit, monitor, core.config().systray.show, is_selmon, local_x)
}

/// Return the title-cell index at `local_x`, independent of the window
/// identity captured in the render snapshot.
pub(crate) fn title_hit_slot(hit: &MonitorHitCache, local_x: i32) -> Option<usize> {
    hit.title_ranges
        .iter()
        .position(|range| local_x >= range.start && local_x < range.end)
}

/// Build a `MonitorHitCache` from scratch using the same utility functions that
/// the renderer uses, for when the render-time cache is not yet available.
pub(crate) fn build_fallback_hit_cache(mon: &Monitor, core: &CoreCtx) -> MonitorHitCache {
    let is_selmon = core.model().expect_selected_monitor().num == mon.num;
    let layout_symbol = if core.model().is_overview_active_on(mon) {
        "OVR"
    } else {
        mon.layout_symbol_for_mask(mon.selected_tags())
    };
    let bar_layout_symbol_width =
        layout_symbol.len() as i32 * 8 + core.derived().bar_horizontal_padding;
    let bar_height = mon.bar_height;

    // ── Tag ranges ────────────────────────────────────────────────────────
    let occupied = mon.occupied_tags(&core.model().clients);
    let visible = crate::tags::bar::visible_tags(core.state(), mon, occupied);
    let mut tag_ranges: Vec<TagHitRange> = Vec::new();
    let mut acc = mon.startmenu_size;
    for tag in &visible {
        tag_ranges.push(TagHitRange {
            start: acc,
            end: acc + tag.width,
            tag_index: tag.tag_index,
        });
        acc += tag.width;
    }
    let tag_end = acc;

    // ── Layout symbol ─────────────────────────────────────────────────────
    let layout_start = tag_end;
    let layout_end = tag_end + bar_layout_symbol_width;

    // ── Shutdown button ───────────────────────────────────────────────────
    let shutdown_end = layout_end + bar_height;

    // ── Status text ───────────────────────────────────────────────────────
    let systray_w = if core.config().systray.show && is_selmon {
        core.bar.runtime.systray_width
    } else {
        0
    };
    let status_hit_x = mon.work_rect().w - systray_w;

    // ── Window title ranges ───────────────────────────────────────────────
    let title_clients = mon.bar_client_order(&core.model().clients);
    let n = title_clients.len() as i32;

    let mut title_ranges: Vec<TitleHitRange> = Vec::new();
    if n > 0 {
        let title_area_start = layout_end;
        let total_width = if mon.bar_clients_width > 0 {
            mon.bar_clients_width + 1
        } else {
            (mon.work_rect().w - title_area_start).max(0)
        };
        let mut cell_start = title_area_start;
        for (win, this_width) in title_clients
            .into_iter()
            .zip(distribute_cells(total_width, n))
        {
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
        systray_slots: Vec::new(),
        overlay: None,
        status_click_targets: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::SystrayHitSlot;

    #[test]
    fn tray_menu_overlay_handles_items_and_blocks_fallthrough() {
        let monitor = Monitor::default();
        let hit = MonitorHitCache {
            status_hit_x: 0,
            systray_slots: vec![SystrayHitSlot {
                idx: 9,
                start: 40,
                end: 80,
            }],
            overlay: Some(BarOverlayHit::TrayMenu {
                start: 40,
                end: 80,
                slots: vec![SystrayHitSlot {
                    idx: 2,
                    start: 40,
                    end: 50,
                }],
            }),
            ..MonitorHitCache::default()
        };

        assert_eq!(
            hit_test(&hit, &monitor, true, true, 45),
            BarPosition::SystrayMenuItem(2)
        );
        assert_eq!(
            hit_test(&hit, &monitor, true, true, 55),
            BarPosition::Root,
            "gaps in the overlay must not activate the tray icon or status below it"
        );
        assert_eq!(
            hit_test(&hit, &monitor, false, true, 45),
            BarPosition::SystrayMenuItem(2),
            "an already-open overlay remains authoritative while configuration changes"
        );
    }

    #[test]
    fn distribute_cells_sums_back_to_total_with_one_pixel_spread() {
        // The leading cells absorb the remainder, so every cell is within
        // one pixel of the others and they sum back to `total`.
        for (total, n) in [(10, 3), (100, 7), (7, 7), (1, 4), (256, 1)] {
            let cells = distribute_cells(total, n);
            assert_eq!(cells.len(), n as usize, "n = {n}");
            assert_eq!(cells.iter().sum::<i32>(), total, "total = {total}, n = {n}");
            assert!(
                cells.iter().max().unwrap() - cells.iter().min().unwrap() <= 1,
                "cells {cells:?} differ by more than one pixel"
            );
        }
    }

    #[test]
    fn distribute_cells_distributes_remainder_over_leading_cells() {
        // 2 pixels across 5 cells: first two get the single-pixel surplus.
        assert_eq!(distribute_cells(2, 5), vec![1, 1, 0, 0, 0]);
    }

    #[test]
    fn distribute_cells_handles_zero_total_and_non_positive_n() {
        assert_eq!(distribute_cells(0, 4), vec![0, 0, 0, 0]);
        assert!(distribute_cells(10, 0).is_empty());
        assert!(distribute_cells(10, -1).is_empty());
    }
}
