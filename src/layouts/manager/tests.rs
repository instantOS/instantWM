use super::{
    available_tree_resize_direction, compute_monitor_z_order, pointer_tree_gap_resize_start,
    shifted_master_count,
};
use crate::config::config_toml::LayoutConfig;
use crate::layouts::PresentationMode;
use crate::layouts::tree::{Preset, Side};
use crate::test_support::{add_client, add_selected_client};
use crate::types::{
    Client, ClientMode, ClientPlacement, InteractionSource, Monitor, MonitorUiMetrics, MouseButton,
    Point, Rect, ResizeDirection, Size, TagMask, WindowId,
};
use std::collections::HashMap;

fn visible_client(win: WindowId) -> Client {
    let mut client = Client {
        win,
        ..Client::default()
    };
    client.set_tag_mask(TagMask::single(1).unwrap());
    client
}

fn wayland_wm() -> crate::wm::Wm {
    crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ))
}

/// Select a new bar-less monitor showing `windows` tiled on tag 1, with the
/// first one focused.
fn add_tiled_monitor(
    wm: &mut crate::wm::Wm,
    windows: &[WindowId],
    monitor_rect: Rect,
) -> crate::types::MonitorId {
    let tags = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect,
        available_rect: monitor_rect,
        bar_default_show: false,
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);
    // Adoption pushes each window onto the front of the focus list, so adding
    // back-to-front leaves it in `windows` order — the order these fixtures
    // describe. The first window is the one that ends up selected.
    for (index, &win) in windows.iter().enumerate().rev() {
        let client = Client {
            win,
            tags,
            mode: ClientMode::tiled(),
            ..Client::default()
        };
        if index == 0 {
            add_selected_client(&mut wm.core.model, monitor_id, client);
        } else {
            add_client(&mut wm.core.model, monitor_id, client);
        }
    }
    wm.core
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .set_selected_tags(tags);
    monitor_id
}

fn apply_preset(
    wm: &mut crate::wm::Wm,
    monitor_id: crate::types::MonitorId,
    preset: Preset,
    windows: &[WindowId],
) {
    wm.core
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .per_tag_state()
        .layout_tree
        .apply_preset(preset, windows, 1);
}

fn keyboard_config() -> crate::layouts::tree::CommandConfig {
    (&LayoutConfig::default()).into()
}

#[test]
fn inner_gap_offers_tree_resize_but_outer_gap_stays_desktop() {
    let mut wm = wayland_wm();
    wm.core.config.animations.enabled = false;
    wm.core.config.layout.inner_gap = 20;
    wm.core.config.layout.outer_gap = 20;
    let first = WindowId(1);
    let second = WindowId(2);
    let monitor_id = add_tiled_monitor(&mut wm, &[first, second], Rect::new(0, 0, 800, 600));
    super::arrange(&mut wm.ctx(), Some(monitor_id));

    let tiling = super::selected_tiling(&wm.ctx());
    let (slots, _) = tiling.slots(
        &wm.core
            .model
            .monitor(monitor_id)
            .unwrap()
            .per_tag()
            .unwrap()
            .layout_tree,
    );
    let first_geo = tiling.placement.client_rect(slots[&first], 0);
    let second_geo = tiling.placement.client_rect(slots[&second], 0);
    let gap = if first_geo.right() <= second_geo.x {
        Point::new((first_geo.right() + second_geo.x) / 2, first_geo.center().y)
    } else if second_geo.right() <= first_geo.x {
        Point::new(
            (second_geo.right() + first_geo.x) / 2,
            second_geo.center().y,
        )
    } else if first_geo.bottom() <= second_geo.y {
        Point::new(
            first_geo.center().x,
            (first_geo.bottom() + second_geo.y) / 2,
        )
    } else {
        Point::new(
            second_geo.center().x,
            (second_geo.bottom() + first_geo.y) / 2,
        )
    };

    assert!(
        pointer_tree_gap_resize_start(&wm.ctx(), gap).is_some(),
        "first={first_geo:?} second={second_geo:?} gap={gap:?}"
    );
    assert!(
        pointer_tree_gap_resize_start(&wm.ctx(), Point::new(10, first_geo.center().y)).is_none(),
        "the configured outer gap must retain root/desktop behavior"
    );
}

#[test]
fn monitor_arrange_consumes_only_its_pending_spawn_animations() {
    let mut wm = wayland_wm();
    let first = WindowId(1);
    let second = WindowId(2);
    let first_monitor = add_tiled_monitor(&mut wm, &[first], Rect::new(0, 0, 800, 600));
    let second_monitor = add_tiled_monitor(&mut wm, &[second], Rect::new(800, 0, 800, 600));
    wm.work.layout.clear();
    {
        let mut ctx = wm.ctx();
        ctx.core_mut()
            .queue_initial_window_layout(first, first_monitor);
        ctx.core_mut()
            .queue_initial_window_layout(second, second_monitor);
    }
    assert!(wm.work.layout.is_urgent());

    super::arrange(&mut wm.ctx(), Some(first_monitor));

    assert_eq!(
        wm.work.spawn_animations.iter().copied().collect::<Vec<_>>(),
        vec![second]
    );

    super::arrange(&mut wm.ctx(), Some(second_monitor));
    assert!(wm.work.spawn_animations.is_empty());
}

