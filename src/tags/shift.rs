//! Moving clients between tags.

use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module

use crate::backend::BackendOps;
use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::geometry::MoveResizeOptions;
use crate::tags::sticky::reset_sticky_win;
use crate::types::{Direction, HorizontalDirection, OverlayMode, Rect, TagMask, WindowId};

pub fn move_client(ctx: &mut WmCtx, dir: HorizontalDirection) {
    shift_tag(ctx, dir.into(), 1);
    crate::tags::view::scroll_view(ctx, dir);
}

pub fn shift_tag(ctx: &mut WmCtx, dir: Direction, offset: i32) {
    let (win, current_tag, overlay_win, tagset, tagmask, animated) = {
        let mon = ctx.core().globals().selected_monitor();
        let Some(win) = mon.sel else {
            return;
        };
        let Some(current_tag) = mon.current_tag else {
            return;
        };
        (
            win,
            current_tag,
            mon.overlay,
            mon.selected_tags(),
            ctx.core().globals().tags.mask(),
            ctx.core().globals().behavior.animated,
        )
    };

    if Some(win) == overlay_win {
        let mode = match dir {
            Direction::Left => OverlayMode::Left,
            Direction::Right => OverlayMode::Right,
            Direction::Up => OverlayMode::Top,
            Direction::Down => OverlayMode::Bottom,
        };
        crate::floating::set_overlay_mode(ctx, mode);
        return;
    }

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= 20 {
        return;
    }

    if !(tagset & tagmask).is_single() {
        return;
    }

    reset_sticky_win(ctx.core_mut(), win);

    if animated {
        play_slide_animation(ctx, win, dir);
    }

    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        match dir {
            Direction::Left if current_tag > 1 => {
                client.update_tag_mask(|tags| TagMask::from_bits(tags.bits() >> offset));
            }
            Direction::Right
                if current_tag < 20
                    && tagset.intersects(TagMask::from_bits(tagmask.bits() >> 1)) =>
            {
                client.update_tag_mask(|tags| TagMask::from_bits(tags.bits() << offset));
            }
            _ => return,
        }
    }

    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    crate::focus::focus_soft(ctx, None);
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
}

fn play_slide_animation(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    ctx.backend().raise_window_visual_only(win);
    let mon_w = ctx.core().globals().selected_monitor().monitor_rect.w;
    let geo = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| c.geo)
        .unwrap_or_default();

    let anim_dx = (mon_w / 10)
        * match dir {
            Direction::Left => -1,
            Direction::Right => 1,
            Direction::Up => -1,
            Direction::Down => 1,
        };

    ctx.move_resize(
        win,
        Rect {
            w: geo.w.max(1),
            h: geo.h.max(1),
            ..geo
        },
        MoveResizeOptions::animate_from(
            Rect {
                x: geo.x + anim_dx,
                y: geo.y,
                w: geo.w.max(1),
                h: geo.h.max(1),
            },
            DEFAULT_FRAME_COUNT,
        ),
    );
}
