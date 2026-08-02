use super::*;

fn wm_with_overview_clients(
    selected_tags: TagMask,
    clients: &[(WindowId, TagMask)],
) -> crate::wm::Wm {
    let mut wm = crate::wm::Wm::new(crate::backend::Backend::new_wayland(
        crate::backend::wayland::WaylandBackend::new(),
    ));
    wm.core.model.tags.num_tags = clients
        .iter()
        .filter_map(|(_, tags)| tags.first_tag())
        .max()
        .unwrap_or(1);
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1200, 700),
        available_rect: Rect::new(0, 0, 1200, 700),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);
    for &(win, tags) in clients {
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            geo: Rect::new(100, 100, 700, 500),
            ..Client::default()
        });
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(selected_tags);
    monitor.clients = clients.iter().map(|(win, _)| *win).collect();
    for &(win, _) in clients {
        monitor.z_order.attach_top(win);
    }
    monitor.selected = clients.first().map(|(win, _)| *win);
    wm
}

#[test]
fn card_field_preserves_sizes_and_uses_both_axes() {
    let work = Rect::new(10, 20, 1200, 700);
    let sizes = vec![Size::new(900, 600); 12];

    let rects = card_field_rects(work, &sizes, 5);

    assert_eq!(rects.len(), sizes.len());
    for (rect, size) in rects.iter().zip(&sizes) {
        assert_eq!(rect.size(), *size);
        assert!(rect.x >= work.x);
        assert!(rect.y >= work.y);
    }
    assert!(
        rects
            .iter()
            .map(|rect| rect.x)
            .collect::<HashSet<_>>()
            .len()
            > 1
    );
    assert!(
        rects
            .iter()
            .map(|rect| rect.y)
            .collect::<HashSet<_>>()
            .len()
            > 1
    );
    for (index, card) in rects.iter().enumerate() {
        assert!(
            rects[index + 1..].iter().all(|later| {
                card.x < later.x
                    || card.y < later.y
                    || card.x >= later.x + later.w
                    || card.y >= later.y + later.h
            }),
            "a later card covered card {index}'s activation corner"
        );
    }
}

#[test]
fn active_card_gets_the_largest_grid_territory() {
    let work = Rect::new(0, 0, 1200, 800);
    let sizes = [Size::new(800, 600); 12];
    let grid = CardGrid::for_work_rect(work, sizes.len());
    let active = 6;
    let (active_row, active_column) = grid.position(active);
    let columns = weighted_edges(work.x, work.w, grid.columns, active_column);
    let rows = weighted_edges(work.y, work.h, grid.rows, active_row);

    let active_width = columns[active_column + 1] - columns[active_column];
    let active_height = rows[active_row + 1] - rows[active_row];

    assert!(
        columns
            .windows(2)
            .all(|edge| edge[1] - edge[0] <= active_width)
    );
    assert!(
        rows.windows(2)
            .all(|edge| edge[1] - edge[0] <= active_height)
    );
    let widths = columns
        .windows(2)
        .map(|edge| edge[1] - edge[0])
        .collect::<Vec<_>>();
    assert!(widths[active_column] > widths[active_column - 1]);
    assert!(widths[active_column - 1] > widths[active_column - 2]);
}

