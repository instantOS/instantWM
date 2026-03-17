//! Moving clients between tags.

use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module

use crate::backend::BackendOps;
use crate::layouts::arrange;
use crate::tags::sticky::reset_sticky_win;
use crate::types::{Direction, OverlayMode, Rect, WindowId};

pub fn move_client(ctx: &mut WmCtx, dir: Direction) {
    shift_tag(ctx, dir, 1);
    crate::tags::view::scroll_view(ctx, dir);
}

pub fn shift_tag(ctx: &mut WmCtx, dir: Direction, offset: i32) {
    let (win, current_tag, overlay_win, tagset, tagmask, animated) = {
        let mon = ctx.g().selected_monitor();
        let Some(win) = mon.sel else {
            return;
        };
        (
            win,
            mon.current_tag as u32,
            mon.overlay,
            mon.selected_tags(),
            ctx.g().tags.mask(),
            ctx.g().behavior.animated,
        )
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

    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    reset_sticky_win(ctx.core_mut(), win);

    if animated {
        play_slide_animation(ctx, win, dir);
    }

    if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
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

    let selected_monitor_id = ctx.g().selected_monitor_id();
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, Some(selected_monitor_id));
}

fn play_slide_animation(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    ctx.backend().raise_window(win);
    let mon_w = ctx.g().selected_monitor().monitor_rect.w;
    let (client_x, client_y) = ctx
        .g()
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
