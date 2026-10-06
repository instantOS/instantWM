//! Keyboard-driven floating window movement, resize, and scaling.

use crate::constants::animation::FLOAT_MOVE_ANIMATION_MILLIS;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::types::*;

/// Move a floating client by one keyboard step.
///
/// Returns whether its geometry changed. A `false` horizontal result lets the
/// key dispatcher continue the same movement onto an adjacent tag.
pub fn key_move(ctx: &mut WmCtx, win: WindowId, dir: Direction) -> bool {
    crate::client::fullscreen::leave_maximized(ctx, win);
    let Some(view) = ctx.core().state.model.client_view(win) else {
        return false;
    };
    let is_floating = view.client.mode().is_normal_floating();
    let geo = view.client.geo;
    let border_width = view.client.border_width;
    let mon_rect = view.monitor.monitor_rect;

    if view.monitor.is_tiling_layout() && !is_floating {
        return false;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.bottom() {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.right() {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    let target = Rect {
        x: new_x,
        y: new_y,
        w: geo.w,
        h: geo.h,
    };
    if target == geo {
        return false;
    }

    ctx.raise_client(win);
    ctx.move_resize(
        win,
        target,
        MoveResizeOptions::animate_to(FLOAT_MOVE_ANIMATION_MILLIS),
    );
    ctx.warp_cursor_to_client(win);
    true
}

pub fn key_resize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    crate::client::fullscreen::leave_maximized(ctx, win);
    let Some(view) = ctx.core().state.model.client_view(win) else {
        return;
    };
    let is_floating = view.client.mode().is_normal_floating();
    let geo = view.client.geo;
    let has_tiling = view.monitor.is_tiling_layout();

    super::snap::reset_snap(ctx, win);

    if has_tiling && !is_floating {
        return;
    }

    ctx.raise_client(win);
    const RESIZE_STEP: i32 = 40;
    let (dw, dh) = dir.delta(RESIZE_STEP);
    let nw = geo.w + dw;
    let nh = geo.h + dh;

    ctx.warp_cursor_to_client(win);

    ctx.move_resize(
        win,
        Rect {
            x: geo.x,
            y: geo.y,
            w: nw,
            h: nh,
        },
        MoveResizeOptions::hinted_immediate(true),
    );
}

pub fn center_window(ctx: &mut WmCtx, win: WindowId) {
    crate::client::fullscreen::leave_maximized(ctx, win);
    let Some(view) = ctx.core().state.model.client_view(win) else {
        return;
    };
    if view.client.is_edge_scratchpad() {
        return;
    }
    let geo = view.client.geo;
    let is_floating = view.client.mode().is_normal_floating();
    let work_rect = view.monitor.work_rect();
    let has_tiling = view.monitor.is_tiling_layout();

    if has_tiling && !is_floating {
        return;
    }

    if geo.w > work_rect.w || geo.h > work_rect.h {
        return;
    }

    // Center on the work area, which already excludes the built-in bar, an
    // external bar reserving the same edge, and any bottom bar.
    ctx.raise_client(win);
    ctx.move_resize(
        win,
        Rect {
            x: work_rect.x + (work_rect.w / 2) - (geo.w / 2),
            y: work_rect.y + (work_rect.h / 2) - (geo.h / 2),
            w: geo.w,
            h: geo.h,
        },
        MoveResizeOptions::hinted_immediate(true),
    );
}

#[cfg(test)]
mod tests {
    use super::{center_window, key_move};
    use crate::layouts::PresentationMode;
    use crate::test_support::MonitorBuilder;
    use crate::test_support::TestWm as Wm;
    use crate::types::{
        Client, ClientMode, ClientPlacement, Direction, Monitor, Rect, TagMask, WindowId,
    };

    #[test]
    fn moving_literal_floating_presentation_maximize_restores_and_clears_protocol_state() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let work_rect = Rect::new(0, 30, 1200, 770);
        let monitor_id = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .rect(Rect::new(0, 0, 1200, 800), work_rect)
                .build(),
        );
        wm.core.state.model.monitors.set_selected(monitor_id);
        wm.core
            .state
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = PresentationMode::Floating;
        let win = WindowId(71);
        let saved = Rect::new(200, 150, 600, 450);
        let mut client = Client {
            win,
            tags: TagMask::single(1).unwrap(),
            geo: work_rect,
            ..Client::default()
        };
        client.set_mode_for_test(ClientMode::maximized(ClientPlacement::Floating));
        client.save_floating_placement(saved, work_rect);
        assert!(wm.core.state.model.add_client(monitor_id, client));

        assert!(key_move(&mut wm.test_ctx(), win, Direction::Right));

        let client = wm.core.state.model.client(win).unwrap();
        assert!(client.mode().is_normal_floating());
        assert_eq!(client.geo, Rect::new(240, 150, 600, 450));
        assert_eq!(
            wm.core.state.model.client_protocol_maximized(win),
            Some(false)
        );
    }

    /// Center a floating window on a monitor built by `configure` and report
    /// the resulting `y` against the work-area center it should have matched.
    fn centered_y(configure: impl FnOnce(&mut Monitor)) -> (i32, i32) {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let mut monitor = MonitorBuilder::new()
            .rect(Rect::new(0, 0, 1200, 800), Rect::new(0, 0, 1200, 800))
            .bar(30, true)
            .tag_count(2)
            .selected_tags(TagMask::single(1).unwrap())
            .build();
        configure(&mut monitor);
        let work_rect = monitor.work_rect();
        let monitor_id = wm.core.state.model.monitors.push(monitor);
        wm.core.state.model.monitors.set_selected(monitor_id);
        // Backends publish the global screen rect during bootstrap; the
        // interactive position clamp reads it, so a bare `Wm` has to as well.
        wm.core.state.derived.display.width = 1200;
        wm.core.state.derived.display.height = 800;

        let win = WindowId(72);
        let mut client = Client {
            win,
            tags: TagMask::single(1).unwrap(),
            geo: Rect::new(100, 100, 400, 300),
            ..Client::default()
        };
        client.set_placement(ClientPlacement::Floating);
        assert!(wm.core.state.model.add_client(monitor_id, client));

        center_window(&mut wm.test_ctx(), win);

        let centered = wm.core.state.model.client(win).unwrap().geo;
        assert_eq!(centered.w, 400);
        assert_eq!(centered.h, 300);
        (centered.y, work_rect.y + (work_rect.h / 2) - 150)
    }

    #[test]
    fn center_window_centers_in_the_work_area_with_the_bar_drawn() {
        let (centered, expected) = centered_y(|monitor| {
            monitor.per_tag_state().show_bar = Some(true);
        });

        assert_eq!(expected, 30 + 385 - 150);
        assert_eq!(centered, expected);
    }

    #[test]
    fn center_window_centers_in_the_work_area_with_the_bar_hidden() {
        // Regression: centering offset y by `bar_height` in one direction or the
        // other based on a hand-maintained bar check, so a hidden bar pulled
        // the window up by a full bar height instead of letting the taller work
        // area recenter it.
        let (centered, expected) = centered_y(|monitor| {
            monitor.per_tag_state().show_bar = Some(false);
        });

        assert_eq!(expected, 400 - 150);
        assert_eq!(centered, expected);
    }

    #[test]
    fn center_window_clears_an_external_bar_reserving_the_same_edge() {
        let (centered, expected) = centered_y(|monitor| {
            monitor.set_available_rect(Rect::new(0, 40, 1200, 760));
        });

        assert_eq!(expected, 40 + 380 - 150);
        assert_eq!(centered, expected);
    }
}
