pub mod color;
pub(crate) mod model;
pub mod paint;
mod renderer;
pub(crate) mod scene;
pub mod status;
pub mod wayland;
mod widgets;
pub mod x11;
mod x11_painter;

pub use model::bar_position_to_gesture;
pub use renderer::reset_bar_common;
pub use x11::resize_bar_win;

use crate::contexts::{CoreCtx, WmCtx};
use crate::globals::Globals;
use crate::types::*;

#[derive(Default)]
pub struct BarState {
    //TODO: should this be used or removed?
    pause_draw: bool,
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
    status_cache_text: String,
    status_cache: status::ParsedStatus,
    status_cache_parsed: bool,
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
        self.pause_draw
    }

    pub fn set_pausedraw(&mut self, paused: bool) {
        self.pause_draw = paused;
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

    pub fn begin_monitor_hit_cache(&mut self, monitor_id: crate::types::MonitorId) {
        let monitor_id = monitor_id.index();
        if self.hit_cache.len() <= monitor_id {
            self.hit_cache
                .resize_with(monitor_id + 1, MonitorHitCache::default);
        }
        self.hit_cache[monitor_id] = MonitorHitCache::default();
    }

    pub fn monitor_hit_cache_mut(
        &mut self,
        monitor_id: crate::types::MonitorId,
    ) -> Option<&mut MonitorHitCache> {
        self.hit_cache.get_mut(monitor_id.index())
    }

    pub fn monitor_hit_cache(
        &self,
        monitor_id: crate::types::MonitorId,
    ) -> Option<&MonitorHitCache> {
        self.hit_cache.get(monitor_id.index())
    }

    pub fn replace_hit_cache(&mut self, monitor_id: crate::types::MonitorId, hit: MonitorHitCache) {
        let monitor_id = monitor_id.index();
        if self.hit_cache.len() <= monitor_id {
            self.hit_cache
                .resize_with(monitor_id + 1, MonitorHitCache::default);
        }
        self.hit_cache[monitor_id] = hit;
    }

    pub fn prepare_status_for_render(&mut self, text: &str) {
        self.status_cache_text.clear();
        self.status_cache_text.push_str(text);
        self.status_cache = status::parse_status_fallback(text);
        self.status_cache_parsed = false;
    }

    fn ensure_status_cached(&mut self, text: &str) {
        if self.status_cache_text.as_str() != text || !self.status_cache_parsed {
            self.status_cache_text.clear();
            self.status_cache_text.push_str(text);
            self.status_cache = status::parse_status(text.as_bytes());
            self.status_cache_parsed = true;
        }
    }

    pub(crate) fn status_items_for_text(&mut self, text: &str) -> &[status::StatusItem] {
        self.ensure_status_cached(text);
        self.status_cache.items.as_slice()
    }

    pub(crate) fn parsed_status_for_text(&mut self, text: &str) -> &status::ParsedStatus {
        self.ensure_status_cached(text);
        &self.status_cache
    }
}

pub fn get_layout_symbol_width(core: &CoreCtx, m: &Monitor) -> i32 {
    // Use cached width if available
    let width = if core.bar.layout_symbol_width > 0 {
        core.bar.layout_symbol_width
    } else {
        // Fallback: estimate based on typical character width
        let symbol = if crate::overview::is_active_on_monitor(core, m) {
            "OVR"
        } else {
            m.layouts_for_mask(m.selected_tags()).symbol()
        };
        symbol.len() as i32 * 8 // rough estimate: 8px per char
    };
    width + core.globals().cfg.horizontal_padding
}

pub fn clear_hover(ctx: &mut WmCtx) {
    if ctx.core().globals().selected_monitor().gesture != Gesture::None {
        reset_bar_common(ctx.core_mut());
        ctx.request_bar_update();
    }
}

