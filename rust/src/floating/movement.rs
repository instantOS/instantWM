//! Keyboard-driven floating window movement, resize, and scaling.

use crate::animation::animate_client;
use crate::client::resize;
use crate::focus::warp_cursor_to_client;
use crate::globals::get_globals;
use crate::types::*;
use crate::util::get_sel_win;
use x11rb::protocol::xproto::Window;

pub fn moveresize(dir: CardinalDirection) {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let (is_floating, geo, border_width) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo, c.border_width),
            None => return,
        }
    };

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.move_delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    let mon_rect = {
        let globals = get_globals();
        match globals.monitors.get(globals.selmon) {
            Some(m) => m.monitor_rect,
            None => return,
        }
    };

    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.y + mon_rect.h {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.x + mon_rect.w {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    animate_client(
        win,
        &Rect {
            x: new_x,
            y: new_y,
            w: geo.w,
            h: geo.h,
        },
        5,
        0,
    );
    warp_cursor_to_client(win);
}

pub fn key_resize(dir: CardinalDirection) {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let (is_floating, geo) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo),
            None => return,
        }
    };

    super::snap::reset_snap(win);

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    const RESIZE_STEP: i32 = 40;
    let (dw, dh) = dir.resize_delta(RESIZE_STEP);
    let nw = geo.w + dw;
    let nh = geo.h + dh;

    warp_cursor_to_client(win);
    resize(
        win,
        &Rect {
            x: geo.x,
            y: geo.y,
            w: nw,
            h: nh,
        },
        true,
    );
}

pub fn center_window() {
    let sel_win = {
        let mon = match get_globals().monitors.get(get_globals().selmon) {
            Some(m) => m,
            None => return,
        };
        match mon.sel {
            Some(sel) if Some(sel) != mon.overlay => Some(sel),
            _ => None,
        }
    };
    let Some(win) = sel_win else { return };

    let (geo, is_floating) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.geo, c.isfloating),
            None => return,
        }
    };

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    let (work_rect, mon_rect, showbar, bh) = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        (mon.work_rect, mon.monitor_rect, mon.showbar, globals.bh)
    };

    if geo.w > work_rect.w || geo.h > work_rect.h {
        return;
    }

    let y_offset = if showbar { bh } else { -bh };

    resize(
        win,
        &Rect {
            x: mon_rect.x + (work_rect.w / 2) - (geo.w / 2),
            y: mon_rect.y + (work_rect.h / 2) - (geo.h / 2) + y_offset,
            w: geo.w,
            h: geo.h,
        },
        true,
    );
}

pub fn upscale_client() {
    if let Some(win) = get_sel_win() {
        scale_client_win(win, 30);
    }
}

pub fn downscale_client() {
    let Some(win) = get_sel_win() else { return };

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if !is_floating {
        crate::focus::focus(Some(win));
        super::state::toggle_floating();
    }

    scale_client_win(win, -30);
}

pub fn scale_client_win(win: Window, scale: i32) {
    let (is_floating, geo) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo),
            None => return,
        }
    };

    if !is_floating {
        return;
    }

    let (mon_rect, bh) = {
        let globals = get_globals();
        match globals.monitors.get(globals.selmon) {
            Some(m) => (m.monitor_rect, globals.bh),
            None => return,
        }
    };

    let mut w = geo.w + scale;
    let mut h = geo.h + scale;
    let mut x = geo.x - scale / 2;
    let mut y = geo.y - scale / 2;

    x = x.max(mon_rect.x);
    w = w.min(mon_rect.w);
    h = h.min(mon_rect.h);
    if h + y > mon_rect.y + mon_rect.h {
        y = mon_rect.h - h;
    }
    y = y.max(bh);

    animate_client(win, &Rect { x, y, w, h }, 3, 0);
}
