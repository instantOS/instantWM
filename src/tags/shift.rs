//! Moving clients between tags.

use crate::contexts::WmCtx;

use crate::constants::animation::DEFAULT_ANIMATION_MILLIS;
use crate::geometry::MoveResizeOptions;
use crate::types::{HorizontalDirection, Rect, TagMask, WindowId};

pub fn move_client_follow_view(ctx: &mut WmCtx, dir: HorizontalDirection) -> bool {
    let Some(win) = ctx.core().model().selected_win() else {
        return false;
    };
    let Some(target_tags) = shift_tag(ctx, dir) else {
        return false;
    };
    crate::tags::view::view_tags(ctx, target_tags);

    // `shift_tag` and `view_tags` deliberately use generic focus fallback,
    // but this combined command promises to keep interacting with the window
    // it moved, provided it is actually shown there.
    let monitor_id = ctx.core().model().selected_monitor_id();
    if !ctx
        .core()
        .model()
        .client_is_visible_on_selected_monitor(win)
    {
        return false;
    }

    crate::focus::focus(ctx, Some(win));
    // Cursor placement must use destination geometry, not the stale rectangle
    // from the tag we just left.
    crate::layouts::arrange(ctx, Some(monitor_id));
    if ctx.core().config().window.focus_follows_mouse.is_enabled() {
        ctx.warp_cursor_to_client_center(win);
    }
    true
}

/// Move the selected client from the single viewed tag to its neighbour.
/// Returns the destination tag when the client moved.
pub fn shift_tag(ctx: &mut WmCtx, dir: HorizontalDirection) -> Option<TagMask> {
    let (win, current_tag, target_tags) = {
        let model = ctx.core().model();
        let mon = model.expect_selected_monitor();
        let current_tags = mon.selected_tags() & model.tags.mask();
        let target_tags =
            crate::tags::view::adjacent_scroll_mask(current_tags, dir, model.tags.count())?;
        (mon.selected?, current_tags.first_tag(), target_tags)
    };

    if ctx.core().model().client(win)?.is_scratchpad() {
        let monitor_id = ctx.core().model().selected_monitor_id();
        return crate::floating::scratchpad::scratchpad_restore_window(
            ctx,
            win,
            Some((monitor_id, target_tags)),
        )
        .is_ok()
        .then_some(target_tags);
    }

    ctx.core_mut()
        .model_mut()
        .client_mut(win)?
        .reset_sticky(current_tag);

    if ctx.core().config().animations.enabled {
        play_slide_animation(ctx, win, dir);
    }

    ctx.core_mut()
        .model_mut()
        .client_mut(win)?
        .update_tag_mask(|tags| match dir {
            HorizontalDirection::Left => TagMask::from_bits(tags.bits() >> 1),
            HorizontalDirection::Right => TagMask::from_bits(tags.bits() << 1),
        });

    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    crate::focus::focus(ctx, None);
    ctx.core_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
    Some(target_tags)
}

fn play_slide_animation(ctx: &mut WmCtx, win: WindowId, dir: HorizontalDirection) {
    ctx.window_backend().raise_window_visual_only(win);
    let mon_w = ctx.core().model().expect_selected_monitor().monitor_rect.w;
    let Some(geo) = ctx.core().client_geo(win) else {
        return;
    };

    let anim_dx = (mon_w / 10)
        * match dir {
            HorizontalDirection::Left => -1,
            HorizontalDirection::Right => 1,
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
            DEFAULT_ANIMATION_MILLIS,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::test_support::{add_client, add_selected_client};
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
        // Adoption prepends to the monitor's focus stack, so the peer is added
        // first to leave the stack in the `moved`-then-`destination_peer` order
        // that the explicit list used to spell out.
        add_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win: destination_peer,
                tags: tag2,
                mode: ClientMode::tiled(),
                ..Client::default()
            },
        );
        add_selected_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win: moved,
                tags: tag1,
                mode: ClientMode::tiled(),
                ..Client::default()
            },
        );

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
