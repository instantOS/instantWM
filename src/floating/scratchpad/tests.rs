use super::{
    EdgeSlideRects, ScratchpadShowOptions, hide_scratchpad_window, name_from_window_identity,
    regular_scratchpad_rect, scratchpad_restore_window, set_scratchpad_direction,
    show_scratchpad_window_with_options, show_transferred_scratchpad,
};
use crate::test_support::MonitorBuilder;
use crate::test_support::TestWm as Wm;
use crate::types::input::EdgeDirection;
use crate::types::{Client, ClientPlacement, Monitor, Rect, Size, TagMask, WindowId};

#[test]
fn scratchpad_identity_accepts_wayland_app_id_and_x11_instance() {
    assert_eq!(
        name_from_window_identity("scratchpad_menu", ""),
        Some("menu")
    );
    assert_eq!(
        name_from_window_identity("kitty", "scratchpad_notes"),
        Some("notes")
    );
    assert_eq!(name_from_window_identity("scratchpad_", "kitty"), None);
    assert_eq!(name_from_window_identity("kitty", "kitty"), None);
}

#[test]
fn regular_scratchpad_percentages_include_borders_and_center_in_content() {
    let content = Rect::new(100, 230, 1920, 1050);
    let rect = regular_scratchpad_rect(content, 2, 50, 60).unwrap();

    assert_eq!(rect, Rect::new(580, 440, 956, 626));
    assert!(content.contains_rect(&Rect::new(rect.x, rect.y, rect.w + 2 * 2, rect.h + 2 * 2)));
}

#[test]
fn regular_scratchpad_rejects_invalid_percentages() {
    let content = Rect::new(0, 0, 1920, 1080);

    assert!(regular_scratchpad_rect(content, 2, 0, 60).is_err());
    assert!(regular_scratchpad_rect(content, 2, 50, 101).is_err());
}

#[test]
fn shown_rects_stay_inside_content_and_hidden_rects_stay_outside() {
    let content = Rect::new(100, 230, 1920, 1050);

    for direction in [
        EdgeDirection::Top,
        EdgeDirection::Right,
        EdgeDirection::Bottom,
        EdgeDirection::Left,
    ] {
        let slide = EdgeSlideRects::new(content, direction, Size::new(640, 360));

        assert!(content.contains_rect(&slide.shown), "{direction:?}");
        assert!(!content.intersects_other(&slide.hidden), "{direction:?}");
        assert_eq!(slide.hidden.size(), slide.shown.size());
    }
}

#[test]
fn oversized_edge_scratchpads_are_clamped_to_content() {
    let content = Rect::new(10, 20, 8, 3);

    for direction in [
        EdgeDirection::Top,
        EdgeDirection::Right,
        EdgeDirection::Bottom,
        EdgeDirection::Left,
    ] {
        let slide = EdgeSlideRects::new(content, direction, Size::new(500, 500));

        assert!(content.contains_rect(&slide.shown), "{direction:?}");
        assert!(slide.shown.w > 0);
        assert!(slide.shown.h > 0);
    }
}

#[test]
fn setting_scratchpad_direction_does_not_mutate_an_ordinary_window() {
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    let monitor_id = wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    let win = WindowId(76);
    let original_geo = Rect::new(100, 120, 800, 600);
    assert!(wm.core.state.model.add_client(
        monitor_id,
        Client {
            win,
            geo: original_geo,
            border_width: 3,
            is_locked: false,
            ..Client::default()
        }
    ));

    set_scratchpad_direction(&mut wm.test_ctx(), win, EdgeDirection::Left);

    let client = wm.core.state.model.client(win).unwrap();
    assert_eq!(client.geo, original_geo);
    assert_eq!(client.border_width, 3);
    assert!(!client.is_locked);
}

