use super::*;
#[test]
fn first_visible_client_prefers_topmost_visible_stack_entry() {
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(TagMask::single(1).unwrap());
    monitor.z_order.attach_top(WindowId(1));
    monitor.z_order.attach_top(WindowId(2));
    monitor.z_order.attach_top(WindowId(3));

    let mut clients = HashMap::new();
    for id in [WindowId(1), WindowId(2), WindowId(3)] {
        let mut client = Client {
            win: id,
            ..Client::default()
        };
        client.set_tag_mask(TagMask::single(1).unwrap());
        clients.insert(id, client);
    }

    assert_eq!(monitor.first_visible_client(&clients), Some(WindowId(3)));
}

#[test]
fn client_z_order_raise_moves_existing_client_to_top() {
    let mut z_order = ClientZOrder::default();
    z_order.attach_top(WindowId(1));
    z_order.attach_top(WindowId(2));
    z_order.attach_top(WindowId(3));

    assert!(z_order.raise(WindowId(2)));
    assert_eq!(
        z_order.iter_bottom_to_top().collect::<Vec<_>>(),
        vec![WindowId(1), WindowId(3), WindowId(2)]
    );
}

#[test]
fn client_z_order_raise_ignores_unknown_client() {
    let mut z_order = ClientZOrder::default();
    z_order.attach_top(WindowId(1));
    z_order.attach_top(WindowId(2));
    z_order.attach_top(WindowId(3));

    assert!(!z_order.raise(WindowId(4)));
    assert_eq!(
        z_order.iter_bottom_to_top().collect::<Vec<_>>(),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );
}

#[test]
fn per_tag_state_defaults_match_normal_tiling_defaults() {
    let state = PerTagState::default();

    assert_eq!(state.master_count, 1);
}

#[test]
fn monitor_lookup_includes_bar_outside_work_area() {
    let monitor = Monitor {
        monitor_rect: Rect::new(100, 50, 800, 600),
        available_rect: Rect::new(100, 50, 800, 600),
        bar_height: 30,
        ..Monitor::default()
    };

    let mut monitors = crate::monitor::MonitorManager::new();
    let id = monitors.push(monitor);
    assert_eq!(
        monitors.id_intersecting_rect(Rect::new(200, 60, 1, 1)),
        Some(id)
    );
}

#[test]
fn local_work_point_accounts_for_monitor_origin_and_reserved_space() {
    let monitor = Monitor {
        monitor_rect: Rect::new(100, 50, 800, 600),
        available_rect: Rect::new(120, 80, 760, 550),
        ..Monitor::default()
    };

    assert_eq!(
        monitor.local_work_point(Point::new(145, 105)),
        Point::new(25, 25)
    );
}

#[test]
fn monitor_lookup_returns_stable_id_for_each_full_output() {
    let left = Monitor {
        monitor_rect: Rect::new(0, 0, 100, 100),
        available_rect: Rect::new(0, 0, 100, 100),
        bar_height: 20,
        ..Monitor::default()
    };
    let right = Monitor {
        monitor_rect: Rect::new(100, 0, 100, 100),
        available_rect: Rect::new(100, 0, 100, 100),
        bar_height: 20,
        ..Monitor::default()
    };

    let mut monitors = crate::monitor::MonitorManager::new();
    monitors.push(left);
    let right_id = monitors.push(right);
    assert_eq!(
        monitors.id_intersecting_rect(Rect::new(150, 5, 1, 1)),
        Some(right_id)
    );
}

#[test]
fn visible_content_rect_tracks_bar_edge_and_fullscreen_visibility() {
    let tags = TagMask::single(1).unwrap();
    let mut monitor = Monitor {
        monitor_rect: Rect::new(100, 50, 800, 600),
        available_rect: Rect::new(100, 50, 800, 600),
        bar_height: 30,
        show_bar: true,
        bar_position: EdgeDirection::Top,
        ..Monitor::default()
    };
    monitor.set_selected_tags(tags);
    let mut clients = HashMap::new();

    assert_eq!(
        monitor.visible_content_rect(&clients),
        Rect::new(100, 80, 800, 570)
    );

    monitor.bar_position = EdgeDirection::Bottom;
    assert_eq!(
        monitor.visible_content_rect(&clients),
        Rect::new(100, 50, 800, 570)
    );

    let mut fullscreen = Client {
        win: WindowId(1),
        mode: crate::types::ClientMode::Tiling.as_fullscreen(),
        ..Client::default()
    };
    fullscreen.set_tag_mask(tags);
    monitor.clients.push(fullscreen.win);
    clients.insert(fullscreen.win, fullscreen);

    assert_eq!(
        monitor.visible_content_rect(&clients),
        monitor.available_rect
    );
}

