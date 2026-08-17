use super::{
    available_tree_resize_direction, clients_with_planned_borders, compute_monitor_z_order,
    manual_tree_pointer_interaction_allowed, pointer_tree_resize_allowed, shifted_master_count,
};
use crate::config::config_toml::LayoutConfig;
use crate::layouts::PresentationMode;
use crate::layouts::tree::{Preset, Side};
use crate::types::{
    Client, ClientMode, ClientPlacement, InteractionSource, Monitor, MouseButton, Point, Rect,
    ResizeDirection, Size, TagMask, WindowId,
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

fn add_tiled_monitor(
    wm: &mut crate::wm::Wm,
    win: WindowId,
    monitor_rect: Rect,
) -> crate::types::MonitorId {
    let tags = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect,
        available_rect: monitor_rect,
        show_bar: false,
        ..Monitor::default()
    });
    assert!(wm.core.model.insert_client(Client {
        win,
        monitor_id,
        tags,
        mode: ClientMode::tiled(),
        ..Client::default()
    }));
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tags);
    monitor.clients.push(win);
    monitor.selected = Some(win);
    monitor_id
}

#[test]
fn monitor_arrange_consumes_only_its_pending_spawn_animations() {
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let first = WindowId(1);
    let second = WindowId(2);
    let first_monitor = add_tiled_monitor(&mut wm, first, Rect::new(0, 0, 800, 600));
    let second_monitor = add_tiled_monitor(&mut wm, second, Rect::new(800, 0, 800, 600));
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
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let live = WindowId(1);
    let destroyed = WindowId(2);
    let unrelated_monitor = add_tiled_monitor(&mut wm, live, Rect::new(800, 0, 800, 600));
    let arranged_monitor = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 800, 600),
        available_rect: Rect::new(0, 0, 800, 600),
        show_bar: false,
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
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let win = WindowId(1);
    let monitor_id = add_tiled_monitor(&mut wm, win, Rect::new(0, 0, 800, 600));
    wm.core.behavior.animated = false;
    wm.work.spawn_animations.insert(win);

    super::arrange(&mut wm.ctx(), Some(monitor_id));

    assert!(wm.work.spawn_animations.is_empty());
}

#[test]
fn arrange_invalidates_pointer_placement_candidates() {
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let tags = TagMask::single(1).unwrap();
    let source = WindowId(1);
    let target = WindowId(2);
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 400, 300),
        available_rect: Rect::new(0, 0, 400, 300),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);
    for win in [source, target] {
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::tiled(),
            ..Client::default()
        });
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tags);
    monitor.clients = vec![source, target];
    monitor.selected = Some(source);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &[source, target], 1);

    assert!(super::preview_tree_at_point(&mut wm.ctx(), source, Point::new(201, 150),).is_some());
    assert!(wm.core.pointer_placement_cache.is_some());

    super::arrange(&mut wm.ctx(), Some(monitor_id));
    assert!(wm.core.pointer_placement_cache.is_none());
}