#[test]
fn spawn_flush_discards_destroyed_windows_without_consuming_other_monitors() {
    let mut wm = wayland_wm();
    let live = WindowId(1);
    let destroyed = WindowId(2);
    let unrelated_monitor = add_tiled_monitor(&mut wm, &[live], Rect::new(800, 0, 800, 600));
    let arranged_monitor = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 800, 600),
        available_rect: Rect::new(0, 0, 800, 600),
        bar_default_show: false,
        ..Monitor::default()
    });
    wm.work.spawn_animations.extend([live, destroyed]);

    super::arrange(&mut wm.ctx(), Some(arranged_monitor));

    assert_eq!(
        wm.work.spawn_animations.iter().copied().collect::<Vec<_>>(),
        vec![live]
    );
    super::arrange(&mut wm.ctx(), Some(unrelated_monitor));
    assert!(wm.work.spawn_animations.is_empty());
}

#[test]
fn disabled_animation_is_still_consumed_after_first_layout() {
    let mut wm = wayland_wm();
    let win = WindowId(1);
    let monitor_id = add_tiled_monitor(&mut wm, &[win], Rect::new(0, 0, 800, 600));
    wm.core.config.animations.enabled = false;
    wm.work.spawn_animations.insert(win);

    super::arrange(&mut wm.ctx(), Some(monitor_id));

    assert!(wm.work.spawn_animations.is_empty());
}

#[test]
fn arrange_invalidates_pointer_placement_candidates() {
    let mut wm = wayland_wm();
    let source = WindowId(1);
    let windows = [source, WindowId(2)];
    let monitor_id = add_tiled_monitor(&mut wm, &windows, Rect::new(0, 0, 400, 300));
    apply_preset(&mut wm, monitor_id, Preset::Grid, &windows);

    assert!(super::preview_tree_at_point(&mut wm.ctx(), source, Point::new(201, 150),).is_some());
    assert!(wm.core.interaction.pointer_placement_cache.is_some());

    super::arrange(&mut wm.ctx(), Some(monitor_id));
    assert!(wm.core.interaction.pointer_placement_cache.is_none());
}

#[test]
fn pointer_preview_and_release_share_the_normalized_candidate() {
    let mut wm = wayland_wm();
    let windows = (1..=20).map(WindowId).collect::<Vec<_>>();
    let monitor_id = add_tiled_monitor(&mut wm, &windows, Rect::new(0, 0, 2000, 1000));
    apply_preset(&mut wm, monitor_id, Preset::Grid, &windows);
    super::arrange(&mut wm.ctx(), Some(monitor_id));

    let source = windows[0];
    let point = Point::new(801, 625);
    let preview = super::preview_tree_at_point(&mut wm.ctx(), source, point)
        .expect("the test point must select a normalized edge candidate");

    assert!(super::place_tree_at_point(&mut wm.ctx(), source, point));
    let tiling = super::selected_tiling(&wm.ctx());
    let (slots, constraints_fit) = tiling.slots(
        &wm.core
            .model
            .expect_selected_monitor()
            .per_tag()
            .unwrap()
            .layout_tree,
    );
    assert!(constraints_fit);
    let applied_preview =
        tiling.outer_rect(wm.core.model.client(source).unwrap(), slots[&source], true);
    assert_eq!(
        applied_preview, preview,
        "release must apply the exact candidate displayed by pointer preview"
    );
}

#[test]
fn master_count_is_bounded_by_the_current_tiled_window_count() {
    assert_eq!(shifted_master_count(1, -1, 4), 0);
    assert_eq!(shifted_master_count(0, -1, 4), 0);
    assert_eq!(shifted_master_count(3, 1, 4), 4);
    assert_eq!(shifted_master_count(4, 1, 4), 4);
    assert_eq!(shifted_master_count(8, -1, 3), 2);
}

#[test]
fn master_count_change_is_rejected_before_mutation_during_tree_resize() {
    let mut wm = wayland_wm();
    let first = WindowId(1);
    let second = WindowId(2);
    let monitor_id = add_tiled_monitor(&mut wm, &[first, second], Rect::new(0, 0, 800, 600));
    apply_preset(&mut wm, monitor_id, Preset::MasterStack, &[first, second]);
    let origin = wm
        .core
        .model
        .monitor(monitor_id)
        .unwrap()
        .per_tag()
        .unwrap()
        .layout_tree
        .clone();

    wm.core
        .interaction
        .drag
        .begin_tree_resize(crate::core_state::TreeResizeStart {
            win: first,
            button: MouseButton::Right,
            source: InteractionSource::Pointer,
            direction: ResizeDirection::Right,
            start: Point::new(400, 300),
            geometry: Rect::new(0, 0, 400, 600),
            origin: origin.into(),
        })
        .unwrap();

    super::inc_master_count_by(&mut wm.ctx(), 1);

    assert_eq!(
        wm.core
            .model
            .monitor(monitor_id)
            .unwrap()
            .per_tag()
            .unwrap()
            .master_count,
        1
    );
}

