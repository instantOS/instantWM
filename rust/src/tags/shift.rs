//! Moving clients between tags.

use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module

use crate::layouts::arrange;
use crate::types::{Direction, OverlayMode, Rect, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode, Window};

pub fn shift_tag_by(ctx: &mut WmCtx, dir: Direction, offset: i32) {
    shift_tag(ctx, dir, offset.max(1));
}

pub fn move_client(ctx: &mut WmCtx, dir: Direction) {
    shift_tag_by(ctx, dir, 1);
    crate::tags::view::scroll_view(ctx, dir);
}

fn shift_tag(ctx: &mut WmCtx, dir: Direction, offset: i32) {
    let Some(win) = ctx.g.selmon().and_then(|mon| mon.sel) else {
        return;
    };

    let (current_tag, overlay_win) = (
        ctx.g.selmon().map(|m| m.current_tag as u32),
        ctx.g.selmon().and_then(|m| m.overlay),
    );

    let Some(current_tag) = current_tag else {
        return;
    };

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

    let (tagset, tagmask) = match ctx.g.selmon() {
        Some(mon) => (mon.tagset[mon.seltags as usize], ctx.g.tags.mask()),
        None => return,
    };

    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    clear_sticky(ctx, win);

    if ctx.g.animated {
        play_slide_animation(ctx, win, dir);
    }

    if let Some(client) = ctx.g.clients.get_mut(&win) {
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

    let selmon = ctx.g.selmon_id();
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selmon));
}

fn clear_sticky(ctx: &mut WmCtx, win: WindowId) {
    let target_tags = ctx.g.selmon().and_then(|mon| {
        if mon.current_tag > 0 {
            Some(1u32 << (mon.current_tag - 1))
        } else {
            None
        }
    });

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        if client.issticky {
            client.issticky = false;
            if let Some(tags) = target_tags {
                client.tags = tags;
            }
        }
    }
}

fn play_slide_animation(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        let x11_win: Window = win.into();
        let _ = conn.configure_window(
            x11_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }

    let mon_w = ctx.g.selmon().map(|m| m.monitor_rect.w).unwrap_or(0);
    let (client_x, client_y) = ctx
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

    crate::animation::animate_client(
        ctx,
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
