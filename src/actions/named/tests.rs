use super::{ConfigAssignment, NamedAction, focus_vertical, move_horizontal, move_vertical};
use crate::backend::Backend;
use crate::backend::wayland::WaylandBackend;
use crate::layouts::tree::Preset;

use crate::layouts::{LayoutCommand, PresentationMode};
use crate::types::{
    Client, ClientMode, HorizontalDirection, Monitor, Rect, StackDirection, TagMask,
    VerticalDirection, WindowId,
};
use crate::wm::Wm;

fn maximized_tiled_wm(windows: &[WindowId], selected: WindowId) -> Wm {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 3;
    let tag = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1200, 800),
        available_rect: Rect::new(0, 0, 1200, 800),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);
    for &win in windows {
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags: tag,
            mode: ClientMode::tiled(),
            ..Client::default()
        });
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag);
    monitor.clients = windows.to_vec();
    monitor.selected = Some(selected);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::Grid, windows, 1);
    monitor.per_tag_state().presentation = PresentationMode::Maximized;
    wm
}

#[test]
fn layout_command_from_name_accepts_only_canonical_names() {
    assert_eq!(LayoutCommand::from_name("tile"), Some(LayoutCommand::Tile));
    assert_eq!(
        LayoutCommand::from_name("floating"),
        Some(LayoutCommand::Floating)
    );
    assert_eq!(
        LayoutCommand::from_name("maximized"),
        Some(LayoutCommand::Maximized)
    );
    assert_eq!(
        LayoutCommand::from_name("bottom-stack"),
        Some(LayoutCommand::BottomStack)
    );
    assert_eq!(LayoutCommand::from_name("bad"), None);
}

#[test]
fn config_toggle_flips_the_animation_switch_and_config_set_forces_it() {
    let mut wm = maximized_tiled_wm(&[WindowId(1)], WindowId(1));
    assert!(wm.core.config.animations.enabled);

    NamedAction::ConfigToggle("animations.enabled".into())
        .execute(&mut wm.ctx())
        .unwrap();
    assert!(!wm.core.config.animations.enabled);

    let force_on = || {
        NamedAction::ConfigSet(ConfigAssignment {
            key: "animations.enabled".into(),
            value: "true".into(),
        })
    };
    force_on().execute(&mut wm.ctx()).unwrap();
    assert!(wm.core.config.animations.enabled);

    let force_off = || {
        NamedAction::ConfigSet(ConfigAssignment {
            key: "animations.enabled".into(),
            value: "false".into(),
        })
    };
    force_off().execute(&mut wm.ctx()).unwrap();
    assert!(!wm.core.config.animations.enabled);
}

fn parse(name: &str, args: &[&str]) -> Result<NamedAction, String> {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    NamedAction::parse(name, &args)
}

#[test]
fn actions_parse_typed_arguments_once() {
    assert_eq!(
        parse("edge_scratchpad_direction_left", &[]),
        Ok(NamedAction::EdgeScratchpadDirectionLeft)
    );
    assert_eq!(
        parse("focus_stack", &["backward"]),
        Ok(NamedAction::FocusStack(StackDirection::Previous))
    );
    assert_eq!(
        parse("config_toggle", &["animations.enabled"]),
        Ok(NamedAction::ConfigToggle("animations.enabled".into()))
    );
    assert_eq!(
        parse("config_set", &["tags.show_icons", "true"]),
        Ok(NamedAction::ConfigSet(ConfigAssignment {
            key: "tags.show_icons".into(),
            value: "true".into(),
        }))
    );
    assert_eq!(
        parse("set_layout", &["bottom-stack"]),
        Ok(NamedAction::SetLayout(LayoutCommand::BottomStack))
    );

    for (name, args) in [
        ("config_toggle", &["a", "b"][..]),
        ("config_set", &["onlykey"][..]),
        ("focus_next", &["unexpected"]),
        ("set_layout", &[]),
        ("set_layout", &["not-a-layout"]),
        ("set_border", &["-1"]),
        ("inc_gaps", &["x"]),
        ("dec_gaps", &["2", "3"]),
        ("spawn", &[]),
        ("none", &[]),
    ] {
        assert!(parse(name, args).is_err(), "{name} {args:?} should fail");
    }
}

