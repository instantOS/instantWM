mod model;
mod status;
mod widgets;
mod x11;

use crate::globals::{get_globals, get_globals_mut};
use crate::types::*;
use model::{BarLayout, ClientBarStats};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

static DRAW_BAR_RECURSION: AtomicUsize = AtomicUsize::new(0);
const MAX_BAR_RECURSION: usize = 50;

/// Pause bar drawing (e.g. during animations).
pub static PAUSEDRAW: AtomicBool = AtomicBool::new(false);
/// Per-command click-region x-offsets; sentinel value -1 marks end of list.
const INIT_COMMAND_OFFSET: AtomicI32 = AtomicI32::new(-1);
pub static COMMANDOFFSETS: [AtomicI32; 20] = [INIT_COMMAND_OFFSET; 20];

pub fn text_width(text: &str) -> i32 {
    let g = get_globals();
    if let Some(ref drw) = g.drw {
        let mut drw = drw.clone();
        drw.fontset_getwidth(text) as i32
    } else {
        0
    }
}

pub(crate) fn layout_symbol(m: &MonitorInner) -> &str {
    if m.ltsymbol.is_empty() {
        "[]="
    } else {
        &m.ltsymbol
    }
}

pub fn get_layout_symbol_width(m: &MonitorInner) -> i32 {
    text_width(layout_symbol(m)) + get_lrpad()
}

pub fn draw_bar(m: &mut MonitorInner) {
    let count = DRAW_BAR_RECURSION.fetch_add(1, Ordering::SeqCst);
    if count > MAX_BAR_RECURSION {
        std::process::abort();
    }

    if PAUSEDRAW.load(Ordering::Relaxed) || !m.showbar {
        DRAW_BAR_RECURSION.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    {
        let g = get_globals_mut();
        let bh = g.bh;
        if let Some(ref mut drw) = g.drw {
            drw.set_drawable(m.barwin);
            drw.w = m.work_rect.w as u32;
            drw.h = bh as u32;
        }
    }

    let g = get_globals();
    let bh = g.bh;
    let is_selmon = g
        .selmon
        .and_then(|selmon_idx| g.monitors.get(selmon_idx))
        .map_or(false, |selmon| selmon.num == m.num);

    let mut layout = BarLayout::default();
    if g.showsystray && is_selmon {
        layout.systray_width = crate::systray::get_systray_width() as i32;
    }

    if is_selmon {
        layout.status_start_x = status::draw_status_bar(m, bh, &g.stext);
    }

    widgets::draw_startmenu_icon(bh);
    x11::resize_bar_win(m);

    let stats = ClientBarStats::collect(m, &g);
    let mut x = g.startmenusize;
    x = widgets::draw_tag_indicators(m, x, stats.occupied_tags, stats.urgent_tags, bh);
    x = widgets::draw_layout_indicator(m, x, bh);

    let status_offset = if is_selmon {
        layout.status_start_x
    } else {
        m.work_rect.w
    };
    layout.title_width = (m.work_rect.w - status_offset - x - layout.systray_width).max(0);

    if layout.title_width > bh {
        widgets::draw_window_titles(m, x, layout.title_width, stats.visible_clients, bh);
    }

    m.bt = stats.visible_clients;
    m.bar_clients_width = layout.title_width;

    if let Some(ref drw) = g.drw {
        unsafe {
            crate::drw::XFlush(drw.display());
        }
    }

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
    let Some(selmon_idx) = g.selmon else {
        return;
    };

    let should_reset = g
        .monitors
        .get(selmon_idx)
        .map_or(false, |selmon| selmon.gesture != Gesture::None);
    if !should_reset {
        return;
    }

    if let Some(selmon) = g.monitors.get_mut(selmon_idx) {
        selmon.gesture = Gesture::None;
        draw_bar(selmon);
    }
}

pub fn update_status() {
    x11::update_status();
}

pub fn update_bar_pos(m: &mut MonitorInner) {
    x11::update_bar_pos(m);
}

pub fn update_bars() {
    x11::update_bars();
}

pub fn toggle_bar(arg: &Arg) {
    x11::toggle_bar(arg);
}

pub(crate) fn get_lrpad() -> i32 {
    get_globals().lrpad
}
