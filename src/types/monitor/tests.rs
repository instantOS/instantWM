use super::*;

/// Give `monitor` ownership of `clients`, with `order` as its focus stack.
fn adopt_clients(monitor: &mut Monitor, order: &[WindowId], clients: Vec<Client>) {
    for client in clients {
        monitor.adopt_client(client, false);
    }
    if !order.is_empty() {
        assert!(monitor.set_focus_order(order.to_vec()));
    }
}

#[test]
fn focus_order_rejects_missing_duplicate_and_foreign_windows() {
    let mut monitor = Monitor::default();
    monitor.adopt_client(Client::new(WindowId(1)), false);
    monitor.adopt_client(Client::new(WindowId(2)), false);
    let original = monitor.stack.to_vec();

    assert!(!monitor.set_focus_order(vec![WindowId(1)]));
    assert!(!monitor.set_focus_order(vec![WindowId(1), WindowId(1)]));
    assert!(!monitor.set_focus_order(vec![WindowId(1), WindowId(3)]));
    assert_eq!(monitor.stack.as_slice(), original);
    assert!(monitor.set_focus_order(vec![WindowId(1), WindowId(2)]));
}

#[test]
fn focus_history_is_deduplicated_mru_and_supports_filtered_recovery() {
    let tags = TagMask::single(1).unwrap();
    let first = WindowId(1);
    let second = WindowId(2);
    let mut monitor = Monitor::default();

    monitor.record_focus(tags, first);
    monitor.record_focus(tags, second);
    monitor.record_focus(tags, first);

    assert_eq!(monitor.most_recent_focus(tags, |_| true), Some(first));
    assert_eq!(
        monitor.most_recent_focus(tags, |win| win != first),
        Some(second)
    );
    assert_eq!(
        monitor.focus_history_windows().collect::<Vec<_>>(),
        vec![second, first]
    );

    monitor.forget_focus(first);
    assert_eq!(monitor.most_recent_focus(tags, |_| true), Some(second));
    monitor.forget_focus(second);
    assert_eq!(monitor.most_recent_focus(tags, |_| true), None);
    assert!(monitor.focus_history_windows().next().is_none());
}

#[test]
fn first_visible_client_prefers_topmost_visible_stack_entry() {
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(TagMask::single(1).unwrap());
    let clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| {
            let mut client = Client {
                win,
                ..Client::default()
            };
            client.set_tag_mask(TagMask::single(1).unwrap());
            client
        })
        .collect();
    adopt_clients(&mut monitor, &[], clients);

    assert_eq!(monitor.first_visible_client(), Some(WindowId(3)));
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
    let lookup = Rect::new(200, 60, 1, 1);
    assert_eq!(
        monitors.monitor_intersecting_rect(lookup).map(|m| m.id()),
        Some(id)
    );
    assert_eq!(monitors.monitor_by_rect(lookup).map(|m| m.id()), Some(id));
    assert_eq!(
        monitors
            .monitor_at_pointer(Point::new(1_000, 1_000))
            .map(|monitor| monitor.id()),
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
        monitors
            .monitor_intersecting_rect(Rect::new(150, 5, 1, 1))
            .map(|m| m.id()),
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
        bar_default_show: true,
        ..Monitor::default()
    };
    monitor.set_selected_tags(tags);

    assert_eq!(monitor.visible_content_rect(), Rect::new(100, 80, 800, 570));

    let mut fullscreen = Client {
        win: WindowId(1),
        mode: crate::types::ClientMode::tiled().as_fullscreen(),
        ..Client::default()
    };
    fullscreen.set_tag_mask(tags);
    let win = fullscreen.win;
    adopt_clients(&mut monitor, &[win], vec![fullscreen]);

    assert_eq!(monitor.visible_content_rect(), monitor.available_rect);
}

#[test]
fn visible_content_rect_preserves_external_exclusive_area() {
    let monitor = Monitor {
        monitor_rect: Rect::new(100, 50, 800, 600),
        available_rect: Rect::new(100, 90, 800, 560),
        bar_height: 30,
        bar_default_show: true,
        ..Monitor::default()
    };

    assert_eq!(monitor.visible_content_rect(), monitor.available_rect);
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
    let mut monitor = Monitor {
        tags: vec![Tag::default(); 3],
        ..Monitor::default()
    };

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
        mode: crate::types::ClientMode::floating(),
        ..Client::default()
    };
    floating.set_tag_mask(TagMask::single(1).unwrap());

    let mut hidden = Client {
        win: WindowId(4),
        is_hidden: true,
        ..Client::default()
    };
    hidden.set_tag_mask(TagMask::single(1).unwrap());

    adopt_clients(
        &mut monitor,
        &[WindowId(1), WindowId(2), WindowId(3), WindowId(4)],
        vec![normal, fullscreen, floating, hidden],
    );

    assert_eq!(monitor.tiled_client_count(), 1);
    assert_eq!(monitor.collect_tiled().len(), 1);
}