/// A bar-bearing monitor that owns `order` bottom-to-top, with `selected`
/// focused.
///
/// The monitor is built outside any model, so it must own its clients itself:
/// `order` is both its focus list and its persistent z-order, and each window
/// gets a plain visible-on-tag-1 client.
fn monitor_with_order(order: &[WindowId], selected: WindowId) -> Monitor {
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(TagMask::single(1).unwrap());
    monitor.selected = Some(selected);
    monitor.bar_win = WindowId(99);
    for &win in order {
        monitor.adopt_client(visible_client(win), false);
    }
    assert!(monitor.set_focus_order(order.to_vec()));
    monitor
}

/// Adopt a fresh `visible_client(win)` into a monitor that is not in any model,
/// so no `WmModel::add_client` is available to do it.
///
/// Unlike `add_client` this *appends* to the focus list: fixtures that add a
/// window halfway through are written as an explicit oldest-first order, and
/// newest-first insertion would reverse the order they describe.
fn append_visible_client(monitor: &mut Monitor, win: WindowId) {
    monitor.adopt_client(visible_client(win), false);
    let mut order = monitor.stack.to_vec();
    order.rotate_left(1);
    assert!(monitor.set_focus_order(order));
}

#[test]
fn pointer_resize_falls_back_to_an_axis_present_in_the_tree() {
    assert_eq!(
        available_tree_resize_direction(
            ResizeDirection::Top,
            false,
            true,
            false,
            false,
            Point::new(80, 20),
            Size::new(100, 100),
        ),
        Some(ResizeDirection::Right)
    );
    assert_eq!(
        available_tree_resize_direction(
            ResizeDirection::Left,
            false,
            false,
            false,
            true,
            Point::new(20, 80),
            Size::new(100, 100),
        ),
        Some(ResizeDirection::Bottom)
    );
}

#[test]
fn pointer_resize_keeps_requested_corner_when_both_axes_exist() {
    assert_eq!(
        available_tree_resize_direction(
            ResizeDirection::TopLeft,
            true,
            true,
            true,
            true,
            Point::new(5, 5),
            Size::new(100, 100),
        ),
        Some(ResizeDirection::TopLeft)
    );
}

#[test]
fn pointer_tree_resize_remains_active_when_client_minimums_are_impossible() {
    let mut wm = wayland_wm();
    let windows = [WindowId(1), WindowId(2)];
    let monitor_id = add_tiled_monitor(&mut wm, &windows, Rect::new(0, 0, 300, 100));
    for win in windows {
        let client = wm.core.model.client_mut(win).unwrap();
        client.size_hints.min_width = 200;
        client.size_hints.min_height = 50;
    }
    apply_preset(&mut wm, monitor_id, Preset::MasterStack, &windows);
    let monitor = wm.core.model.monitor(monitor_id).unwrap();
    let origin = monitor.per_tag().unwrap().layout_tree.clone();
    let before = origin.bounds(monitor.available_rect)[&windows[0]];

    assert!(super::update_pointer_tree_resize(
        &mut wm.ctx(),
        windows[0],
        &origin,
        ResizeDirection::Right,
        Point::new(before.right(), before.y + before.h / 2),
        Point::new(before.right() + 30, before.y + before.h / 2),
    ));

    let monitor = wm.core.model.monitor(monitor_id).unwrap();
    let after = monitor
        .per_tag()
        .unwrap()
        .layout_tree
        .bounds(monitor.available_rect)[&windows[0]];
    assert_eq!(after.w, before.w + 30);
}

#[test]
fn arrange_commits_planned_borders_before_computing_geometry() {
    let win = WindowId(1);
    let mut monitor = monitor_with_order(&[win], win);
    monitor.monitor_rect = Rect::new(0, 0, 800, 600);
    monitor.available_rect = monitor.monitor_rect;
    let client = monitor.client_mut(win).unwrap();
    client.border_width = 2;
    client.old_border_width = 2;

    let plan = monitor.compute_arrange(&LayoutConfig::default(), true, false);

    assert_eq!(monitor.client(win).unwrap().border_width, 0);
    assert_eq!(plan.borders, [(win, 0)]);
    assert_eq!(plan.client_moves[0].rect, monitor.available_rect);
}

#[test]
fn tiled_focus_does_not_mutate_or_project_a_different_persistent_order() {
    let monitor = monitor_with_order(&[WindowId(1), WindowId(2), WindowId(3)], WindowId(2));

    let projected = compute_monitor_z_order(&monitor).unwrap();

    assert_eq!(
        projected,
        vec![WindowId(1), WindowId(2), WindowId(3), WindowId(99)]
    );
    assert_eq!(
        monitor.z_order.iter_bottom_to_top().collect::<Vec<_>>(),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );
}

#[test]
fn floating_focus_does_not_raise_within_the_floating_layer() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let mut monitor = monitor_with_order(&windows, WindowId(2));
    for win in windows {
        monitor
            .client_mut(win)
            .unwrap()
            .set_placement(ClientPlacement::Floating);
    }

    let projected = compute_monitor_z_order(&monitor).unwrap();

    assert_eq!(
        projected,
        vec![WindowId(99), WindowId(1), WindowId(2), WindowId(3)]
    );
}