#[test]
fn every_card_uses_one_duration_even_in_a_dense_overview() {
    let tags = TagMask::single(1).unwrap();
    let windows = (1..=12).map(WindowId).collect::<Vec<_>>();
    let mut monitor = Monitor {
        monitor_rect: Rect::new(0, 0, 1200, 700),
        available_rect: Rect::new(0, 0, 1200, 700),
        clients: windows.clone(),
        overview_state: Some(OverviewState::new(
            tags,
            windows.clone(),
            HashMap::new(),
            windows.first().copied(),
        )),
        ..Monitor::default()
    };
    monitor.set_selected_tags(tags);
    let clients = windows
        .iter()
        .copied()
        .map(|win| {
            (
                win,
                Client {
                    win,
                    tags,
                    geo: Rect::new(100, 100, 700, 500),
                    ..Client::default()
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let layout = compute(&mut monitor, &clients);

    assert_eq!(layout.moves.len(), windows.len());
    assert!(layout.moves.iter().all(|output| {
        output.options.mode == crate::geometry::MoveResizeMode::AnimateTo
            && output.options.duration
                == std::time::Duration::from_millis(
                    crate::constants::animation::DEFAULT_ANIMATION_MILLIS,
                )
    }));
}

#[test]
fn keyboard_navigation_matches_the_visual_grid() {
    let work = Rect::new(0, 0, 1200, 700);
    let windows = (1..=8).map(WindowId).collect::<Vec<_>>();

    assert_eq!(
        grid_neighbor(&windows, Some(WindowId(2)), Direction::Down, work),
        Some(WindowId(6))
    );
    assert_eq!(
        grid_neighbor(&windows, Some(WindowId(8)), Direction::Up, work),
        Some(WindowId(4))
    );
    assert_eq!(
        grid_neighbor(&windows, Some(WindowId(1)), Direction::Left, work),
        None
    );
}

#[test]
fn stationary_pointer_cannot_retarget_a_moving_card_field() {
    let tags = TagMask::single(1).unwrap();
    let first = WindowId(1);
    let second = WindowId(2);
    let point = Point::new(400, 300);
    let mut state = OverviewState::new(tags, vec![first, second], HashMap::new(), Some(first));

    assert!(state.update_pointer_target(Some(second), Some(point)));
    // A synthetic crossing caused by the animation uses the same root point.
    assert!(!state.update_pointer_target(Some(first), Some(point)));
    assert_eq!(state.active_window, Some(second));

    assert!(state.update_pointer_target(Some(first), Some(Point::new(399, 300))));
    assert_eq!(state.active_window, Some(first));
}

#[test]
fn returning_from_overview_does_not_create_same_tag_history() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let mut monitor = Monitor {
        prev_tag: Some(1),
        ..Monitor::default()
    };
    monitor.set_selected_tags(tag2);
    commit_overview_tags(&mut monitor, tag2);

    assert_eq!(monitor.selected_tags(), tag2);
    assert_eq!(monitor.prev_tag, Some(1));
    assert_ne!(monitor.current_tag_number(), monitor.prev_tag);
    assert_eq!(monitor.prev_tag.and_then(TagMask::single), Some(tag1));
}

#[test]
fn selecting_another_overview_card_records_the_origin_tag() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let tag3 = TagMask::single(3).unwrap();
    let mut monitor = Monitor {
        prev_tag: Some(1),
        ..Monitor::default()
    };
    monitor.set_selected_tags(tag2);
    commit_overview_tags(&mut monitor, tag3);

    assert_eq!(monitor.selected_tags(), tag3);
    assert_eq!(monitor.prev_tag, Some(2));
    assert_eq!(monitor.prev_tag.and_then(TagMask::single), Some(tag2));
    assert_ne!(monitor.prev_tag.and_then(TagMask::single), Some(tag1));
}

#[test]
fn overview_order_groups_windows_by_their_first_tag_stably() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let mut monitor = Monitor::default();
    monitor.clients = vec![WindowId(3), WindowId(1), WindowId(2)];
    let clients = HashMap::from([
        (
            WindowId(1),
            Client {
                win: WindowId(1),
                tags: tag1,
                ..Client::default()
            },
        ),
        (
            WindowId(2),
            Client {
                win: WindowId(2),
                tags: tag1,
                ..Client::default()
            },
        ),
        (
            WindowId(3),
            Client {
                win: WindowId(3),
                tags: tag2,
                ..Client::default()
            },
        ),
    ]);

    assert_eq!(
        initial_window_order(&monitor, &clients, tag1 | tag2),
        vec![WindowId(1), WindowId(2), WindowId(3)]
    );
}

#[test]
fn a_window_mapped_during_overview_gets_one_restore_snapshot() {
    let tags = TagMask::single(1).unwrap();
    let win = WindowId(1);
    let original = Rect::new(40, 60, 500, 400);
    let mut monitor = Monitor {
        available_rect: Rect::new(0, 0, 1000, 700),
        clients: vec![win],
        overview_state: Some(OverviewState::new(tags, Vec::new(), HashMap::new(), None)),
        ..Monitor::default()
    };
    monitor.set_selected_tags(tags);
    let mut client = Client {
        win,
        tags,
        geo: original,
        ..Client::default()
    };
    let mut clients = HashMap::from([(win, client.clone())]);

    let _ = compute(&mut monitor, &clients);
    client.geo = Rect::new(300, 200, 500, 400);
    clients.insert(win, client);
    let _ = compute(&mut monitor, &clients);

    assert_eq!(
        monitor.overview_state.as_ref().unwrap().restore_geometry[&win],
        original
    );
}

#[test]
fn hovered_card_is_committed_on_overview_confirmation() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let first = WindowId(1);
    let second = WindowId(2);
    let mut wm = wm_with_overview_clients(tag1, &[(first, tag1), (second, tag2)]);

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag1
    );
    assert_eq!(
        wm.core.model.expect_selected_monitor().visible_tags(),
        TagMask::all(2)
    );
    assert!(hover_window(
        &mut wm.ctx(),
        Some(second),
        Some(Point::new(900, 300))
    ));
    // Hover selection is pending: the application does not receive keyboard
    // focus until the user confirms overview.
    assert_eq!(wm.core.model.selected_win(), Some(first));

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);

    assert_eq!(wm.core.model.selected_win(), Some(second));
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag2
    );
}

