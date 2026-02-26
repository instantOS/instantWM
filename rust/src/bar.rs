mod model;
mod status;
mod widgets;
pub mod x11;

pub use model::{bar_position_at_x, bar_position_to_gesture};
pub use x11::{resize_bar_win, resize_bar_win_ctx};

use crate::backend::BackendKind;
use crate::contexts::WmCtx;
use crate::types::*;
use model::ClientBarStats;

#[derive(Default)]
pub struct BarState {
    pausedraw: bool,
    draw_bar_recursion: usize,
    pub command_offsets: [i32; 20],
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
}

pub fn text_width_ctx(ctx: &crate::contexts::WmCtx, text: &str) -> i32 {
    if ctx.x11_conn().is_none() {
        return 0;
    }
    // Transitional helper: avoid going through get_drw() when ctx is available.
    let Some(mut drw) = ctx
        .g
        .cfg
        .drw
        .as_ref()
        .and_then(|drw| drw.has_display().then(|| drw.clone()))
    else {
        return 0;
    };
    drw.fontset_getwidth(text) as i32
}

pub(crate) fn layout_symbol(m: &Monitor) -> String {
    m.layout_symbol()
}

pub fn get_layout_symbol_width(ctx: &WmCtx, m: &Monitor) -> i32 {
    text_width_ctx(ctx, &layout_symbol(m)) + ctx.g.cfg.horizontal_padding
}

pub fn draw_bar(ctx: &mut WmCtx, mon_idx: usize) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    if ctx.x11_conn().is_none() {
        return;
    }
    ctx.bar.recursion_enter();

    let m_info = match ctx.g.monitor(mon_idx) {
        Some(m) => {
            if !m.shows_bar() || ctx.bar.pausedraw() {
                ctx.bar.recursion_exit();
                return;
            }
            (m.num, m.work_rect.w, m.barwin)
        }
        None => {
            ctx.bar.recursion_exit();
            return;
        }
    };

    let monitor_num = m_info.0;
    let work_rect_w = m_info.1;
    let barwin = m_info.2;

    let bh = ctx.g.cfg.bar_height;
    ctx.g
        .cfg
        .drw
        .as_mut()
        .expect("draw_bar called before drw initialised")
        .resize(work_rect_w as u32, bh as u32);

    let is_selmon = ctx
        .g
        .selmon()
        .is_some_and(|selmon| selmon.num == monitor_num);

    let systray_width = if ctx.g.cfg.showsystray && is_selmon {
        crate::systray::get_systray_width(ctx) as i32
    } else {
        0
    };

    let (status_start_x, status_width) = if is_selmon {
        // Avoid borrowing ctx.g and ctx (mut) at once.
        let m = ctx.g.monitor(mon_idx).cloned().unwrap();
        status::draw_status_bar(ctx, &m, bh)
    } else {
        (0, 0)
    };

    if is_selmon {
        ctx.g.status_text_width = status_width;
    }

    widgets::draw_startmenu_icon(ctx, bh);

    {
        let ctx_imm = &*ctx;
        let m = ctx_imm.g.monitor(mon_idx).unwrap();
        x11::resize_bar_win_ctx(ctx_imm, m);
    }

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
        x = widgets::draw_tag_indicators(ctx_imm, m, x, occupied_tags, urgent_tags, bh);
        x = widgets::draw_layout_indicator(ctx_imm, m, x, bh);
    }

    // Draw the shutdown/power button directly after the layout indicator
    // when there is no selected client on this monitor (mirrors C behaviour).
    if !mon_has_sel {
        x = widgets::draw_shutdown_button(ctx, x, bh);
    }

    let title_end_x = if is_selmon {
        status_start_x
    } else {
        work_rect_w - systray_width
    };
    let title_width = (title_end_x - x).max(0);

    let mut new_activeoffset = None;
    if title_width > bh {
        let m = ctx.g.monitor(mon_idx).unwrap();
        let ctx_imm = &*ctx;
        new_activeoffset =
            widgets::draw_window_titles(ctx_imm, m, x, title_width, visible_clients, bh);
    }

    if let Some(m) = ctx.g.monitor_mut(mon_idx) {
        m.bt = visible_clients;
        m.bar_clients_width = title_width;
        if let Some(offset) = new_activeoffset {
            m.activeoffset = offset;
        }
    }

    ctx.g
        .cfg
        .drw
        .as_ref()
        .expect("draw_bar called before drw initialised")
        .map(barwin.into(), 0, 0, work_rect_w as u16, bh as u16);

    ctx.bar.recursion_exit();
}

pub fn draw_bars(ctx: &mut WmCtx) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    if ctx.x11_conn().is_none() {
        return;
    }
    let indices: Vec<usize> = ctx.g.monitors_iter().map(|(i, _)| i).collect();
    for i in indices {
        draw_bar(ctx, i);
    }
}

pub fn reset_bar(ctx: &mut WmCtx) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    if ctx.x11_conn().is_none() {
        return;
    }
    let selmon_idx = ctx.g.selmon_id();

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

    draw_bar(ctx, selmon_idx);
}