#[test]
fn monitor_without_a_bar_window_is_not_projected_as_window_zero() {
    let mut monitor = monitor_with_order(&[WindowId(1), WindowId(2)], WindowId(2));
    monitor.bar_win = WindowId::default();
    monitor.bottom_bar_win = WindowId::default();

    let projected = compute_monitor_z_order(&monitor).unwrap();

    assert_eq!(projected, vec![WindowId(1), WindowId(2)]);
}

#[test]
fn transient_dialogs_stay_above_ordinary_windows_and_nested_children() {
    let mut monitor = monitor_with_order(
        &[WindowId(1), WindowId(3), WindowId(4), WindowId(2)],
        WindowId(2),
    );
    for win in [WindowId(1), WindowId(2), WindowId(3), WindowId(4)] {
        monitor
            .client_mut(win)
            .unwrap()
            .set_placement(ClientPlacement::Floating);
    }
    monitor.client_mut(WindowId(3)).unwrap().transient_for = Some(WindowId(1));
    monitor.client_mut(WindowId(4)).unwrap().transient_for = Some(WindowId(3));

    let projected = compute_monitor_z_order(&monitor).unwrap();

    assert_eq!(
        projected,
        vec![
            WindowId(99),
            WindowId(1),
            WindowId(2),
            WindowId(3),
            WindowId(4)
        ]
    );
}

#[test]
fn arrange_consumes_persistent_tree_instead_of_reapplying_grid() {
    let mut monitor = monitor_with_order(
        &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
        WindowId(1),
    );
    monitor.available_rect = crate::types::Rect::new(0, 0, 100, 100);
    let windows = monitor.stack.to_vec();
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);

    let first = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    assert!(monitor.per_tag_state().layout_tree.resize(
        WindowId(1),
        Side::Right,
        keyboard_config()
    ));
    let second = monitor.compute_arrange(&LayoutConfig::default(), true, false);

    let first_rect = first
        .client_moves
        .iter()
        .find(|output| output.win == WindowId(1))
        .unwrap()
        .rect;
    let second_rect = second
        .client_moves
        .iter()
        .find(|output| output.win == WindowId(1))
        .unwrap()
        .rect;
    assert_ne!(first_rect, second_rect);
}

#[test]
fn second_tiled_window_is_placed_in_the_left_half() {
    let mut monitor = monitor_with_order(&[WindowId(1)], WindowId(1));
    monitor.available_rect = Rect::new(0, 0, 1600, 900);
    monitor.monitor_rect = monitor.available_rect;
    let config = LayoutConfig::default();
    let _ = monitor.compute_arrange(&config, true, false);

    append_visible_client(&mut monitor, WindowId(2));
    let plan = monitor.compute_arrange(&config, true, false);
    let rects = plan
        .client_moves
        .iter()
        .map(|output| (output.win, output.rect))
        .collect::<HashMap<_, _>>();

    assert_eq!(rects[&WindowId(2)], Rect::new(0, 0, 800, 900));
    assert_eq!(rects[&WindowId(1)], Rect::new(800, 0, 800, 900));
}

#[test]
fn changing_new_window_policy_does_not_rewrite_an_existing_tree() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let mut monitor = monitor_with_order(&windows, WindowId(3));
    monitor.available_rect = Rect::new(0, 0, 1200, 800);
    monitor.monitor_rect = monitor.available_rect;
    let auto = LayoutConfig {
        new_window_placement: crate::config::config_toml::NewWindowPlacement::Auto,
        ..LayoutConfig::default()
    };
    let before = monitor.compute_arrange(&auto, true, false);
    let force = LayoutConfig {
        new_window_placement: crate::config::config_toml::NewWindowPlacement::Force,
        ..auto
    };
    let after = monitor.compute_arrange(&force, true, false);

    let rectangles = |plan: crate::layouts::ArrangePlan| {
        plan.client_moves
            .into_iter()
            .map(|output| (output.win, output.rect))
            .collect::<HashMap<_, _>>()
    };
    assert_eq!(rectangles(before), rectangles(after));
}

#[test]
fn arrange_reserves_tiled_minimum_sizes_without_overlap_or_overflow() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let mut monitor = monitor_with_order(&windows, WindowId(2));
    monitor.available_rect = Rect::new(10, 20, 300, 100);
    monitor.monitor_rect = monitor.available_rect;
    monitor
        .client_mut(WindowId(2))
        .unwrap()
        .size_hints
        .min_width = 160;
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &windows, 1);

    let plan = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    let rects = plan
        .client_moves
        .iter()
        .map(|output| (output.win, output.rect))
        .collect::<HashMap<_, _>>();

    assert!(rects[&WindowId(2)].w >= 160);
    for rect in rects.values() {
        assert!(monitor.available_rect.contains_rect(rect));
    }
    for (index, first) in rects.values().enumerate() {
        for second in rects.values().skip(index + 1) {
            assert!(
                !first.intersects_other(second),
                "tiled slots must not overlap: {first:?} {second:?}"
            );
        }
    }
}