#[test]
fn pointer_preview_and_release_share_the_normalized_candidate() {
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let tags = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 2000, 1000),
        available_rect: Rect::new(0, 0, 2000, 1000),
        show_bar: false,
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);
    let windows = (1..=20).map(WindowId).collect::<Vec<_>>();
    for &win in &windows {
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::tiled(),
            ..Client::default()
        });
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tags);
    monitor.clients = windows.clone();
    monitor.selected = Some(windows[0]);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);
    super::arrange(&mut wm.ctx(), Some(monitor_id));

    let source = windows[0];
    let point = Point::new(801, 625);
    let preview = super::preview_tree_at_point(&mut wm.ctx(), source, point)
        .expect("the test point must select a normalized edge candidate");

    assert!(super::place_tree_at_point(&mut wm.ctx(), source, point));
    let (placement, minimums) =
        super::selected_tiling_constraints(&wm.ctx()).expect("fixture has a selected monitor");
    let applied_slot = wm
        .core
        .model
        .expect_selected_monitor()
        .per_tag()
        .unwrap()
        .layout_tree
        .constrained_bounds(placement.work_rect(), &minimums)
        .unwrap()[&source];
    let applied_preview = crate::layouts::keyboard_placement::tree_slot_outer_rect(
        &wm.ctx(),
        source,
        placement,
        applied_slot,
    )
    .unwrap();
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
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let first = WindowId(1);
    let second = WindowId(2);
    let monitor_id = add_tiled_monitor(&mut wm, first, Rect::new(0, 0, 800, 600));
    let tags = TagMask::single(1).unwrap();
    assert!(wm.core.model.insert_client(Client {
        win: second,
        monitor_id,
        tags,
        mode: ClientMode::tiled(),
        ..Client::default()
    }));
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.clients.push(second);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &[first, second], 1);
    let origin = monitor.per_tag().unwrap().layout_tree.clone();

    wm.core
        .drag
        .begin_tree_resize(crate::core_state::TreeResizeParams {
            win: first,
            button: MouseButton::Right,
            source: InteractionSource::Pointer,
            direction: ResizeDirection::Right,
            start: Point::new(400, 300),
            geometry: Rect::new(0, 0, 400, 600),
            origin,
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

fn monitor_with_order(order: &[WindowId], selected: WindowId) -> Monitor {
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(TagMask::single(1).unwrap());
    monitor.selected = Some(selected);
    monitor.bar_win = WindowId(99);
    for &win in order {
        monitor.z_order.attach_top(win);
    }
    monitor
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
fn pointer_tree_resize_preserves_the_requested_floating_fallbacks() {
    assert!(!manual_tree_pointer_interaction_allowed(
        PresentationMode::Tiled,
        true,
        1,
    ));
    assert!(!manual_tree_pointer_interaction_allowed(
        PresentationMode::Maximized,
        true,
        3,
    ));
    assert!(manual_tree_pointer_interaction_allowed(
        PresentationMode::Tiled,
        true,
        3,
    ));

    assert!(!pointer_tree_resize_allowed(
        PresentationMode::Tiled,
        true,
        1,
        true,
        false,
    ));
    assert!(!pointer_tree_resize_allowed(
        PresentationMode::Maximized,
        true,
        3,
        true,
        true,
    ));
    assert!(!pointer_tree_resize_allowed(
        PresentationMode::Tiled,
        false,
        3,
        true,
        true,
    ));
    assert!(pointer_tree_resize_allowed(
        PresentationMode::Tiled,
        true,
        3,
        true,
        false,
    ));
}

#[test]
fn pointer_tree_resize_remains_active_when_client_minimums_are_impossible() {
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let tags = TagMask::single(1).unwrap();
    let windows = [WindowId(1), WindowId(2)];
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 300, 100),
        available_rect: Rect::new(0, 0, 300, 100),
        show_bar: false,
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);
    for win in windows {
        let mut client = Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::tiled(),
            ..Client::default()
        };
        client.size_hints.min_width = 200;
        client.size_hints.min_height = 50;
        wm.core.model.insert_client(client);
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tags);
    monitor.clients = windows.to_vec();
    monitor.selected = Some(windows[0]);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &windows, 1);
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
fn planned_border_is_used_without_waiting_for_next_arrange() {
    let win = WindowId(1);
    let mut client = visible_client(win);
    client.border_width = 2;
    let clients = HashMap::from([(win, client)]);

    let planned = clients_with_planned_borders(&clients, &[(win, 0)]);

    assert_eq!(planned[&win].border_width, 0);
    assert_eq!(clients[&win].border_width, 2);
}

#[test]
fn tiled_focus_does_not_mutate_or_project_a_different_persistent_order() {
    let monitor = monitor_with_order(&[WindowId(1), WindowId(2), WindowId(3)], WindowId(2));
    let clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| (win, visible_client(win)))
        .collect::<HashMap<_, _>>();

    let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

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
    let monitor = monitor_with_order(&[WindowId(1), WindowId(2), WindowId(3)], WindowId(2));
    let clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| {
            let mut client = visible_client(win);
            client.set_placement(ClientPlacement::Floating);
            (win, client)
        })
        .collect::<HashMap<_, _>>();

    let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

    assert_eq!(
        projected,
        vec![WindowId(99), WindowId(1), WindowId(2), WindowId(3)]
    );
}

