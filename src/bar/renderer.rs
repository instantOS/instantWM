use crate::bar::model::ClientBarStats;
use crate::bar::paint::BarPainter;
use crate::bar::{status, widgets};
use crate::contexts::CoreCtx;
use crate::types::BarHoverState;

/// Core bar drawing implementation shared between X11 and Wayland.
///
/// Systray width must be cached in `core.g.systray_width` by the caller
/// before invoking this function.
pub(crate) fn draw_bar(core: &mut CoreCtx, mon_idx: usize, painter: &mut dyn BarPainter) {
    let bar = core.bar as *mut crate::bar::BarState;
    unsafe { (*bar).recursion_enter() };

    let (monitor_num, work_rect_w, monitor_id) = match core.g.monitor(mon_idx) {
        Some(m) => {
            if !m.shows_bar() || core.bar.pausedraw() {
                unsafe { (*bar).recursion_exit() };
                return;
            }
            (m.num, m.work_rect.w, m.id())
        }
        None => {
            unsafe { (*bar).recursion_exit() };
            return;
        }
    };

    let bar_height = core.g.cfg.bar_height;
    if work_rect_w <= 0 || bar_height <= 0 {
        unsafe { (*bar).recursion_exit() };
        return;
    }

    let is_selmon = core.g.selected_monitor().num == monitor_num;

    let systray_width = if core.g.cfg.show_systray && is_selmon {
        core.g.systray_width
    } else {
        0
    };

    let (status_start_x, status_width, status_click_targets) = if is_selmon {
        let m = core.g.monitor(mon_idx).cloned().unwrap();
        status::draw_status_bar(core, systray_width, &m, bar_height, painter)
    } else {
        (0, 0, Vec::new())
    };

    core.bar.clear_cached_widths();
    core.bar.begin_monitor_hit_cache(monitor_id);
    if let Some(hit) = core.bar.monitor_hit_cache_mut(monitor_id) {
        hit.status_click_targets = status_click_targets;
    }

    widgets::draw_startmenu_icon(core, bar_height, painter);

    let (occupied_tags, urgent_tags, visible_clients) = {
        let m = core.g.monitor(mon_idx).unwrap();
        let stats = ClientBarStats::collect(m, core.g);
        (
            stats.occupied_tags,
            stats.urgent_tags,
            stats.visible_clients,
        )
    };

    let mut x = core.g.cfg.startmenusize;

    let mon_has_sel = core.g.monitor(mon_idx).is_some_and(|m| m.sel.is_some());

    {
        let m = core.g.monitor(mon_idx).cloned().unwrap();
        x = widgets::draw_tag_indicators(
            core,
            &m,
            x,
            occupied_tags,
            urgent_tags,
            bar_height,
            painter,
        );
        x = widgets::draw_layout_indicator(core, &m, x, bar_height, painter);
    }

    if !mon_has_sel {
        x = widgets::draw_shutdown_button(core, x, bar_height, painter);
    }

    if let Some(hit) = core.bar.monitor_hit_cache_mut(monitor_id) {
        hit.shutdown_end = x;
    }

    let title_end_x = if is_selmon && status_width > 0 {
        status_start_x
    } else {
        work_rect_w - systray_width
    };
    let title_width = (title_end_x - x).max(0);

    if let Some(hit) = core.bar.monitor_hit_cache_mut(monitor_id) {
        hit.status_hit_x = if is_selmon && status_width > 0 {
            status_start_x
        } else {
            work_rect_w - systray_width
        };
    }

    let mut new_activeoffset = None;
    if title_width > 0 {
        let m = core.g.monitor(mon_idx).cloned().unwrap();
        new_activeoffset = widgets::draw_window_titles(
            core,
            &m,
            x,
            title_width,
            visible_clients,
            bar_height,
            painter,
        );
    }

    if let Some(m) = core.g.monitor_mut(mon_idx) {
        m.bar_clients_width = title_width;
        if let Some(offset) = new_activeoffset {
            m.activeoffset = offset;
        }
    }

    unsafe { (*bar).recursion_exit() };
}

pub fn reset_bar_common(core: &mut CoreCtx) {
    let selmon = core.g.selected_monitor();
    if selmon.bar_hover_state == BarHoverState::None {
        return;
    }

    core.g.selected_monitor_mut().bar_hover_state = BarHoverState::None;
}