#[test]
fn arrange_softens_impossible_minimums_and_restores_them_when_space_returns() {
    let windows = [WindowId(1), WindowId(2)];
    let mut monitor = monitor_with_order(&windows, WindowId(1));
    monitor.available_rect = Rect::new(0, 0, 300, 100);
    monitor.monitor_rect = monitor.available_rect;
    for win in windows {
        let client = monitor.client_mut(win).unwrap();
        client.size_hints.min_width = 200;
        client.size_hints.min_height = 50;
    }
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &windows, 1);

    let overcommitted = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    let overcommitted_rects = overcommitted
        .client_moves
        .iter()
        .map(|output| (output.win, output.rect))
        .collect::<HashMap<_, _>>();
    assert!(overcommitted_rects.values().all(|rect| rect.w < 200));
    assert_eq!(
        overcommitted_rects[&WindowId(1)].right(),
        overcommitted_rects[&WindowId(2)].x
    );
    assert!(
        overcommitted
            .client_moves
            .iter()
            .all(|output| { output.options.size_hints == crate::geometry::SizeHintPolicy::Ignore })
    );

    monitor.available_rect = Rect::new(0, 0, 500, 100);
    monitor.monitor_rect = monitor.available_rect;
    let recovered = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    assert!(recovered.client_moves.iter().all(|output| {
        output.rect.w >= 200
            && output.options.size_hints == crate::geometry::SizeHintPolicy::Respect
    }));
}

#[test]
fn dense_manual_layout_uses_one_animation_duration_for_every_window() {
    let windows = (1..=12).map(WindowId).collect::<Vec<_>>();
    let mut monitor = monitor_with_order(&windows, windows[0]);
    monitor.available_rect = Rect::new(0, 0, 1200, 700);
    monitor.monitor_rect = monitor.available_rect;
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);

    let plan = monitor.compute_arrange(&LayoutConfig::default(), true, true);

    assert_eq!(plan.client_moves.len(), windows.len());
    assert!(plan.client_moves.iter().all(|output| {
        output.options.mode == crate::geometry::MoveResizeMode::AnimateTo
            && output.options.duration
                == std::time::Duration::from_millis(
                    crate::constants::animation::DEFAULT_ANIMATION_MILLIS,
                )
    }));
}

#[test]
fn overview_treats_true_fullscreen_as_an_ordinary_card() {
    let tags = TagMask::single(1).unwrap();
    let win = WindowId(1);
    let original = Rect::new(0, 0, 1200, 800);
    let mut monitor = Monitor {
        monitor_rect: original,
        available_rect: original,
        overview_state: Some(crate::overview::OverviewState::new(
            tags,
            vec![win],
            HashMap::from([(win, original)]),
            Some(win),
        )),
        ..Monitor::default()
    };
    monitor.set_selected_tags(tags);
    monitor.adopt_client(
        Client {
            win,
            tags,
            geo: original,
            mode: ClientMode::tiled().as_fullscreen(),
            ..Client::default()
        },
        false,
    );

    let plan = monitor.compute_arrange(&LayoutConfig::default(), true, false);

    assert_eq!(plan.client_moves.len(), 1);
    assert!(plan.fullscreen_moves.is_empty());
    assert_eq!(plan.z_order, Some(vec![win]));
}

#[test]
fn fullscreen_preserves_a_tiled_clients_tree_slot() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let fullscreen_win = windows[1];
    let mut monitor = monitor_with_order(&windows, fullscreen_win);
    monitor.monitor_rect = Rect::new(0, 0, 1200, 800);
    monitor.available_rect = monitor.monitor_rect;
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);

    let before = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    let before_rect = before
        .client_moves
        .iter()
        .find(|output| output.win == fullscreen_win)
        .unwrap()
        .rect;
    let leaves_before = monitor.per_tag_state().layout_tree.leaves();

    monitor
        .client_mut(fullscreen_win)
        .unwrap()
        .enter_fullscreen();
    let fullscreen = monitor.compute_arrange(&LayoutConfig::default(), true, false);

    assert_eq!(monitor.per_tag_state().layout_tree.leaves(), leaves_before);
    assert!(
        fullscreen
            .client_moves
            .iter()
            .all(|output| output.win != fullscreen_win)
    );
    assert!(
        fullscreen
            .fullscreen_moves
            .iter()
            .any(|output| output.win == fullscreen_win)
    );

    monitor.client_mut(fullscreen_win).unwrap().restore_mode();
    let restored = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    let restored_rect = restored
        .client_moves
        .iter()
        .find(|output| output.win == fullscreen_win)
        .unwrap()
        .rect;

    assert_eq!(restored_rect, before_rect);
    assert_eq!(monitor.per_tag_state().layout_tree.leaves(), leaves_before);
}

