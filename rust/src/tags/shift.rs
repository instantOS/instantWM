//! Moving clients between tags.

use crate::contexts::{CoreCtx, X11Ctx};
// focus() is used via focus_soft() in this module

use crate::animation::animate_client_x11;
use crate::layouts::arrange;
use crate::types::{Direction, OverlayMode, Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode, Window};

//TODO: this seems redundant
pub fn shift_tag_by(core: &mut CoreCtx, x11: &X11Ctx, dir: Direction, offset: i32) {
    shift_tag(core, x11, dir, offset.max(1));
}

pub fn move_client(core: &mut CoreCtx, x11: &X11Ctx, dir: Direction) {
    shift_tag_by(core, x11, dir, 1);
    crate::tags::view::scroll_view(core, x11, dir);
}

fn shift_tag(core: &mut CoreCtx, x11: &X11Ctx, dir: Direction, offset: i32) {
    let mon = core.g.selected_monitor();
    let Some(win) = mon.sel else {
        return;
    };

    let current_tag = mon.current_tag as u32;
    let overlay_win = mon.overlay;

    if Some(win) == overlay_win {
        let mode = match dir {
            Direction::Left => OverlayMode::Left,
            Direction::Right => OverlayMode::Right,
            Direction::Up => OverlayMode::Top,
            Direction::Down => OverlayMode::Bottom,
        };
        crate::overlay::set_overlay_mode(ctx, mode);
        return;
    }

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= 20 {
        return;
    }

    let (tagset, tagmask) = (mon.selected_tags(), core.g.tags.mask());

    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    clear_sticky(core, win);

    if core.g.animated {
        play_slide_animation(core, x11, win, dir);
    }

    if let Some(client) = core.g.clients.get_mut(&win) {
        match dir {
            Direction::Left if tagset > 1 => {
                client.tags >>= offset;
            }
            Direction::Right if (tagset & (tagmask >> 1)) != 0 => {
                client.tags <<= offset;
            }
            _ => return,
        }
    }

    let selected_monitor_id = core.g.selected_monitor_id();
    crate::focus::focus_soft_x11(core, x11, None);
    arrange(core, Some(selected_monitor_id));
}

fn clear_sticky(core: &mut CoreCtx, win: WindowId) {
    let target_tags = {
        let mon = core.g.selected_monitor();
        if mon.current_tag > 0 {
            Some(1u32 << (mon.current_tag - 1))
        } else {
            None
        }
    };

    if let Some(client) = core.g.clients.get_mut(&win) {
        if client.issticky {
            client.issticky = false;
            if let Some(tags) = target_tags {
                client.tags = tags;
            }
        }
    }
}

fn play_slide_animation(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId, dir: Direction) {
    let x11_win: Window = win.into();
    let _ = x11.conn.configure_window(
        x11_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = x11.conn.flush();

    let mon_w = core.g.selected_monitor().monitor_rect.w;
    let (client_x, client_y) = core
        .g
        .clients
        .get(&win)
        .map(|c| (c.geo.x, c.geo.y))
        .unwrap_or((0, 0));

    let anim_dx = (mon_w / 10)
        * match dir {
            Direction::Left => -1,
            Direction::Right => 1,
            Direction::Up => -1,
            Direction::Down => 1,
        };

    crate::animation::animate_client_x11(
        core,
        x11,
        win,
        &Rect {
            x: client_x + anim_dx,
            y: client_y,
            w: 0,
            h: 0,
        },
        0,
        7,
    );
}