pub fn resolve_bar_position_at_root(
    core: &mut CoreCtx,
    root: Point,
    sync_selected_monitor: bool,
) -> Option<(MonitorId, BarPosition)> {
    let rect = crate::mouse::pointer::point_rect(root);
    let monitor_id = crate::types::find_monitor_by_rect(core.globals().monitors.monitors(), &rect)?;
    if sync_selected_monitor && monitor_id != core.globals().selected_monitor_id() {
        core.globals_mut().set_selected_monitor(monitor_id);
    }

    let mon = core.globals().monitor(monitor_id)?;
    let bar_h = core.globals().cfg.bar_height.max(1);
    let in_bar = monitor_bar_visible(core.globals(), mon)
        && root.y >= mon.bar_y
        && root.y < mon.bar_y + bar_h;
    if !in_bar {
        return None;
    }

    let local_x = root.x - mon.work_rect.x;
    Some((monitor_id, mon.bar_position_at_x(core, local_x)))
}

pub(crate) fn monitor_has_real_fullscreen(globals: &Globals, monitor: &Monitor) -> bool {
    let selected_tags = monitor.selected_tags();
    monitor
        .iter_clients(globals.clients.map())
        .any(|(_, client)| client.mode.is_true_fullscreen() && client.is_visible(selected_tags))
}

pub(crate) fn monitor_bar_visible(globals: &Globals, monitor: &Monitor) -> bool {
    monitor.shows_bar() && !monitor_has_real_fullscreen(globals, monitor)
}

#[cfg(test)]
mod tests {
    use super::BarState;
    use crate::bar::status::StatusItem;

    #[test]
    fn prepared_status_is_parsed_on_first_cache_read() {
        let text = r#"[{"full_text":"cpu","name":"cpu"}]"#;
        let mut bar = BarState::default();

        bar.prepare_status_for_render(text);

        let parsed = bar.parsed_status_for_text(text);
        assert!(parsed.i3bar.is_some());
        assert!(matches!(parsed.items.first(), Some(StatusItem::I3Block(_))));
    }
}

pub fn update_hover(
    ctx: &mut WmCtx,
    root: Point,
    reset_start_menu: bool,
    sync_selected_monitor: bool,
) -> Option<BarPosition> {
    let Some((_monitor_id, pos)) =
        resolve_bar_position_at_root(ctx.core_mut(), root, sync_selected_monitor)
    else {
        clear_hover(ctx);
        return None;
    };

    if reset_start_menu && pos == BarPosition::StartMenu {
        reset_bar_common(ctx.core_mut());
        ctx.request_bar_update();
    }

    let old_gesture = ctx.core().globals().selected_monitor().gesture;
    let gesture = if pos == BarPosition::StatusText {
        old_gesture
    } else {
        bar_position_to_gesture(pos)
    };
    if old_gesture != gesture {
        ctx.core_mut().globals_mut().selected_monitor_mut().gesture = gesture;
        ctx.request_bar_update();
    }

    Some(pos)
}

pub fn handle_status_text_click(ctx: &mut WmCtx, root: Point, button_code: u8, clean_state: u32) {
    if crate::overview::is_active(ctx.core()) {
        ctx.with_behavior_mut(|behavior| behavior.overview_accept_selection_on_exit = false);
        ctx.reset_mode();
        ctx.request_bar_update();
        return;
    }

    let mode = ctx.current_mode();
    if !mode.is_empty() && mode != "default" {
        ctx.reset_mode();
        ctx.request_bar_update();
        return;
    }

    let selected_monitor = ctx.core().globals().selected_monitor().clone();
    let local_x = root.x - selected_monitor.work_rect.x;
    let status_text = ctx.core().globals().bar_runtime.status_text.clone();
    let parsed = ctx
        .core_mut()
        .bar
        .parsed_status_for_text(&status_text)
        .clone();
    let click_targets = ctx
        .core()
        .bar
        .monitor_hit_cache(selected_monitor.id())
        .map(|h| h.status_click_targets.as_slice())
        .unwrap_or(&[]);
    status::emit_i3bar_status_click(
        &parsed,
        click_targets,
        local_x,
        root.y - selected_monitor.bar_y,
        button_code,
        ctx.core().globals().cfg.bar_height,
        clean_state,
    );
}
