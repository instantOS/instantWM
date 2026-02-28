use crate::backend::BackendKind;
use crate::bar::model::ClientBarStats;
use crate::bar::paint::BarPainter;
use crate::bar::{status, widgets};
use crate::contexts::WmCtx;
use crate::types::Gesture;

pub fn draw_bar_common(ctx: &mut WmCtx, mon_idx: usize, painter: &mut dyn BarPainter) {
    if ctx.backend_kind() != BackendKind::Wayland {
        if ctx.x11_conn().is_none() {
            return;
        }
    }

    let bar = ctx.bar as *mut crate::bar::BarState;
    unsafe { (*bar).recursion_enter() };

    let (monitor_num, work_rect_w) = match ctx.g.monitor(mon_idx) {
        Some(m) => {
            if !m.shows_bar() || ctx.bar.pausedraw() {
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

    let bh = ctx.g.cfg.bar_height;
    if work_rect_w <= 0 || bh <= 0 {
        unsafe { (*bar).recursion_exit() };
        return;
    }

    let is_selmon = ctx
        .g
        .selmon()
        .is_some_and(|selmon| selmon.num == monitor_num);

    let systray_width = if ctx.backend_kind() == BackendKind::Wayland {
        0
    } else if ctx.g.cfg.showsystray && is_selmon {
        crate::systray::get_systray_width(ctx) as i32
    } else {
        0
    };

    let (status_start_x, status_width) = if is_selmon {
        let m = ctx.g.monitor(mon_idx).cloned().unwrap();
        status::draw_status_bar(ctx, &m, bh, painter)
    } else {
        (0, 0)
    };

    if is_selmon {
        ctx.g.status_text_width = status_width;
    }

    widgets::draw_startmenu_icon(ctx, bh, painter);

    let (occupied_tags, urgent_tags, visible_clients) = {
        let m = ctx.g.monitor(mon_idx).unwrap();
        let stats = ClientBarStats::collect(m, ctx.g);
        (
            stats.occupied_tags,
            stats.urgent_tags,
            stats.visible_clients,
        )
    };

    let mut x = ctx.g.cfg.startmenusize;

    let mon_has_sel = ctx.g.monitor(mon_idx).is_some_and(|m| m.sel.is_some());

    {
        let ctx_imm = &*ctx;
        let m = ctx_imm.g.monitor(mon_idx).unwrap();
        x = widgets::draw_tag_indicators(ctx_imm, m, x, occupied_tags, urgent_tags, bh, painter);
        x = widgets::draw_layout_indicator(ctx_imm, m, x, bh, painter);
    }

    if !mon_has_sel {
        x = widgets::draw_shutdown_button(ctx, x, bh, painter);
    }

    let title_end_x = if is_selmon {
        status_start_x
    } else {
        work_rect_w - systray_width
    };
    let title_width = (title_end_x - x).max(0);

    let mut new_activeoffset = None;
    if title_width > 0 {
        let m = ctx.g.monitor(mon_idx).unwrap();
        let ctx_imm = &*ctx;
        new_activeoffset =
            widgets::draw_window_titles(ctx_imm, m, x, title_width, visible_clients, bh, painter);
    }

    if let Some(m) = ctx.g.monitor_mut(mon_idx) {
        m.bt = visible_clients;
        m.bar_clients_width = title_width;
        if let Some(offset) = new_activeoffset {
            m.activeoffset = offset;
        }
    }

    unsafe { (*bar).recursion_exit() };
}

pub fn reset_bar_common(ctx: &mut WmCtx) {
    let should_reset = ctx
        .g
        .selmon()
        .is_some_and(|selmon| selmon.gesture != Gesture::None);
    if !should_reset {
        return;
    }

    if let Some(selmon) = ctx.g.selmon_mut() {
        selmon.gesture = Gesture::None;
    }
}

pub fn should_draw_bar_common(ctx: &WmCtx) -> bool {
    if ctx.backend_kind() == BackendKind::Wayland {
        return ctx.g.cfg.showbar;
    }
    ctx.x11_conn().is_some()
}

pub fn compute_status_hit_width(painter: &dyn BarPainter, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    painter.text_width(text) + crate::bar::status::TEXT_PADDING * 2
}