#[test]
fn rendered_arguments_parse_back_to_the_same_action() {
    for action in [
        NamedAction::Spawn(vec!["printf".into(), "hello world".into()]),
        NamedAction::FocusMon(crate::types::MonitorDirection::Prev),
        NamedAction::ConfigSet(ConfigAssignment {
            key: "window.focus_follows_mouse".into(),
            value: "force".into(),
        }),
        NamedAction::IncMasterCount(Some(-1)),
        NamedAction::SetBorder(None),
    ] {
        assert_eq!(
            NamedAction::parse(action.name(), &action.args()),
            Ok(action)
        );
    }
}

#[test]
fn gap_actions_move_both_gaps_and_clamp_at_zero() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.config.layout.inner_gap = 4;
    wm.core.config.layout.outer_gap = 8;

    NamedAction::IncGaps(Some(3))
        .execute(&mut wm.ctx())
        .unwrap();
    assert_eq!(wm.core.config.layout.inner_gap, 7);
    assert_eq!(wm.core.config.layout.outer_gap, 11);

    // The default step applies when no argument is passed.
    NamedAction::DecGaps(None).execute(&mut wm.ctx()).unwrap();
    assert_eq!(wm.core.config.layout.inner_gap, 5);
    assert_eq!(wm.core.config.layout.outer_gap, 9);

    // Decreasing past the floor clamps instead of disabling windows into
    // negative gaps.
    for _ in 0..10 {
        NamedAction::DecGaps(None).execute(&mut wm.ctx()).unwrap();
    }
    assert_eq!(wm.core.config.layout.inner_gap, 0);
    assert_eq!(wm.core.config.layout.outer_gap, 0);
}

#[test]
fn config_actions_are_idempotent_when_set_and_alternate_when_toggled() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let set_on = || {
        NamedAction::ConfigSet(ConfigAssignment {
            key: "tags.show_icons".into(),
            value: "true".into(),
        })
    };
    set_on().execute(&mut wm.ctx()).unwrap();
    set_on().execute(&mut wm.ctx()).unwrap();
    assert!(wm.core.config.tags.show_icons);

    let toggle = || NamedAction::ConfigToggle("tags.show_icons".into());
    toggle().execute(&mut wm.ctx()).unwrap();
    assert!(!wm.core.config.tags.show_icons);
    toggle().execute(&mut wm.ctx()).unwrap();
    assert!(wm.core.config.tags.show_icons);
}

#[test]
fn config_actions_reject_bad_keys_and_values_without_mutating() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    assert!(
        NamedAction::ConfigToggle("layout.inner_gap".into())
            .execute(&mut wm.ctx())
            .is_err()
    );
    assert!(
        NamedAction::ConfigSet(ConfigAssignment {
            key: "window.border_width_px".into(),
            value: "-2".into(),
        })
        .execute(&mut wm.ctx())
        .is_err()
    );
    assert!(wm.core.config.window.border_width_px > 0);
}

#[test]
fn quit_action_uses_the_normal_wm_shutdown_flag() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    NamedAction::Quit.execute(&mut wm.ctx()).unwrap();
    assert!(!wm.running);
}

#[test]
fn action_dispatch_rejects_unknown_and_interaction_owned_modes() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

    let unknown = NamedAction::SetMode("does-not-exist".to_string())
        .execute(&mut wm.ctx())
        .unwrap_err();
    assert!(unknown.contains("not found"));

    let placement = NamedAction::SetMode(crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string())
        .execute(&mut wm.ctx())
        .unwrap_err();
    assert!(placement.contains("begin_tree_placement"));
}

#[test]
fn horizontal_window_move_crosses_tags_only_at_the_tree_edge() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 3;
    let tag1 = TagMask::single(1).unwrap();
    let tag2 = TagMask::single(2).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1200, 800),
        available_rect: Rect::new(0, 0, 1200, 800),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);

    let left = WindowId(1);
    let right = WindowId(2);
    for win in [left, right] {
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags: tag1,
            mode: ClientMode::tiled(),
            ..Client::default()
        });
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag1);
    monitor.clients = vec![left, right];
    monitor.selected = Some(left);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &[left, right], 1);

    move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

    // The first press has a visual neighbour, so it only swaps the tree.
    assert_eq!(wm.core.model.client(left).unwrap().tags, tag1);
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag1
    );

    move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

    // The same client is now at the right edge, so the next press carries
    // it into the adjacent tag and follows it there.
    assert_eq!(wm.core.model.client(left).unwrap().tags, tag2);
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag2
    );
    assert_eq!(wm.core.model.selected_win(), Some(left));
}

