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
    let sizes = vec![Size::new(800, 600); 12];
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
    monitor.set_selected_tags(TagMask::all(9));

    restore_overview_tags(&mut monitor, tag2, tag2);

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
    monitor.set_selected_tags(TagMask::all(9));

    restore_overview_tags(&mut monitor, tag2, tag3);

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