#[test]
fn maximized_presentation_overlaps_tiled_clients_without_rewriting_tree() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let mut monitor = monitor_with_order(&windows, WindowId(3));
    monitor.available_rect = Rect::new(0, 0, 400, 300);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);
    let tree_before = monitor
        .per_tag_state()
        .layout_tree
        .bounds(Rect::new(0, 0, 400, 300));
    monitor.per_tag_state().presentation = PresentationMode::Maximized;

    let maximized = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    assert_eq!(maximized.client_moves.len(), windows.len());
    assert!(
        maximized
            .client_moves
            .iter()
            .all(|output| output.rect == Rect::new(0, 0, 400, 300))
    );
    assert_eq!(
        monitor
            .per_tag_state()
            .layout_tree
            .bounds(Rect::new(0, 0, 400, 300)),
        tree_before
    );

    monitor.per_tag_state().presentation = PresentationMode::Tiled;
    let manual = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    let first_rect = manual.client_moves.first().unwrap().rect;
    assert!(
        manual
            .client_moves
            .iter()
            .skip(1)
            .any(|output| output.rect != first_rect)
    );
    assert_eq!(
        monitor
            .per_tag_state()
            .layout_tree
            .bounds(Rect::new(0, 0, 400, 300)),
        tree_before
    );
}

#[test]
fn maximized_presentation_reconciles_new_tiled_leaves() {
    let mut monitor = monitor_with_order(&[WindowId(1), WindowId(2)], WindowId(1));
    monitor.available_rect = Rect::new(0, 0, 300, 200);
    monitor.per_tag_state().presentation = PresentationMode::Maximized;
    let _ = monitor.compute_arrange(&LayoutConfig::default(), true, false);

    append_visible_client(&mut monitor, WindowId(3));
    let _ = monitor.compute_arrange(&LayoutConfig::default(), true, false);

    let leaves = monitor.per_tag_state().layout_tree.leaves();
    assert_eq!(leaves.len(), 3);
    assert!(leaves.contains(&WindowId(3)));
}

#[test]
fn floating_presentation_overlaps_tiled_clients_without_rewriting_tree() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let mut monitor = monitor_with_order(&windows, WindowId(2));
    monitor.available_rect = Rect::new(0, 0, 400, 300);
    monitor
        .client_mut(WindowId(3))
        .unwrap()
        .set_placement(ClientPlacement::Floating);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows[..2], 1);
    let tree_before = monitor
        .per_tag_state()
        .layout_tree
        .bounds(Rect::new(0, 0, 400, 300));
    monitor.per_tag_state().presentation = PresentationMode::Floating;

    let floating = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    assert!(floating.client_moves.is_empty());
    assert_eq!(
        monitor
            .per_tag_state()
            .layout_tree
            .bounds(Rect::new(0, 0, 400, 300)),
        tree_before
    );
    assert_eq!(
        monitor.client(WindowId(1)).unwrap().mode(),
        ClientMode::tiled()
    );
    assert_eq!(
        monitor.client(WindowId(3)).unwrap().mode(),
        ClientMode::floating()
    );

    monitor.per_tag_state().presentation = PresentationMode::Tiled;
    let manual = monitor.compute_arrange(&LayoutConfig::default(), true, false);
    let first_rect = manual.client_moves.first().unwrap().rect;
    assert!(
        manual
            .client_moves
            .iter()
            .skip(1)
            .any(|output| output.rect != first_rect)
    );
    assert_eq!(
        monitor
            .per_tag_state()
            .layout_tree
            .bounds(Rect::new(0, 0, 400, 300)),
        tree_before
    );
    assert_eq!(
        manual
            .client_moves
            .iter()
            .filter(|output| output.win == WindowId(3))
            .count(),
        0
    );
}

#[test]
fn projected_z_order_keeps_floating_above_tiled_and_fullscreen_above_floating() {
    let mut monitor = monitor_with_order(
        &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
        WindowId(2),
    );
    monitor
        .client_mut(WindowId(3))
        .unwrap()
        .set_placement(crate::types::ClientPlacement::Floating);
    monitor.client_mut(WindowId(4)).unwrap().enter_fullscreen();

    let projected = compute_monitor_z_order(&monitor).unwrap();

    assert_eq!(
        projected,
        vec![
            WindowId(1),
            WindowId(2),
            WindowId(99),
            WindowId(3),
            WindowId(4)
        ]
    );
}

#[test]
fn projected_z_order_keeps_last_tiled_focus_visible_under_floating_focus() {
    let mut monitor = monitor_with_order(&[WindowId(1), WindowId(2), WindowId(3)], WindowId(2));
    monitor.record_focus(monitor.selected_tags(), WindowId(1));
    monitor
        .client_mut(WindowId(2))
        .unwrap()
        .set_placement(crate::types::ClientPlacement::Floating);

    let projected = compute_monitor_z_order(&monitor).unwrap();

    assert_eq!(
        projected,
        vec![WindowId(3), WindowId(1), WindowId(99), WindowId(2)]
    );
    assert_eq!(
        monitor.z_order.iter_bottom_to_top().collect::<Vec<_>>(),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );
}

// ── Layout slot semantics ─────────────────────────────────────────────────────

use crate::layouts::LayoutCommand;

fn slotted_wm(windows: &[WindowId]) -> (crate::wm::Wm, crate::types::MonitorId) {
    let mut wm = wayland_wm();
    let monitor_id = add_tiled_monitor(&mut wm, windows, Rect::new(0, 0, 1200, 800));
    (wm, monitor_id)
}