#[test]
fn maximized_window_move_reorders_adjacent_titles_not_hidden_visual_neighbors() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];
    let selected = WindowId(4);
    let mut wm = maximized_tiled_wm(&windows, selected);

    // In this grid, window 4's hidden visual neighbour to the left is
    // window 2. The exposed maximized order instead places window 3
    // immediately before it.
    assert_eq!(
        wm.core
            .model
            .expect_selected_monitor()
            .per_tag()
            .unwrap()
            .layout_tree
            .visual_neighbor(selected, crate::layouts::tree::Side::Left),
        Some(WindowId(2))
    );

    move_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

    let monitor = wm.core.model.expect_selected_monitor();
    assert_eq!(
        monitor.per_tag().unwrap().layout_tree.leaves(),
        vec![WindowId(1), WindowId(2), WindowId(4), WindowId(3)]
    );
    assert_eq!(
        monitor.bar_client_order(&wm.core.model.clients),
        vec![WindowId(1), WindowId(2), WindowId(4), WindowId(3)]
    );
    assert_eq!(monitor.selected, Some(selected));
}

#[test]
fn maximized_horizontal_move_crosses_tags_at_title_strip_boundary() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let selected = WindowId(3);
    let mut wm = maximized_tiled_wm(&windows, selected);
    let tag2 = TagMask::single(2).unwrap();

    move_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

    assert_eq!(wm.core.model.client(selected).unwrap().tags, tag2);
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag2
    );
    assert_eq!(wm.core.model.selected_win(), Some(selected));
}

#[test]
fn maximized_vertical_move_stops_at_title_strip_boundary() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let selected = WindowId(3);
    let mut wm = maximized_tiled_wm(&windows, selected);
    let tag1 = TagMask::single(1).unwrap();

    move_vertical(&mut wm.ctx(), VerticalDirection::Up);
    assert_eq!(
        wm.core
            .model
            .expect_selected_monitor()
            .per_tag()
            .unwrap()
            .layout_tree
            .leaves(),
        vec![WindowId(1), WindowId(3), WindowId(2)]
    );

    move_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_eq!(
        wm.core
            .model
            .expect_selected_monitor()
            .per_tag()
            .unwrap()
            .layout_tree
            .leaves(),
        windows
    );

    move_vertical(&mut wm.ctx(), VerticalDirection::Down);

    let monitor = wm.core.model.expect_selected_monitor();
    assert_eq!(monitor.per_tag().unwrap().layout_tree.leaves(), windows);
    assert_eq!(monitor.selected_tags(), tag1);
    assert_eq!(monitor.selected, Some(selected));
}

#[test]
fn maximized_move_does_not_treat_pending_tree_reconciliation_as_a_boundary() {
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let selected = WindowId(3);
    let mut wm = maximized_tiled_wm(&windows, selected);
    let tag1 = TagMask::single(1).unwrap();
    assert!(
        wm.core
            .model
            .expect_selected_monitor_mut()
            .per_tag_state()
            .layout_tree
            .remove(selected)
    );

    // Title order defensively appends a newly managed tiled client before
    // the next arrange reconciles its leaf. Moving left during that window
    // must not fall through to an adjacent-tag transfer.
    move_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

    assert_eq!(wm.core.model.client(selected).unwrap().tags, tag1);
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag1
    );
    assert_eq!(wm.core.model.selected_win(), Some(selected));
}

#[test]
fn vertical_focus_falls_back_to_cycling_in_bar_order() {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let tag = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(Monitor {
        monitor_rect: Rect::new(0, 0, 1200, 800),
        available_rect: Rect::new(0, 0, 1200, 800),
        ..Monitor::default()
    });
    wm.core.model.monitors.set_selected(monitor_id);

    let left = WindowId(1);
    let middle = WindowId(2);
    let right = WindowId(3);
    for win in [left, middle, right] {
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags: tag,
            mode: ClientMode::tiled(),
            ..Client::default()
        });
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag);
    monitor.clients = vec![left, middle, right];
    monitor.selected = Some(middle);
    monitor.per_tag_state().layout_tree.apply_preset(
        Preset::BottomStack,
        &[left, middle, right],
        0,
    );

    focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_eq!(wm.core.model.selected_win(), Some(right));

    focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_eq!(wm.core.model.selected_win(), Some(left));

    focus_vertical(&mut wm.ctx(), VerticalDirection::Up);
    assert_eq!(wm.core.model.selected_win(), Some(right));
}
