mod model;
mod status;
mod widgets;
pub mod x11;

pub use model::{bar_position_at_x, bar_position_to_gesture};
pub use x11::{resize_bar_win, resize_bar_win_ctx};

use crate::contexts::WmCtx;
use crate::globals::{get_drw, get_drw_mut, get_globals};
use crate::types::*;
use model::ClientBarStats;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

static DRAW_BAR_RECURSION: AtomicUsize = AtomicUsize::new(0);
const MAX_BAR_RECURSION: usize = 50;

/// Pause bar drawing (e.g. during animations).
pub static PAUSEDRAW: AtomicBool = AtomicBool::new(false);
/// Per-command click-region x-offsets; sentinel value -1 marks end of list.
const INIT_COMMAND_OFFSET: AtomicI32 = AtomicI32::new(-1);
pub static COMMANDOFFSETS: [AtomicI32; 20] = [INIT_COMMAND_OFFSET; 20];

pub fn text_width(text: &str) -> i32 {
    let mut drw = get_drw().clone();
    drw.fontset_getwidth(text) as i32
}

pub(crate) fn layout_symbol(m: &Monitor) -> String {
    let _g = get_globals();
    m.layout_symbol()
}

pub fn get_layout_symbol_width(m: &Monitor) -> i32 {
    text_width(&layout_symbol(m)) + get_lrpad()
}

pub fn draw_bar(ctx: &mut WmCtx, mon_idx: usize) {
    let count = DRAW_BAR_RECURSION.fetch_add(1, Ordering::SeqCst);
    if count > MAX_BAR_RECURSION {
        std::process::abort();
    }

    let m_info = match ctx.g.monitor(mon_idx) {
        Some(m) => {
            if !m.shows_bar() || PAUSEDRAW.load(Ordering::Relaxed) {
                DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
                return;
            }
            (m.num, m.work_rect.w, m.barwin)
        }
        None => {
            DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
            return;
        }
    };

    let monitor_num = m_info.0;
    let work_rect_w = m_info.1;
    let barwin = m_info.2;

    let bh = ctx.g.cfg.bh;
    get_drw_mut().resize(work_rect_w as u32, bh as u32);

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
        let ctx_imm = &*ctx;
        let m = ctx_imm.g.monitor(mon_idx).unwrap();
        status::draw_status_bar(ctx_imm, m, bh)
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
        new_activeoffset = widgets::draw_window_titles(m, x, title_width, visible_clients, bh);
    }

    if let Some(m) = ctx.g.monitor_mut(mon_idx) {
        m.bt = visible_clients;
        m.bar_clients_width = title_width;
        if let Some(offset) = new_activeoffset {
            m.activeoffset = offset;
        }
    }

    get_drw().map(barwin, 0, 0, work_rect_w as u16, bh as u16);

    DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
}

pub fn draw_bars(ctx: &mut WmCtx) {
    let indices: Vec<usize> = ctx.g.monitors_iter().map(|(i, _)| i).collect();
    for i in indices {
        draw_bar(ctx, i);
    }
}

pub fn reset_bar(ctx: &mut WmCtx) {
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

pub(crate) fn get_lrpad() -> i32 {
    get_globals().cfg.lrpad
}