fn slot_tree_bounds(
    wm: &crate::wm::Wm,
    monitor_id: crate::types::MonitorId,
) -> HashMap<WindowId, Rect> {
    let monitor = wm.core.model.monitor(monitor_id).unwrap();
    monitor
        .per_tag()
        .unwrap()
        .layout_tree
        .bounds(monitor.available_rect)
}

fn slot_presentation(wm: &crate::wm::Wm, monitor_id: crate::types::MonitorId) -> PresentationMode {
    wm.core.model.monitor(monitor_id).unwrap().current_layout()
}

#[test]
fn switching_layouts_back_and_forth_restores_manual_edits() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .layout_tree
            .resize(WindowId(1), Side::Right, keyboard_config())
    );
    let adjusted = slot_tree_bounds(&wm, monitor_id);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Tile);
    assert_ne!(slot_tree_bounds(&wm, monitor_id), adjusted);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert_eq!(slot_tree_bounds(&wm, monitor_id), adjusted);
}

#[test]
fn reactivating_the_visible_layout_resets_manual_edits() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    let stock = slot_tree_bounds(&wm, monitor_id);

    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .layout_tree
            .resize(WindowId(1), Side::Right, keyboard_config())
    );
    assert_ne!(slot_tree_bounds(&wm, monitor_id), stock);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert_eq!(slot_tree_bounds(&wm, monitor_id), stock);
}

#[test]
fn first_activation_applies_the_rule_to_the_current_tree() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    // A manual arrangement with a non-stack leaf order.
    {
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        let state = monitor.per_tag_state();
        state
            .layout_tree
            .apply_preset(Preset::MasterStack, &windows, 1);
        assert!(state.layout_tree.swap_windows(WindowId(2), WindowId(3)));
    }
    let manual_order = wm
        .core
        .model
        .monitor(monitor_id)
        .unwrap()
        .per_tag()
        .unwrap()
        .layout_tree
        .leaves();

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);

    let monitor = wm.core.model.monitor(monitor_id).unwrap();
    let state = monitor.per_tag().unwrap();
    assert_eq!(state.layout_tree.leaves(), manual_order);
    assert_eq!(state.active_preset, Preset::Grid);
    // Grid geometry, not master/stack: the first column is capped at two rows.
    let bounds = state.layout_tree.bounds(monitor.available_rect);
    assert_eq!(bounds[&WindowId(1)].h, 400);
    assert_eq!(bounds[&WindowId(1)].w, bounds[&WindowId(2)].w);
}

#[test]
fn layout_key_lifts_a_lens_without_resetting_the_slot() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .layout_tree
            .resize(WindowId(1), Side::Right, keyboard_config())
    );
    let adjusted = slot_tree_bounds(&wm, monitor_id);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Maximized);
    assert_eq!(
        slot_presentation(&wm, monitor_id),
        PresentationMode::Maximized
    );

    // Pressing the hidden layout's key reveals the remembered tree untouched.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert_eq!(slot_presentation(&wm, monitor_id), PresentationMode::Tiled);
    assert_eq!(slot_tree_bounds(&wm, monitor_id), adjusted);

    // Only the next press, with the layout visible tiled, resets it.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert_ne!(slot_tree_bounds(&wm, monitor_id), adjusted);
}

#[test]
fn never_activated_default_tree_seeds_instead_of_being_remembered() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    // The organically grown default tree is not a remembered tile slot.
    wm.core
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &windows, 1);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    {
        let state = wm
            .core
            .model
            .monitor(monitor_id)
            .unwrap()
            .per_tag()
            .unwrap();
        assert_eq!(state.active_preset, Preset::Grid);
        assert!(state.stored_trees.is_empty());
    }

    // Tile's first activation seeds from the grid tree and applies its rule.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Tile);
    let monitor = wm.core.model.monitor(monitor_id).unwrap();
    let state = monitor.per_tag().unwrap();
    assert_eq!(state.active_preset, Preset::MasterStack);
    assert!(state.stored_trees.contains_key(&Preset::Grid));
    let bounds = state.layout_tree.bounds(monitor.available_rect);
    assert_eq!(bounds[&WindowId(1)].h, 800);
}

#[test]
fn restored_slot_reconciles_windows_opened_and_closed_while_inactive() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Tile);

    // Close one window and open another while the grid slot is inactive.
    let tags = TagMask::single(1).unwrap();
    {
        let mut ctx = wm.ctx();
        assert!(
            ctx.core_mut()
                .model_mut()
                .remove_client(WindowId(4))
                .is_some()
        );
        assert!(ctx.core_mut().model_mut().add_client(
            monitor_id,
            Client {
                win: WindowId(5),
                tags,
                mode: ClientMode::tiled(),
                ..Client::default()
            }
        ));
    }

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);

    let leaves = wm
        .core
        .model
        .monitor(monitor_id)
        .unwrap()
        .per_tag()
        .unwrap()
        .layout_tree
        .leaves();
    // Membership must match the visible set exactly; where the insertion
    // policy places the newcomer is its own decision, not the slot's.
    let mut membership = leaves.clone();
    membership.sort_by_key(|win| win.0);
    assert_eq!(
        membership,
        vec![WindowId(1), WindowId(2), WindowId(3), WindowId(5)]
    );
}

