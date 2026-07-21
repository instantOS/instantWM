//! Moving clients between tags.

use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module

use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::geometry::MoveResizeOptions;
use crate::types::{Direction, HorizontalDirection, Rect, TagMask, WindowId};

pub fn move_client_follow_view(ctx: &mut WmCtx, dir: HorizontalDirection) {
    let Some(win) = ctx.core().model().selected_win() else {
        return;
    };
    shift_tag(ctx, dir.into(), 1);
    crate::tags::view::scroll_view(ctx, dir);

    // `shift_tag` and `scroll_view` deliberately use generic focus fallback,
    // but this combined command promises to keep interacting with the window
    // it moved. Verify that the move reached the displayed view before
    // restoring that explicit focus target.
    let monitor_id = ctx.core().model().selected_monitor_id();
    if !ctx
        .core()
        .model()
        .client_is_visible_on_selected_monitor(win)
    {
        return;
    }

    crate::focus::focus(ctx, Some(win));
    // Cursor placement must use destination geometry, not the stale rectangle
    // from the tag we just left.
    crate::layouts::arrange(ctx, Some(monitor_id));
    if ctx.core().behavior().focus_follows_mouse {
        ctx.warp_cursor_to_client_center(win);
    }
}

pub fn shift_tag(ctx: &mut WmCtx, dir: Direction, offset: i32) {
    let (win, current_tag, tagset, tagmask, animated) = {
        let mon = ctx.core().model().expect_selected_monitor();
        let Some(win) = mon.selected else {
            return;
        };
        let Some(current_tag) = mon.current_tag_number() else {
            return;
        };
        (
            win,
            current_tag,
            mon.selected_tags(),
            ctx.core().model().tags.mask(),
            ctx.core().behavior().animated,
        )
    };

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= 20 {
        return;
    }

    if !(tagset & tagmask).is_single() {
        return;
    }

    let target_tags = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .current_tag_number();

    // Get mutable borrow for reset_sticky, then drop it
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.reset_sticky(target_tags);
    }

    if animated {
        play_slide_animation(ctx, win, dir);
    }

    // Re-borrow for tag mask update
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
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

    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    crate::focus::focus(ctx, None);
    ctx.core_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
}

fn play_slide_animation(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    ctx.window_backend().raise_window_visual_only(win);
    let mon_w = ctx.core().model().expect_selected_monitor().monitor_rect.w;
    let Some(geo) = ctx.core().state().model.client(win).map(|c| c.geo) else {
        return;
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, ClientMode, Monitor};
    use crate::wm::Wm;

    #[test]
    fn move_and_follow_keeps_the_moved_window_selected() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 3;
        let tag1 = TagMask::single(1).expect("tag 1");
        let tag2 = TagMask::single(2).expect("tag 2");
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        wm.core
            .model
            .monitor_mut(monitor_id)
            .expect("monitor")
            .set_selected_tags(tag1);

        let moved = WindowId(1);
        let destination_peer = WindowId(2);
        wm.core.model.insert_client(Client {
            win: moved,
            monitor_id,
            tags: tag1,
            mode: ClientMode::Tiling,
            ..Client::default()
        });
        wm.core.model.insert_client(Client {
            win: destination_peer,
            monitor_id,
            tags: tag2,
            mode: ClientMode::Tiling,
            ..Client::default()
        });
        let monitor = wm.core.model.monitor_mut(monitor_id).expect("monitor");
        monitor.clients = vec![moved, destination_peer];
        monitor.selected = Some(moved);

        move_client_follow_view(&mut wm.ctx(), HorizontalDirection::Right);

        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag2
        );
        assert_eq!(wm.core.model.selected_win(), Some(moved));
        assert_eq!(
            wm.core.model.client(moved).map(|client| client.tags),
            Some(tag2)
        );

        let tag3 = TagMask::single(3).expect("tag 3");
        move_client_follow_view(&mut wm.ctx(), HorizontalDirection::Right);
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag3
        );
        assert_eq!(wm.core.model.selected_win(), Some(moved));
        assert_eq!(
            wm.core.model.client(moved).map(|client| client.tags),
            Some(tag3)
        );
    }
}