#[test]
fn transient_dialogs_stay_above_ordinary_windows_and_nested_children() {
    let monitor = monitor_with_order(
        &[WindowId(1), WindowId(3), WindowId(4), WindowId(2)],
        WindowId(2),
    );
    let mut clients = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)]
        .into_iter()
        .map(|win| {
            let mut client = visible_client(win);
            client.set_placement(ClientPlacement::Floating);
            (win, client)
        })
        .collect::<HashMap<_, _>>();
    clients.get_mut(&WindowId(3)).unwrap().transient_for = Some(WindowId(1));
    clients.get_mut(&WindowId(4)).unwrap().transient_for = Some(WindowId(3));

    let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

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
    monitor.clients = vec![WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let clients = monitor
        .clients
        .iter()
        .copied()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    let windows = monitor.clients.clone();
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);

    let first = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
    assert!(
        monitor
            .per_tag_state()
            .layout_tree
            .resize(WindowId(1), Side::Right)
    );
    let second = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);

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
    monitor.clients = vec![WindowId(1)];
    let mut clients = HashMap::from([(WindowId(1), visible_client(WindowId(1)))]);
    let config = LayoutConfig::default();
    let _ = monitor.compute_arrange(&clients, &config, true, 0, false);

    monitor.clients.push(WindowId(2));
    monitor.z_order.attach_top(WindowId(2));
    clients.insert(WindowId(2), visible_client(WindowId(2)));
    let plan = monitor.compute_arrange(&clients, &config, true, 0, false);
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
    monitor.clients = windows.to_vec();
    let clients = windows
        .into_iter()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    let auto = LayoutConfig {
        new_window_placement: crate::config::config_toml::NewWindowPlacement::Auto,
        ..LayoutConfig::default()
    };
    let before = monitor.compute_arrange(&clients, &auto, true, 0, false);
    let force = LayoutConfig {
        new_window_placement: crate::config::config_toml::NewWindowPlacement::Force,
        ..auto
    };
    let after = monitor.compute_arrange(&clients, &force, true, 0, false);

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
    monitor.clients = windows.to_vec();
    let mut clients = windows
        .into_iter()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    clients.get_mut(&WindowId(2)).unwrap().size_hints.min_width = 160;
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &windows, 1);

    let plan = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
    let rects = plan
        .client_moves
        .iter()
        .map(|output| (output.win, output.rect))
        .collect::<HashMap<_, _>>();

    assert!(rects[&WindowId(2)].w >= 160);
    for rect in rects.values() {
        assert!(rect.x >= monitor.available_rect.x);
        assert!(rect.y >= monitor.available_rect.y);
        assert!(rect.x + rect.w <= monitor.available_rect.x + monitor.available_rect.w);
        assert!(rect.y + rect.h <= monitor.available_rect.y + monitor.available_rect.h);
    }
    for (index, first) in rects.values().enumerate() {
        for second in rects.values().skip(index + 1) {
            let overlaps = first.x < second.x + second.w
                && second.x < first.x + first.w
                && first.y < second.y + second.h
                && second.y < first.y + first.h;
            assert!(
                !overlaps,
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
    monitor.clients = windows.to_vec();
    let mut clients = windows
        .into_iter()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    for client in clients.values_mut() {
        client.size_hints.min_width = 200;
        client.size_hints.min_height = 50;
    }
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &windows, 1);

    let overcommitted = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
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
    let recovered = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
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
    monitor.clients = windows.clone();
    let clients = windows
        .iter()
        .copied()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);

    let plan = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, true);

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
        clients: vec![win],
        overview_state: Some(crate::overview::OverviewState::new(
            tags,
            vec![win],
            HashMap::from([(win, original)]),
            Some(win),
        )),
        ..Monitor::default()
    };
    monitor.set_selected_tags(tags);
    let clients = HashMap::from([(
        win,
        Client {
            win,
            tags,
            geo: original,
            mode: ClientMode::tiled().as_fullscreen(),
            ..Client::default()
        },
    )]);

    let plan = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);

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
    monitor.clients = windows.to_vec();
    let mut clients = windows
        .into_iter()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);

    let before = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
    let before_rect = before
        .client_moves
        .iter()
        .find(|output| output.win == fullscreen_win)
        .unwrap()
        .rect;
    let leaves_before = monitor.per_tag_state().layout_tree.leaves();

    clients.get_mut(&fullscreen_win).unwrap().enter_fullscreen();
    let fullscreen = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);

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

    clients.get_mut(&fullscreen_win).unwrap().restore_mode();
    let restored = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
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
    monitor.clients = windows.to_vec();
    let clients = windows
        .into_iter()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, &windows, 1);
    let tree_before = monitor
        .per_tag_state()
        .layout_tree
        .bounds(Rect::new(0, 0, 400, 300));
    monitor.per_tag_state().presentation = PresentationMode::Maximized;

    let maximized = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
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
    let manual = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
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
    monitor.clients = vec![WindowId(1), WindowId(2)];
    monitor.per_tag_state().presentation = PresentationMode::Maximized;
    let mut clients = monitor
        .clients
        .iter()
        .copied()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    let _ = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);

    monitor.clients.push(WindowId(3));
    monitor.z_order.attach_top(WindowId(3));
    clients.insert(WindowId(3), visible_client(WindowId(3)));
    let _ = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);

    let leaves = monitor.per_tag_state().layout_tree.leaves();
    assert_eq!(leaves.len(), 3);
    assert!(leaves.contains(&WindowId(3)));
}