#[test]
fn keyboard_navigation_continues_from_the_hovered_card() {
    let tags = TagMask::single(1).unwrap();
    let first = WindowId(1);
    let second = WindowId(2);
    let mut wm = wm_with_overview_clients(tags, &[(first, tags), (second, tags)]);

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
    hover_window(&mut wm.ctx(), Some(second), Some(Point::new(900, 300)));
    assert!(focus_direction(&mut wm.ctx(), Direction::Left));

    let state = wm
        .core
        .model
        .expect_selected_monitor()
        .overview_state
        .as_ref()
        .unwrap();
    assert_eq!(state.active_window, Some(first));
    assert_eq!(wm.core.model.selected_win(), Some(first));
}

#[test]
fn layout_action_commits_hovered_card_before_changing_its_tag_layout() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let first = WindowId(1);
    let second = WindowId(2);
    let mut wm = wm_with_overview_clients(tag1, &[(first, tag1), (second, tag2)]);

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
    hover_window(&mut wm.ctx(), Some(second), Some(Point::new(900, 300)));
    crate::actions::execute_key_action(
        &mut wm.ctx(),
        &crate::actions::KeyAction::named(crate::actions::NamedAction::ToggleTilingMaximized),
    );

    let monitor = wm.core.model.expect_selected_monitor();
    assert!(monitor.overview_state.is_none());
    assert_eq!(monitor.selected_tags(), tag2);
    assert_eq!(monitor.selected, Some(second));
    assert_eq!(
        monitor.current_layout(),
        crate::layouts::PresentationMode::Maximized
    );
    assert!(!monitor.per_tag.contains_key(&TagMask::all(2)));
}

#[test]
fn explicit_tag_navigation_cancels_the_overview_projection() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let mut wm = wm_with_overview_clients(tag1, &[(WindowId(1), tag1), (WindowId(2), tag2)]);

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
    crate::actions::execute_key_action(
        &mut wm.ctx(),
        &crate::actions::KeyAction::ViewTag { tag_idx: 1 },
    );

    let monitor = wm.core.model.expect_selected_monitor();
    assert!(monitor.overview_state.is_none());
    assert_eq!(monitor.selected_tags(), tag2);
}

#[test]
fn visibility_uses_the_projection_while_workspace_state_stays_authoritative() {
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let mut wm = wm_with_overview_clients(tag1, &[(WindowId(1), tag1), (WindowId(2), tag2)]);

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);

    let monitor = wm.core.model.expect_selected_monitor();
    assert_eq!(monitor.selected_tags(), tag1);
    assert_eq!(monitor.visible_tags(), TagMask::all(2));
    assert!(
        crate::client::visibility::visibility_plan(&wm.core.model)
            .into_iter()
            .all(|entry| entry.visible)
    );
}

#[test]
fn changing_monitors_cancels_the_session_on_its_owner() {
    let tag1 = TagMask::single(1).unwrap();
    let mut wm = wm_with_overview_clients(tag1, &[(WindowId(1), tag1)]);
    let first_monitor_id = wm.core.model.selected_monitor_id();
    let second_monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(1200, 0, 1200, 700),
        available_rect: Rect::new(1200, 0, 1200, 700),
        tag_set: [tag1; 2],
        ..Monitor::default()
    });

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
    assert!(crate::focus::select_monitor(
        &mut wm.ctx(),
        second_monitor_id
    ));

    assert_eq!(wm.core.model.selected_monitor_id(), second_monitor_id);
    assert!(
        wm.core
            .model
            .monitor(first_monitor_id)
            .unwrap()
            .overview_state
            .is_none()
    );
    assert_eq!(
        wm.core.behavior.current_mode,
        crate::core_state::ActiveWmMode::Default
    );
}

#[test]
fn removing_the_last_card_leaves_overview() {
    let tags = TagMask::single(1).unwrap();
    let win = WindowId(1);
    let mut wm = wm_with_overview_clients(tags, &[(win, tags)]);

    toggle_overview(&mut wm.ctx(), TagMask::ALL_BITS);
    assert!(crate::client::lifecycle::remove_managed_client(&mut wm.ctx(), win).is_some());

    assert!(!wm.core.model.is_overview_active());
    assert_eq!(
        wm.core.behavior.current_mode,
        crate::core_state::ActiveWmMode::Default
    );
}

#[test]
fn overview_exit_is_a_noop_for_other_modes() {
    let tags = TagMask::single(1).unwrap();
    let mut wm = wm_with_overview_clients(tags, &[(WindowId(1), tags)]);
    let resize_mode = crate::core_state::ActiveWmMode::Named("resize".to_string());
    wm.core.behavior.current_mode = resize_mode.clone();

    exit_overview(&mut wm.ctx(), ExitMode::RestorePrevious);

    assert_eq!(wm.core.behavior.current_mode, resize_mode);
}
