pub mod color;
pub(crate) mod model;
pub mod paint;
mod renderer;
pub mod status;
pub(crate) mod theme;
pub mod wayland;
mod widgets;
pub mod x11;
mod x11_painter;

pub use model::bar_position_to_gesture;
pub use renderer::reset_bar_common;
pub use x11::resize_bar_win;

use crate::contexts::CoreCtx;
use crate::types::*;

#[derive(Default)]
pub struct BarState {
    pausedraw: bool,
    draw_bar_recursion: usize,
    bar_update_seq: u64,
    last_drawn_seq: u64,
    /// Cached tag widths for hit-testing. Computed during render, used during hit-testing.
    pub tag_widths: Vec<i32>,
    /// Total width of the tag strip (including start menu)
    pub tag_strip_width: i32,
    /// Layout symbol width
    pub layout_symbol_width: i32,
    /// Per-monitor hit-test geometry built during bar rendering.
    hit_cache: Vec<MonitorHitCache>,
    /// Cached parsed status commands for unchanged status text.
    status_cache_text: String,
    status_cache: status::ParsedStatus,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct SystrayHitSlot {
    pub idx: usize,
    pub start: i32,
    pub end: i32,
}

#[derive(Clone, Debug, Default)]
pub struct MonitorHitCache {
    pub tag_ranges: Vec<TagHitRange>,
    pub title_ranges: Vec<TitleHitRange>,
    pub layout_start: i32,
    pub layout_end: i32,
    pub shutdown_end: i32,
    pub status_hit_x: i32,
    /// Systray item hit slots for Wayland bars. Populated during rendering.
    pub systray_slots: Vec<SystrayHitSlot>,
    /// Systray menu item hit slots for Wayland bars. Populated during rendering.
    pub systray_menu_slots: Vec<SystrayHitSlot>,
    pub(crate) status_click_targets: Vec<status::StatusClickTarget>,
}

impl BarState {
    pub fn pausedraw(&self) -> bool {
        self.pausedraw
    }

    pub fn set_pausedraw(&mut self, paused: bool) {
        self.pausedraw = paused;
    }

    pub(crate) fn try_recursion_enter(&mut self) -> bool {
        if self.draw_bar_recursion > 0 {
            self.mark_dirty();
            return false;
        }
        self.draw_bar_recursion = 1;
        true
    }

    pub(crate) fn recursion_exit(&mut self) {
        self.draw_bar_recursion = self.draw_bar_recursion.saturating_sub(1);
    }

    pub fn is_drawing(&self) -> bool {
        self.draw_bar_recursion > 0
    }

    /// Bump the backend-agnostic bar invalidation sequence.
    pub fn mark_dirty(&mut self) {
        self.bar_update_seq = self.bar_update_seq.wrapping_add(1);
    }

    /// Current bar invalidation sequence.
    pub fn update_seq(&self) -> u64 {
        self.bar_update_seq
    }

    pub fn needs_redraw(&self) -> bool {
        self.bar_update_seq != self.last_drawn_seq
    }

    pub fn mark_drawn(&mut self) {
        self.last_drawn_seq = self.bar_update_seq;
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

    pub(crate) fn status_items_for_text(&mut self, text: &str) -> &[status::StatusItem] {
        if self.status_cache_text.as_str() != text {
            self.status_cache_text.clear();
            self.status_cache_text.push_str(text);
            self.status_cache = status::parse_status(text.as_bytes());
        }
        self.status_cache.items.as_slice()
    }

    pub(crate) fn parsed_status_for_text(&mut self, text: &str) -> &status::ParsedStatus {
        if self.status_cache_text.as_str() != text {
            self.status_cache_text.clear();
            self.status_cache_text.push_str(text);
            self.status_cache = status::parse_status(text.as_bytes());
        }
        &self.status_cache
    }
}

pub fn get_layout_symbol_width(core: &CoreCtx, m: &Monitor) -> i32 {
    // Use cached width if available
    let width = if core.bar.layout_symbol_width > 0 {
        core.bar.layout_symbol_width
    } else {
        // Fallback: estimate based on typical character width
        let symbol = m.layout_symbol();
        symbol.len() as i32 * 8 // rough estimate: 8px per char
    };
    width + core.globals().cfg.horizontal_padding
}
