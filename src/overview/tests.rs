use super::*;

#[test]
fn landscape_card_hand_preserves_sizes_and_exposes_every_card() {
    let work = Rect::new(10, 20, 1200, 700);
    let sizes = vec![Size::new(900, 600); 12];

    let rects = card_hand_rects(work, &sizes);

    assert_eq!(rects.len(), sizes.len());
    for (rect, size) in rects.iter().zip(&sizes) {
        assert_eq!(rect.size(), *size);
        assert!(rect.y >= work.y);
    }
    for pair in rects.windows(2) {
        let exposed_width = (pair[1].x - pair[0].x).min(pair[0].w);
        assert!(exposed_width > 0, "every covered card needs a hit strip");
    }
    let top = rects.last().unwrap();
    assert!(top.x < work.x + work.w);
}

#[test]
fn portrait_card_hand_cascades_vertically() {
    let work = Rect::new(50, 100, 600, 1200);
    let sizes = vec![Size::new(500, 850); 8];

    let rects = card_hand_rects(work, &sizes);

    for pair in rects.windows(2) {
        let exposed_height = (pair[1].y - pair[0].y).min(pair[0].h);
        assert!(exposed_height > 0, "every covered card needs a hit strip");
    }
    assert!(rects.windows(2).all(|pair| pair[0].x == pair[1].x));
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
        overview_state: Some(OverviewState::new(tags, Vec::new(), HashMap::new())),
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
