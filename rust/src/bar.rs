mod model;
mod status;
mod widgets;
pub mod x11;

pub use model::{bar_position_at_x, BarPosition};
pub use x11::{resize_bar_win, resize_bar_win_ctx};

use crate::contexts::WmCtx;
use crate::globals::{get_drw, get_drw_mut, get_globals, get_globals_mut, get_x11};
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

pub fn draw_bar(ctx: &mut WmCtx, m: &mut Monitor) {
    let count = DRAW_BAR_RECURSION.fetch_add(1, Ordering::SeqCst);
    if count > MAX_BAR_RECURSION {
        std::process::abort();
    }

    let showbar = m.shows_bar();
    if PAUSEDRAW.load(Ordering::Relaxed) || !showbar {
        DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    {
        let bh = ctx.g.cfg.bh;
        get_drw_mut().resize(m.work_rect.w as u32, bh as u32);
    }

    let bh = ctx.g.cfg.bh;
    let is_selmon = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .is_some_and(|selmon| selmon.num == m.num);

    let systray_width = if ctx.g.cfg.showsystray && is_selmon {
        crate::systray::get_systray_width(ctx) as i32
    } else {
        0
    };

    let status_start_x = if is_selmon {
        status::draw_status_bar(ctx, m, bh)
    } else {
        0
    };

    widgets::draw_startmenu_icon(ctx, bh);
    x11::resize_bar_win_ctx(ctx, m);

    let stats = ClientBarStats::collect(m, ctx.g);
    let mut x = ctx.g.cfg.startmenusize;
    x = widgets::draw_tag_indicators(ctx, m, x, stats.occupied_tags, stats.urgent_tags, bh);
    x = widgets::draw_layout_indicator(ctx, m, x, bh);

    let title_end_x = if is_selmon {
        status_start_x
    } else {
        m.work_rect.w - systray_width
    };
    let title_width = (title_end_x - x).max(0);

    if title_width > bh {
        widgets::draw_window_titles(m, x, title_width, stats.visible_clients, bh);
    }

    m.bt = stats.visible_clients;
    m.bar_clients_width = title_width;

    get_drw().map(m.barwin, 0, 0, m.work_rect.w as u16, bh as u16);

    DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
}

pub fn draw_bars(ctx: &mut WmCtx) {
    let monitor_count = ctx.g.monitors.len();
    for i in 0..monitor_count {
        if let Some(m) = ctx.g.monitors.get_mut(i) {
            // We need to be careful about borrowing here
            // For now, create a temporary ctx for each iteration
            let x11 = get_x11();
            let g = get_globals_mut();
            let mut ctx = WmCtx::new(g, x11.as_conn());
            if let Some(m) = ctx.g.monitors.get_mut(i) {
                draw_bar(&mut ctx, m);
            }
        }
    }
}

pub fn reset_bar(ctx: &mut WmCtx) {
    let selmon_idx = ctx.g.selmon;

    let should_reset = ctx
        .g
        .monitors
        .get(selmon_idx)
        .is_some_and(|selmon| selmon.gesture != Gesture::None);
    if !should_reset {
        return;
    }

    if let Some(selmon) = ctx.g.monitors.get_mut(selmon_idx) {
        selmon.gesture = Gesture::None;
        draw_bar(ctx, selmon);
    }
}

pub(crate) fn get_lrpad() -> i32 {
    get_globals().cfg.lrpad
}