#[test]
fn edge_scratchpad_hide_defers_concealment_until_the_animation_finishes() {
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    let monitor_id = wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    wm.core.state.model.monitors.set_selected(monitor_id);

    let scratchpad = WindowId(90);
    let mut client = Client {
        win: scratchpad,
        geo: Rect::new(0, 0, 640, 360),
        ..Client::default()
    };
    client
        .promote_to_scratchpad(monitor_id, "edge", Some(EdgeDirection::Top), 1920, 1080)
        .unwrap();
    wm.core.state.model.add_client(monitor_id, client);

    hide_scratchpad_window(&mut wm.test_ctx(), scratchpad);

    // The slide-out is playing: the window stays logically visible and a
    // pending hide is queued.
    assert!(
        wm.core
            .state
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
    assert!(wm.core.work.has_pending_scratchpad_hide(scratchpad));

    // Completing the animation performs the deferred logical hide.
    crate::floating::scratchpad::finish_scratchpad_hides(&mut wm.test_ctx(), &[scratchpad]);
    assert!(
        !wm.core
            .state
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
}

#[test]
fn showing_during_a_slide_out_cancels_the_pending_hide() {
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    let monitor_id = wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    wm.core.state.model.monitors.set_selected(monitor_id);

    let scratchpad = WindowId(91);
    let mut client = Client {
        win: scratchpad,
        geo: Rect::new(0, 0, 640, 360),
        ..Client::default()
    };
    client
        .promote_to_scratchpad(monitor_id, "edge", Some(EdgeDirection::Top), 1920, 1080)
        .unwrap();
    wm.core.state.model.add_client(monitor_id, client);

    hide_scratchpad_window(&mut wm.test_ctx(), scratchpad);
    assert!(wm.core.work.has_pending_scratchpad_hide(scratchpad));

    let shown = show_scratchpad_window_with_options(
        &mut wm.test_ctx(),
        scratchpad,
        ScratchpadShowOptions {
            monitor_id,
            focus: true,
            warp_pointer: false,
        },
    )
    .unwrap();

    // Reversing the toggle reports the show succeeded and cancels the
    // deferred hide; the overlay stays up.
    assert!(shown);
    assert!(!wm.core.work.has_pending_scratchpad_hide(scratchpad));
    assert!(
        wm.core
            .state
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
}

#[test]
fn transferred_scratchpad_targets_a_monitor_without_stealing_selection() {
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    let source = wm.core.state.model.monitors.push(Monitor::default());
    let target = wm.core.state.model.monitors.push(Monitor::default());
    wm.core.state.model.monitors.set_selected(source);

    let focused = WindowId(80);
    wm.core.state.model.readopt_client(
        source,
        Client {
            win: focused,
            ..Client::default()
        },
        true,
    );

    let scratchpad = WindowId(81);
    let mut client = Client {
        win: scratchpad,
        is_hidden: true,
        ..Client::default()
    };
    client
        .promote_to_scratchpad(target, "transfer", None, 1920, 1080)
        .unwrap();
    wm.core.state.model.add_client(target, client);

    show_transferred_scratchpad(&mut wm.test_ctx(), scratchpad, target);

    assert_eq!(wm.core.state.model.selected_monitor_id(), source);
    assert_eq!(wm.core.state.model.selected_win(), Some(focused));
    assert_eq!(
        wm.core.state.model.monitor_of_client(scratchpad),
        Some(target)
    );
    assert!(
        wm.core
            .state
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
}

#[test]
fn restoring_a_hidden_portable_scratchpad_returns_to_its_original_monitor() {
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    wm.core.state.model.tags.num_tags = 9;
    let monitor = MonitorBuilder::new()
        .monitor_rect(Rect::new(0, 0, 1920, 1080))
        .bar(0, false)
        .tag_count(9)
        .build();
    let original_monitor = wm.core.state.model.monitors.push(monitor.clone());
    let scratch_monitor = wm.core.state.model.monitors.push(monitor);
    wm.core.state.model.monitors.set_selected(scratch_monitor);

    let win = WindowId(77);
    let original_tags = TagMask::single(2).unwrap();
    wm.core
        .state
        .model
        .monitor_mut(original_monitor)
        .unwrap()
        .set_selected_tags(original_tags);
    wm.core
        .state
        .model
        .monitor_mut(scratch_monitor)
        .unwrap()
        .set_selected_tags(TagMask::single(1).unwrap());
    let mut client = Client {
        win,
        tags: original_tags,
        ..Client::default()
    };
    client
        .promote_to_scratchpad(original_monitor, "portable", None, 1920, 1080)
        .unwrap();
    client.is_hidden = true;
    assert!(wm.core.state.model.add_client(original_monitor, client));
    assert!(
        wm.core
            .state
            .model
            .reassign_client_monitor(win, scratch_monitor)
    );

    scratchpad_restore_window(&mut wm.test_ctx(), win, None).unwrap();

    assert_eq!(
        wm.core.state.model.monitor_of_client(win),
        Some(original_monitor)
    );
    let restored = wm.core.state.model.client(win).unwrap();
    assert!(!restored.is_scratchpad());
    assert!(!restored.is_hidden);
    assert_eq!(restored.tags, original_tags);
    assert_eq!(restored.placement(), ClientPlacement::Tiling);
}

#[test]
fn transferring_during_slide_out_preserves_pending_hide() {
    let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
    let source = wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    let target = wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(1920, 0, 1920, 1080))
            .build(),
    );
    wm.core.state.model.monitors.set_selected(source);
    let win = WindowId(92);
    let mut client = Client {
        win,
        geo: Rect::new(0, 0, 640, 360),
        ..Client::default()
    };
    client
        .promote_to_scratchpad(
            source,
            "transfer-hide",
            Some(EdgeDirection::Top),
            1920,
            1080,
        )
        .unwrap();
    wm.core.state.model.add_client(source, client);
    assert!(hide_scratchpad_window(&mut wm.test_ctx(), win));
    assert!(wm.core.work.has_pending_scratchpad_hide(win));

    let outcome = crate::monitor::transfer_client(
        &mut wm.test_ctx(),
        win,
        target,
        crate::monitor::TransferFocus::Preserve,
    )
    .unwrap();

    assert_eq!(outcome.target_monitor, target);
    assert_eq!(wm.core.state.model.monitor_of_client(win), Some(target));
    assert!(wm.core.work.has_pending_scratchpad_hide(win));
    super::finish_scratchpad_hides(&mut wm.test_ctx(), &[win]);
    assert!(wm.core.state.model.client(win).unwrap().is_hidden);
}