#[test]
fn visible_content_rect_preserves_external_exclusive_area() {
    let monitor = Monitor {
        monitor_rect: Rect::new(100, 50, 800, 600),
        available_rect: Rect::new(100, 90, 800, 560),
        bar_height: 30,
        show_bar: true,
        bar_position: EdgeDirection::Top,
        ..Monitor::default()
    };

    assert_eq!(
        monitor.visible_content_rect(&HashMap::new()),
        monitor.available_rect
    );
}

#[test]
fn current_tag_index_is_derived_from_selected_tags() {
    let mut monitor = Monitor::default();

    monitor.set_selected_tags(TagMask::single(3).unwrap());
    assert_eq!(monitor.current_tag_number(), Some(3));

    monitor.set_selected_tags(
        TagMask::single(2).unwrap_or(TagMask::EMPTY) | TagMask::single(3).unwrap_or(TagMask::EMPTY),
    );
    assert_eq!(monitor.current_tag_number(), None);

    monitor.set_selected_tags(TagMask::EMPTY);
    assert_eq!(monitor.current_tag_number(), None);
}

#[test]
fn all_tags_view_is_derived_from_selected_mask() {
    let mut monitor = Monitor::default();
    monitor.tags = vec![TagNames::default(); 3];

    monitor.set_selected_tags(TagMask::all(3));
    assert!(monitor.is_all_tags_view());

    monitor.set_selected_tags(TagMask::single(1).unwrap());
    assert!(!monitor.is_all_tags_view());

    monitor.set_selected_tags(TagMask::single(1).unwrap() | TagMask::single(2).unwrap());
    assert!(!monitor.is_all_tags_view());
}

#[test]
fn tiled_client_count_matches_collected_tiled_clients() {
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(TagMask::single(1).unwrap());
    monitor.clients = vec![WindowId(1), WindowId(2), WindowId(3), WindowId(4)];

    let mut normal = Client {
        win: WindowId(1),
        ..Client::default()
    };
    normal.set_tag_mask(TagMask::single(1).unwrap());

    let mut fullscreen = Client {
        win: WindowId(2),
        ..Client::default()
    };
    fullscreen.enter_fullscreen();
    fullscreen.set_tag_mask(TagMask::single(1).unwrap());

    let mut floating = Client {
        win: WindowId(3),
        mode: crate::types::ClientMode::Floating,
        ..Client::default()
    };
    floating.set_tag_mask(TagMask::single(1).unwrap());

    let mut hidden = Client {
        win: WindowId(4),
        is_hidden: true,
        ..Client::default()
    };
    hidden.set_tag_mask(TagMask::single(1).unwrap());

    let clients = HashMap::from([
        (WindowId(1), normal),
        (WindowId(2), fullscreen),
        (WindowId(3), floating),
        (WindowId(4), hidden),
    ]);

    assert_eq!(monitor.tiled_client_count(&clients), 1);
    assert_eq!(monitor.collect_tiled(&clients).len(), 1);
}

#[test]
fn maximized_bar_titles_put_the_keyboard_cycle_order_first() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
    monitor.clients = vec![WindowId(2), WindowId(3), WindowId(1), WindowId(4)];
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[WindowId(3), WindowId(1), WindowId(2)],
        1,
    );
    monitor.per_tag_state().presentation = PresentationMode::Maximized;

    let clients = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)]
        .into_iter()
        .map(|win| {
            let mut client = Client {
                win,
                tags: tag,
                ..Client::default()
            };
            if matches!(win, WindowId(2) | WindowId(4)) {
                client.replace_mode_with_base(crate::types::BaseClientMode::Floating);
            }
            (win, client)
        })
        .collect::<HashMap<_, _>>();

    assert_eq!(
        monitor.tiled_tree_order(&clients),
        vec![WindowId(3), WindowId(1)]
    );
    assert_eq!(
        monitor.bar_client_order(&clients),
        vec![WindowId(3), WindowId(1), WindowId(2), WindowId(4)]
    );
}
