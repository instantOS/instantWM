pub mod color;
mod model;
pub mod paint;
mod renderer;
mod status;
mod theme;
pub mod wayland;
mod widgets;
pub mod x11;
mod x11_painter;

pub use model::{bar_position_at_x, bar_position_to_gesture};
pub use wayland::{draw_bars_wayland, reset_bar_wayland};
pub use x11::resize_bar_win;

use crate::backend::x11::X11BackendRef;
use crate::contexts::CoreCtx;
use crate::types::*;

#[derive(Default)]
pub struct BarState {
    pausedraw: bool,
    draw_bar_recursion: usize,
    bar_update_seq: u64,
    pub command_offsets: [i32; 20],
    /// Cached tag widths for hit-testing. Computed during render, used during hit-testing.
    pub tag_widths: Vec<i32>,
    /// Total width of the tag strip (including start menu)
    pub tag_strip_width: i32,
    /// Layout symbol width
    pub layout_symbol_width: i32,
    /// Per-monitor hit-test geometry built during bar rendering.
    pub hit_cache: Vec<MonitorHitCache>,
    /// Cached parsed status commands for unchanged status text.
    pub status_cache_text: String,
    pub status_cache_items: Vec<status::StatusItem>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TagHitRange {
    pub start: i32,
    pub end: i32,
    pub tag_index: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TitleHitRange {
    pub start: i32,
    pub end: i32,
    pub win: WindowId,
}

#[derive(Clone, Debug, Default)]
pub struct MonitorHitCache {
    pub tag_ranges: Vec<TagHitRange>,
    pub title_ranges: Vec<TitleHitRange>,
    pub layout_start: i32,
    pub layout_end: i32,
    pub shutdown_end: i32,
    pub status_hit_x: i32,
}

impl BarState {
    pub fn pausedraw(&self) -> bool {
        self.pausedraw
    }

    pub fn set_pausedraw(&mut self, paused: bool) {
        self.pausedraw = paused;
    }

    fn recursion_enter(&mut self) {
        self.draw_bar_recursion += 1;
        if self.draw_bar_recursion > 50 {
            std::process::abort();
        }
    }

    fn recursion_exit(&mut self) {
        self.draw_bar_recursion = self.draw_bar_recursion.saturating_sub(1);
    }

    pub fn clear_command_offsets(&mut self) {
        self.command_offsets.fill(-1);
    }

    /// Bump the backend-agnostic bar invalidation sequence.
    pub fn mark_dirty(&mut self) {
        self.bar_update_seq = self.bar_update_seq.wrapping_add(1);
    }

    /// Current bar invalidation sequence.
    pub fn update_seq(&self) -> u64 {
        self.bar_update_seq
    }

    /// Clear cached widths. Called at the start of each bar render.
    pub fn clear_cached_widths(&mut self) {
        self.tag_widths.clear();
        self.tag_strip_width = 0;
        self.layout_symbol_width = 0;
    }

    /// Cache a tag width at the given slot index.
    pub fn cache_tag_width(&mut self, slot: usize, width: i32) {
        if self.tag_widths.len() <= slot {
            self.tag_widths.resize(slot + 1, 0);
        }
        self.tag_widths[slot] = width;
    }

    /// Get cached width for a tag slot.
    pub fn get_tag_width(&self, slot: usize) -> i32 {
        self.tag_widths.get(slot).copied().unwrap_or(0)
    }

    pub fn begin_monitor_hit_cache(&mut self, monitor_id: usize) {
        if self.hit_cache.len() <= monitor_id {
            self.hit_cache
                .resize_with(monitor_id + 1, MonitorHitCache::default);
        }
        self.hit_cache[monitor_id] = MonitorHitCache::default();
    }

    pub fn monitor_hit_cache_mut(&mut self, monitor_id: usize) -> Option<&mut MonitorHitCache> {
        self.hit_cache.get_mut(monitor_id)
    }

    pub fn monitor_hit_cache(&self, monitor_id: usize) -> Option<&MonitorHitCache> {
        self.hit_cache.get(monitor_id)
    }
}

//TODO: remove, redundant
pub(crate) fn layout_symbol(m: &Monitor) -> String {
    m.layout_symbol()
}

pub fn get_layout_symbol_width(core: &CoreCtx, m: &Monitor) -> i32 {
    // Use cached width if available
    let width = if core.bar.layout_symbol_width > 0 {
        core.bar.layout_symbol_width
    } else {
        // Fallback: estimate based on typical character width
        let symbol = layout_symbol(m);
        symbol.len() as i32 * 8 // rough estimate: 8px per char
    };
    width + core.g.cfg.horizontal_padding
}

pub fn draw_bar(core: &mut CoreCtx, x11: &X11BackendRef, mon_idx: usize) {
    let bar_win = core
        .g
        .monitor(mon_idx)
        .map(|m| m.bar_win)
        .unwrap_or_default();
    if bar_win == WindowId::default() {
        return;
    }
    let work_rect_w = match core.g.monitor(mon_idx) {
        Some(m) => m.work_rect.w,
        None => return,
    };
    let bar_height = core.g.cfg.bar_height;
    if work_rect_w <= 0 || bar_height <= 0 {
        return;
    }

    let drw = {
        let Some(drw) = core.g.x11.drw.as_mut() else {
            return;
        };
        if !drw.has_display() {
            return;
        }
        drw.resize(work_rect_w as u32, bar_height as u32);
        drw.clone()
    };

    let mut painter = x11_painter::X11BarPainter::new(drw);

    renderer::draw_bar_common(core, Some(x11), mon_idx, &mut painter);

    painter.map(bar_win, 0, 0, work_rect_w as u16, bar_height as u16);
}

pub fn draw_bars_x11(core: &mut CoreCtx, x11: &X11BackendRef) {
    let indices: Vec<usize> = core.g.monitors_iter().map(|(i, _)| i).collect();
    for i in indices {
        draw_bar(core, x11, i);
    }
}

pub fn reset_bar_x11(core: &mut CoreCtx, x11: &X11BackendRef) {
    let selmon_idx = core.g.selected_monitor_id();
    renderer::reset_bar_common(core);
    draw_bar(core, x11, selmon_idx);
}