#[test]
fn floating_presentation_overlaps_tiled_clients_without_rewriting_tree() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let mut monitor = monitor_with_order(&windows, WindowId(2));
    monitor.available_rect = Rect::new(0, 0, 400, 300);
    monitor.clients = windows.to_vec();
    let mut clients = windows
        .into_iter()
        .map(|window| (window, visible_client(window)))
        .collect::<HashMap<_, _>>();
    clients
        .get_mut(&WindowId(3))
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

    let floating = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
    assert!(floating.client_moves.is_empty());
    assert_eq!(
        monitor
            .per_tag_state()
            .layout_tree
            .bounds(Rect::new(0, 0, 400, 300)),
        tree_before
    );
    assert_eq!(
        clients.get(&WindowId(1)).unwrap().mode(),
        ClientMode::tiled()
    );
    assert_eq!(
        clients.get(&WindowId(3)).unwrap().mode(),
        ClientMode::floating()
    );

    monitor.per_tag_state().presentation = PresentationMode::Tiled;
    let manual = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);
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
    let monitor = monitor_with_order(
        &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
        WindowId(2),
    );
    let mut clients = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)]
        .into_iter()
        .map(|win| (win, visible_client(win)))
        .collect::<HashMap<_, _>>();
    clients
        .get_mut(&WindowId(3))
        .unwrap()
        .set_placement(crate::types::ClientPlacement::Floating);
    let fullscreen = clients.get_mut(&WindowId(4)).unwrap();
    fullscreen.enter_fullscreen();

    let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

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
    let mut clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| (win, visible_client(win)))
        .collect::<HashMap<_, _>>();
    clients
        .get_mut(&WindowId(2))
        .unwrap()
        .set_placement(crate::types::ClientPlacement::Floating);

    let projected = compute_monitor_z_order(&monitor, &clients).unwrap();

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
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    let tags = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1200, 800),
        available_rect: Rect::new(0, 0, 1200, 800),
        show_bar: false,
        ..Monitor::default()
    });
    for &win in windows {
        assert!(wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::tiled(),
            ..Client::default()
        }));
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tags);
    monitor.clients = windows.to_vec();
    monitor.selected = windows.first().copied();
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
            .resize(WindowId(1), Side::Right)
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
            .resize(WindowId(1), Side::Right)
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
            .resize(WindowId(1), Side::Right)
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
        assert!(ctx.core_mut().model_mut().insert_client(Client {
            win: WindowId(5),
            monitor_id,
            tags,
            mode: ClientMode::tiled(),
            ..Client::default()
        }));
    }
    {
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.clients.retain(|&win| win != WindowId(4));
        monitor.clients.push(WindowId(5));
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
            .resize(WindowId(1), Side::Right)
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
            .resize(WindowId(1), Side::Right)
    );

    // Hidden behind a lens, the reset still targets the active slot and
    // lifts the lens so the stock layout is what the user sees.
    crate::layouts::set_layout(&mut wm.ctx(), LayoutCommand::Maximized);
    crate::layouts::reset_active_layout(&mut wm.ctx());
    assert_eq!(slot_presentation(&wm, monitor_id), PresentationMode::Tiled);
    assert_eq!(slot_tree_bounds(&wm, monitor_id), stock);
}