#[test]
fn maximized_focus_cycle_uses_tree_order_and_excludes_floating_clients() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[WindowId(3), WindowId(1), WindowId(2)],
        1,
    );
    monitor.per_tag_state().presentation = PresentationMode::Maximized;
    let clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| {
            let mut client = Client {
                win,
                tags: tag,
                ..Client::default()
            };
            if win == WindowId(2) {
                client.set_placement(crate::types::ClientPlacement::Floating);
            }
            client
        })
        .collect();
    adopt_clients(
        &mut monitor,
        &[WindowId(1), WindowId(2), WindowId(3)],
        clients,
    );

    let cycle_order = monitor.focus_cycle_order();
    let bar_order = monitor.bar_client_order();
    assert_eq!(cycle_order, vec![WindowId(3), WindowId(1)]);
    assert_eq!(&bar_order[..cycle_order.len()], cycle_order);
}

#[test]
fn focus_cycle_skips_minimized_tree_positions() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[WindowId(1), WindowId(2), WindowId(3)],
        1,
    );
    monitor.per_tag_state().presentation = PresentationMode::Maximized;
    let clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| {
            let mut client = Client {
                win,
                tags: tag,
                ..Client::default()
            };
            if win == WindowId(2) {
                client.is_hidden = true;
            }
            client
        })
        .collect();
    adopt_clients(
        &mut monitor,
        &[WindowId(1), WindowId(2), WindowId(3)],
        clients,
    );

    // The minimized entry keeps its title position but cannot receive focus.
    assert_eq!(
        monitor.bar_client_order(),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );
    assert_eq!(monitor.focus_cycle_order(), vec![WindowId(1), WindowId(3)]);
}

#[test]
fn maximized_focus_cycle_falls_back_to_bar_order_when_no_tile_is_focusable() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[WindowId(1), WindowId(2)],
        1,
    );
    monitor.per_tag_state().presentation = PresentationMode::Maximized;
    let clients = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| {
            let mut client = Client {
                win,
                tags: tag,
                ..Client::default()
            };
            match win {
                // Every tiled client is minimized.
                WindowId(1) | WindowId(2) => client.is_hidden = true,
                // The only focusable client is a floating overlay.
                _ => client.set_placement(crate::types::ClientPlacement::Floating),
            }
            client
        })
        .collect();
    adopt_clients(
        &mut monitor,
        &[WindowId(1), WindowId(2), WindowId(3)],
        clients,
    );

    // With no focusable tile the tree order would be empty, so floating
    // clients must stay cyclable.
    assert_eq!(monitor.tiled_tree_order(), vec![WindowId(1), WindowId(2)]);
    assert_eq!(monitor.focus_cycle_order(), vec![WindowId(3)]);
}

#[test]
fn maximized_bar_titles_put_the_keyboard_cycle_order_first() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
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
                client.set_placement(crate::types::ClientPlacement::Floating);
            }
            client
        })
        .collect();
    adopt_clients(
        &mut monitor,
        &[WindowId(2), WindowId(3), WindowId(1), WindowId(4)],
        clients,
    );

    assert_eq!(monitor.tiled_tree_order(), vec![WindowId(3), WindowId(1)]);
    assert_eq!(
        monitor.bar_client_order(),
        vec![WindowId(3), WindowId(1), WindowId(2), WindowId(4)]
    );
}

#[test]
fn minimized_tiled_titles_keep_their_position_in_maximized_presentation() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[WindowId(1), WindowId(2), WindowId(3)],
        1,
    );
    monitor.per_tag_state().presentation = PresentationMode::Maximized;

    let mut clients: Vec<Client> = [WindowId(1), WindowId(2), WindowId(3)]
        .into_iter()
        .map(|win| Client {
            win,
            tags: tag,
            ..Client::default()
        })
        .collect();
    clients
        .iter_mut()
        .find(|client| client.win == WindowId(2))
        .unwrap()
        .is_hidden = true;
    adopt_clients(
        &mut monitor,
        &[WindowId(1), WindowId(2), WindowId(3)],
        clients,
    );

    // Order role: minimizing via the bar must not move the title.
    assert_eq!(
        monitor.tiled_tree_order(),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );
    assert_eq!(
        monitor.bar_client_order(),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );

    // The maximized arrange path retains the leaf for order maintenance.
    let order_members = monitor.collect_tree_order_members();
    let windows: Vec<_> = order_members.iter().map(|client| client.win).collect();
    monitor.per_tag_state().layout_tree.reconcile_for_layout(
        &windows,
        crate::config::config_toml::NewWindowPlacement::default(),
        Rect::new(0, 0, 800, 600),
        &HashMap::new(),
    );
    assert_eq!(
        monitor.per_tag().unwrap().layout_tree.leaves(),
        vec![WindowId(1), WindowId(2), WindowId(3)],
        "minimized client must keep its tree leaf"
    );

    // Geometry role stays visibility-filtered: the minimized client claims no
    // tiling space.
    assert_eq!(monitor.collect_tree_order_members().len(), 3);
    assert_eq!(monitor.collect_tiling_tree_members().len(), 2);
    assert_eq!(monitor.collect_tiled().len(), 2);
}
