mod model;
mod status;
mod widgets;
pub mod x11;

pub use model::{bar_position_at_x, BarPosition};
pub use x11::resize_bar_win;

use crate::globals::{get_drw, get_drw_mut, get_globals, get_globals_mut};
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
    let g = get_globals();
    crate::monitor::get_current_ltsymbol(m, &g.tags, &g.layouts)
}

pub fn get_layout_symbol_width(m: &Monitor) -> i32 {
    text_width(&layout_symbol(m)) + get_lrpad()
}

pub fn draw_bar(m: &mut Monitor) {
    let count = DRAW_BAR_RECURSION.fetch_add(1, Ordering::SeqCst);
    if count > MAX_BAR_RECURSION {
        std::process::abort();
    }

    let g = get_globals();
    let showbar = crate::monitor::get_current_showbar(m, &g.tags);
    if PAUSEDRAW.load(Ordering::Relaxed) || !showbar {
        DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    {
        let g = get_globals_mut();
        let bh = g.bh;
        get_drw_mut().resize(m.work_rect.w as u32, bh as u32);
    }

    let g = get_globals();
    let bh = g.bh;
    let is_selmon = g
        .monitors
        .get(g.selmon)
        .is_some_and(|selmon| selmon.num == m.num);

    let systray_width = if g.showsystray && is_selmon {
        crate::systray::get_systray_width() as i32
    } else {
        0
    };

    let status_start_x = if is_selmon {
        status::draw_status_bar(m, bh, &g.status_text)
    } else {
        0
    };

    widgets::draw_startmenu_icon(bh);
    x11::resize_bar_win(m);

    let stats = ClientBarStats::collect(m, g);
    let mut x = g.startmenusize;
    x = widgets::draw_tag_indicators(m, x, stats.occupied_tags, stats.urgent_tags, bh);
    x = widgets::draw_layout_indicator(m, x, bh);

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

pub fn draw_bars() {
    let monitor_count = get_globals().monitors.len();
    for i in 0..monitor_count {
        let g = get_globals_mut();
        if let Some(m) = g.monitors.get_mut(i) {
            draw_bar(m);
        }
    }
}

pub fn reset_bar() {
    let g = get_globals_mut();
    let selmon_idx = g.selmon;

    let should_reset = g
        .monitors
        .get(selmon_idx)
        .is_some_and(|selmon| selmon.gesture != Gesture::None);
    if !should_reset {
        return;
    }

    if let Some(selmon) = g.monitors.get_mut(selmon_idx) {
        selmon.gesture = Gesture::None;
        draw_bar(selmon);
    }
}

pub(crate) fn get_lrpad() -> i32 {
    get_globals().lrpad
}
