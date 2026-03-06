use crate::backend::x11::X11BackendRef;
use crate::bar::model::ClientBarStats;
use crate::bar::paint::BarPainter;
use crate::bar::{status, widgets};
use crate::contexts::CoreCtx;
use crate::types::Gesture;

pub fn draw_bar_common(
    core: &mut CoreCtx,
    x11: Option<&X11BackendRef>,
    mon_idx: usize,
    painter: &mut dyn BarPainter,
) {
    let bar = core.bar as *mut crate::bar::BarState;
    unsafe { (*bar).recursion_enter() };

    let (monitor_num, work_rect_w) = match core.g.monitor(mon_idx) {
        Some(m) => {
            if !m.shows_bar() || core.bar.pausedraw() {
                unsafe { (*bar).recursion_exit() };
                return;
            }
            (m.num, m.work_rect.w)
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

    let systray_width = if core.g.cfg.showsystray && is_selmon && x11.is_some() {
        crate::systray::get_systray_width(core) as i32
    } else {
        0
    };

    let (status_start_x, status_width) = if is_selmon {
        let m = core.g.monitor(mon_idx).cloned().unwrap();
        status::draw_status_bar(core, &m, bar_height, painter)
    } else {
        (0, 0)
    };

    if is_selmon {
        core.g.status_text_width = status_width;
    }
    core.bar.clear_cached_widths();

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

    let title_end_x = if is_selmon && status_width > 0 {
        status_start_x
    } else {
        work_rect_w - systray_width
    };
    let title_width = (title_end_x - x).max(0);

    let mut new_activeoffset = None;
    if title_width > 0 {
        let m = core.g.monitor(mon_idx).unwrap();
        let ctx_imm = &*core;
        new_activeoffset = widgets::draw_window_titles(
            ctx_imm,
            m,
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
    if selmon.gesture == Gesture::None {
        return;
    }

    core.g.selected_monitor_mut().gesture = Gesture::None;
}

pub fn compute_status_hit_width(painter: &mut dyn BarPainter, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    painter.text_width(text) + crate::bar::status::TEXT_PADDING * 2
}
