use super::{
    available_tree_resize_direction, clients_with_planned_borders, compute_monitor_z_order,
    manual_tree_pointer_interaction_allowed, pointer_tree_resize_allowed, shifted_master_count,
};
use crate::config::config_toml::LayoutConfig;
use crate::layouts::PresentationMode;
use crate::layouts::tree::{Preset, Side};
use crate::types::{
    BaseClientMode, Client, ClientMode, Monitor, Point, Rect, ResizeDirection, Size, TagMask,
    WindowId,
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

#[test]
fn master_count_is_bounded_by_the_current_tiled_window_count() {
    assert_eq!(shifted_master_count(1, -1, 4), 0);
    assert_eq!(shifted_master_count(0, -1, 4), 0);
    assert_eq!(shifted_master_count(3, 1, 4), 4);
    assert_eq!(shifted_master_count(4, 1, 4), 4);
    assert_eq!(shifted_master_count(8, -1, 3), 2);
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
            client.replace_mode_with_base(BaseClientMode::Floating);
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
            client.replace_mode_with_base(BaseClientMode::Floating);
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
    clients.get_mut(&WindowId(2)).unwrap().size_hints.minw = 160;
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
            && output.options.frames == crate::constants::animation::DEFAULT_FRAME_COUNT
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
            mode: ClientMode::TrueFullscreen {
                restore: BaseClientMode::Tiling,
            },
            ..Client::default()
        },
    )]);

    let plan = monitor.compute_arrange(&clients, &LayoutConfig::default(), true, 0, false);

    assert_eq!(plan.client_moves.len(), 1);
    assert!(plan.fullscreen_moves.is_empty());
    assert_eq!(plan.z_order, Some(vec![win]));
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
        .replace_mode_with_base(crate::types::BaseClientMode::Floating);
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
    monitor
        .tag_tiled_focus_history
        .insert(monitor.selected_tags(), WindowId(1));
    let mut clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| (win, visible_client(win)))
        .collect::<HashMap<_, _>>();
    clients
        .get_mut(&WindowId(2))
        .unwrap()
        .replace_mode_with_base(crate::types::BaseClientMode::Floating);

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