#[test]
fn maximized_reorder_edits_the_active_slot() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Maximized);
    assert_eq!(
        crate::layouts::reorder_maximized_stack(&mut wm.ctx(), crate::types::StackDirection::Next),
        crate::layouts::MaximizedStackReorder::Reordered
    );
    let reordered = wm
        .core
        .model
        .monitor(monitor_id)
        .unwrap()
        .per_tag()
        .unwrap()
        .layout_tree
        .leaves();
    assert_eq!(reordered, vec![WindowId(2), WindowId(1), WindowId(3)]);

    // The order edit belongs to the grid slot and survives a slot round trip.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Tile);
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert_eq!(
        wm.core
            .model
            .monitor(monitor_id)
            .unwrap()
            .per_tag()
            .unwrap()
            .layout_tree
            .leaves(),
        reordered
    );
}

#[test]
fn cycling_a_full_lap_restores_the_starting_layout_without_resetting_it() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .layout_tree
            .resize(WindowId(1), Side::Right, keyboard_config())
    );
    let adjusted = slot_tree_bounds(&wm, monitor_id);

    // One step per cycle entry, lenses included: a complete lap.
    for _ in 0..LayoutCommand::all().len() {
        crate::layouts::cycle_layout_direction(&mut wm.ctx(), true);
    }

    let state = wm
        .core
        .model
        .monitor(monitor_id)
        .unwrap()
        .per_tag()
        .unwrap();
    assert_eq!(state.active_preset, Preset::Grid);
    assert_eq!(slot_tree_bounds(&wm, monitor_id), adjusted);
}

#[test]
fn cycling_visits_lenses_and_never_lands_on_the_current_state() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    let active_preset = |wm: &crate::wm::Wm| {
        wm.core
            .model
            .monitor(monitor_id)
            .unwrap()
            .per_tag()
            .unwrap()
            .active_preset
    };

    // Tile → Grid: a plain slot switch.
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), true);
    assert_eq!(active_preset(&wm), Preset::Grid);
    assert_eq!(slot_presentation(&wm, monitor_id), PresentationMode::Tiled);

    // Grid → Floating → Maximized: the lenses over the grid slot.
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), true);
    assert_eq!(
        slot_presentation(&wm, monitor_id),
        PresentationMode::Floating
    );
    assert_eq!(active_preset(&wm), Preset::Grid);
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), true);
    assert_eq!(
        slot_presentation(&wm, monitor_id),
        PresentationMode::Maximized
    );

    // Maximized → BottomStack: cycling off a lens lands on the next slot.
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), true);
    assert_eq!(slot_presentation(&wm, monitor_id), PresentationMode::Tiled);
    assert_eq!(active_preset(&wm), Preset::BottomStack);

    // Floating over the grid slot must step to maximized, never re-land on
    // floating itself: a press that sometimes does nothing feels broken.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Floating);
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), true);
    assert_eq!(
        slot_presentation(&wm, monitor_id),
        PresentationMode::Maximized
    );

    // Stepping backward off floating reveals the underlying slot instead.
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), false);
    assert_eq!(
        slot_presentation(&wm, monitor_id),
        PresentationMode::Floating
    );
    crate::layouts::cycle_layout_direction(&mut wm.ctx(), false);
    assert_eq!(slot_presentation(&wm, monitor_id), PresentationMode::Tiled);
    assert_eq!(active_preset(&wm), Preset::Grid);
}

#[test]
fn reset_active_layout_returns_stock_geometry_and_drops_a_lens() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let (mut wm, monitor_id) = slotted_wm(&windows);

    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Grid);
    let stock = slot_tree_bounds(&wm, monitor_id);
    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .layout_tree
            .resize(WindowId(1), Side::Right, keyboard_config())
    );

    // Hidden behind a lens, the reset still targets the active slot and
    // lifts the lens so the stock layout is what the user sees.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Maximized);
    crate::layouts::reset_active_layout(&mut wm.ctx());
    assert_eq!(slot_presentation(&wm, monitor_id), PresentationMode::Tiled);
    assert_eq!(slot_tree_bounds(&wm, monitor_id), stock);
}

#[test]
fn arrange_does_not_overwrite_a_scaled_monitor_bar_height() {
    // Regression: `arrange` used to read the unscaled bar height and write it back onto the monitor, undoing the
    // per-output scaling applied by the monitor-sync path. On a 2x output that
    // left a 1x-tall bar alongside 2x padding and start-menu width.
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    wm.core.config.animations.enabled = false;

    let win = WindowId(1);
    let monitor_id = add_tiled_monitor(&mut wm, &[win], Rect::new(0, 0, 1600, 1200));
    wm.core
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .set_ui_metrics(
            2.0,
            MonitorUiMetrics {
                bar_height: 60,
                horizontal_padding: 30,
                startmenu_size: 60,
            },
        );

    super::arrange(&mut wm.ctx(), Some(monitor_id));

    let monitor = wm.core.model.monitor(monitor_id).unwrap();
    assert_eq!(
        monitor.bar_height, 60,
        "arrange must not clobber the scaled bar height with the unscaled global"
    );
    assert_eq!(monitor.horizontal_padding, 30);
    assert_eq!(monitor.startmenu_size, 60);
}
