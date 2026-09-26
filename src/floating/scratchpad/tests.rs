use super::{
    EdgeSlideRects, ScratchpadShowOptions, hide_scratchpad_window, name_from_window_identity,
    regular_scratchpad_rect, scratchpad_restore_window, set_scratchpad_direction,
    show_scratchpad_window_with_options, show_transferred_scratchpad,
};
use crate::backend::Backend;
use crate::backend::wayland::WaylandBackend;
use crate::types::input::EdgeDirection;
use crate::types::{Client, ClientPlacement, Monitor, Rect, Size, TagMask, WindowId};
use crate::wm::Wm;

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
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1920, 1080),
        available_rect: Rect::new(0, 0, 1920, 1080),
        ..Monitor::default()
    });
    let win = WindowId(76);
    let original_geo = Rect::new(100, 120, 800, 600);
    assert!(wm.core.model.add_client(
        monitor_id,
        Client {
            win,
            geo: original_geo,
            border_width: 3,
            is_locked: false,
            ..Client::default()
        }
    ));

    set_scratchpad_direction(&mut wm.ctx(), win, EdgeDirection::Left);

    let client = wm.core.model.client(win).unwrap();
    assert_eq!(client.geo, original_geo);
    assert_eq!(client.border_width, 3);
    assert!(!client.is_locked);
}

#[test]
fn edge_scratchpad_hide_defers_concealment_until_the_animation_finishes() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1920, 1080),
        available_rect: Rect::new(0, 0, 1920, 1080),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);

    let scratchpad = WindowId(90);
    let mut client = Client {
        win: scratchpad,
        geo: Rect::new(0, 0, 640, 360),
        ..Client::default()
    };
    client
        .promote_to_scratchpad(monitor_id, "edge", Some(EdgeDirection::Top), 1920, 1080)
        .unwrap();
    wm.core.model.add_client(monitor_id, client);

    hide_scratchpad_window(&mut wm.ctx(), scratchpad);

    // The slide-out is playing: the window stays logically visible and a
    // pending hide is queued.
    assert!(
        wm.core
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
    assert!(wm.work.has_pending_scratchpad_hide(scratchpad));

    // Completing the animation performs the deferred logical hide.
    crate::floating::scratchpad::finish_scratchpad_hides(&mut wm.ctx(), &[scratchpad]);
    assert!(
        !wm.core
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
}

#[test]
fn showing_during_a_slide_out_cancels_the_pending_hide() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1920, 1080),
        available_rect: Rect::new(0, 0, 1920, 1080),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);

    let scratchpad = WindowId(91);
    let mut client = Client {
        win: scratchpad,
        geo: Rect::new(0, 0, 640, 360),
        ..Client::default()
    };
    client
        .promote_to_scratchpad(monitor_id, "edge", Some(EdgeDirection::Top), 1920, 1080)
        .unwrap();
    wm.core.model.add_client(monitor_id, client);

    hide_scratchpad_window(&mut wm.ctx(), scratchpad);
    assert!(wm.work.has_pending_scratchpad_hide(scratchpad));

    let shown = show_scratchpad_window_with_options(
        &mut wm.ctx(),
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
    assert!(!wm.work.has_pending_scratchpad_hide(scratchpad));
    assert!(
        wm.core
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
}

#[test]
fn transferred_scratchpad_targets_a_monitor_without_stealing_selection() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let source = wm.core.model.monitors.push(Monitor::default());
    let target = wm.core.model.monitors.push(Monitor::default());
    wm.core.model.monitors.set_selected(source);

    let focused = WindowId(80);
    wm.core.model.readopt_client(
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
    wm.core.model.add_client(target, client);

    show_transferred_scratchpad(&mut wm.ctx(), scratchpad, target);

    assert_eq!(wm.core.model.selected_monitor_id(), source);
    assert_eq!(wm.core.model.selected_win(), Some(focused));
    assert_eq!(wm.core.model.monitor_of_client(scratchpad), Some(target));
    assert!(
        wm.core
            .model
            .client(scratchpad)
            .unwrap()
            .is_scratchpad_visible()
    );
}

#[test]
fn restoring_a_hidden_portable_scratchpad_returns_to_its_original_monitor() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 9;
    let monitor = Monitor {
        monitor_rect: Rect::new(0, 0, 1920, 1080),
        available_rect: Rect::new(0, 0, 1920, 1080),
        bar_default_show: false,
        ..Monitor::default()
    };
    let original_monitor = wm.core.model.monitors.push(monitor.clone());
    let scratch_monitor = wm.core.model.monitors.push(monitor);
    wm.core.model.monitors.set_selected(scratch_monitor);

    let win = WindowId(77);
    let original_tags = TagMask::single(2).unwrap();
    wm.core
        .model
        .monitor_mut(original_monitor)
        .unwrap()
        .set_selected_tags(original_tags);
    wm.core
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
    assert!(wm.core.model.add_client(original_monitor, client));
    assert!(wm.core.model.reassign_client_monitor(win, scratch_monitor));

    scratchpad_restore_window(&mut wm.ctx(), win, None).unwrap();

    assert_eq!(wm.core.model.monitor_of_client(win), Some(original_monitor));
    let restored = wm.core.model.client(win).unwrap();
    assert!(!restored.is_scratchpad());
    assert!(!restored.is_hidden);
    assert_eq!(restored.tags, original_tags);
    assert_eq!(restored.placement(), ClientPlacement::Tiling);
}
