use super::{
    ConfigAssignment, NamedAction, focus_horizontal, focus_vertical, move_horizontal, move_vertical,
};
use crate::backend::Backend;
use crate::backend::wayland::WaylandBackend;
use crate::config::config_toml::{HorizontalEdge, VerticalEdge};
use crate::layouts::tree::Preset;

use crate::layouts::{LayoutCommand, PresentationMode};
use crate::test_support::{MonitorBuilder, add_client};
use crate::types::{
    Client, ClientMode, HorizontalDirection, MonitorId, Rect, StackDirection, TagMask,
    VerticalDirection, WindowId,
};
use crate::wm::Wm;

/// Restore a monitor's focus stack to `order`.
///
/// `add_client` prepends, so fixtures that care about bar order — or that
/// deliberately scramble it so a test can prove geometry wins — spell the
/// order out instead of inheriting it from the insertion sequence.
fn set_focus_order(wm: &mut Wm, monitor_id: MonitorId, order: &[WindowId]) {
    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .expect("monitor")
            .set_focus_order(order.to_vec())
    );
}

fn maximized_tiled_wm(windows: &[WindowId], selected: WindowId) -> Wm {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 3;
    let tag = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1200, 800))
            .tag_count(3)
            .build(),
    );
    wm.core.model.monitors.set_selected(monitor_id);
    for &win in windows {
        add_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win,
                tags: tag,
                mode: ClientMode::tiled(),
                ..Client::default()
            },
        );
    }
    set_focus_order(&mut wm, monitor_id, windows);
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag);
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
    let monitor_id = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1200, 800))
            .tag_count(3)
            .build(),
    );
    wm.core.model.monitors.set_selected(monitor_id);

    let left = WindowId(1);
    let right = WindowId(2);
    for win in [left, right] {
        add_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win,
                tags: tag1,
                mode: ClientMode::tiled(),
                ..Client::default()
            },
        );
    }
    set_focus_order(&mut wm, monitor_id, &[left, right]);
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag1);
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
        monitor.bar_client_order(),
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
fn vertical_focus_wraps_across_the_screen_when_tiled() {
    let (mut wm, [top, _, bottom]) = stacked_wm();

    // No window is below the bottom-most one, so `wrap` answers with the
    // topmost — a jump across the screen, not a walk through bar order.
    wm.core.model.expect_selected_monitor_mut().selected = Some(bottom);
    focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_eq!(wm.core.model.selected_win(), Some(top));

    // And symmetrically off the top edge, so neither direction is special.
    focus_vertical(&mut wm.ctx(), VerticalDirection::Up);
    assert_eq!(wm.core.model.selected_win(), Some(bottom));
}

#[test]
fn maximized_vertical_focus_cycles_in_bar_order() {
    // Maximized windows overlap, so there is no screen to jump across and
    // the bar order is the only cycle left. This is the sole place bar order
    // is consulted: tiled presentation resolves `wrap` against geometry.
    let windows = [WindowId(1), WindowId(2), WindowId(3)];
    let mut wm = maximized_tiled_wm(&windows, WindowId(3));

    focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_eq!(wm.core.model.selected_win(), Some(WindowId(1)));

    focus_vertical(&mut wm.ctx(), VerticalDirection::Up);
    assert_eq!(wm.core.model.selected_win(), Some(WindowId(3)));
}

/// Three windows stacked down the screen with real geometry, `top` focused.
///
/// Gives `focus_vertical` both an ordinary downward step and a genuine bottom
/// edge to reach, which a geometry-less fixture cannot tell apart: every
/// window shares one coordinate there, so the first press is already the
/// boundary.
fn stacked_wm() -> (Wm, [WindowId; 3]) {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 3;
    let tag = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1200, 800))
            .tag_count(3)
            .build(),
    );
    wm.core.model.monitors.set_selected(monitor_id);

    let [top, middle, bottom] = [WindowId(1), WindowId(2), WindowId(3)];
    // Three equal bands, so `direction_focus` has a real "below" to resolve
    // for the first two and genuinely nothing for the last.
    for (win, y) in [(top, 0), (middle, 266), (bottom, 533)] {
        add_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win,
                tags: tag,
                mode: ClientMode::tiled(),
                geo: Rect::new(0, y, 1200, 267),
                ..Client::default()
            },
        );
    }
    set_focus_order(&mut wm, monitor_id, &[top, middle, bottom]);
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag);
    monitor.selected = Some(top);
    monitor.per_tag_state().layout_tree.apply_preset(
        Preset::MasterStack,
        &[top, middle, bottom],
        1,
    );
    (wm, [top, middle, bottom])
}

#[test]
fn vertical_focus_ignores_the_horizontal_edge_policy() {
    // The two policies are read by different actions, so the horizontal one
    // must not be able to redirect a `focus_down`. Press the same key against
    // each setting and require an identical answer.
    let mut landed = Vec::new();
    for edge in [
        HorizontalEdge::Overflow,
        HorizontalEdge::Wrap,
        HorizontalEdge::None,
    ] {
        let (mut wm, [top, _, _]) = stacked_wm();
        wm.core.config.focus.horizontal_edge = edge;

        focus_vertical(&mut wm.ctx(), VerticalDirection::Down);

        assert_ne!(
            wm.core.model.selected_win(),
            Some(top),
            "{edge:?} turned the ordinary step into a boundary"
        );
        landed.push(wm.core.model.selected_win());
    }
    assert!(
        landed.iter().all(|win| *win == landed[0]),
        "{landed:?}: {landed:?} horizontal edges disagree about where a vertical step lands"
    );
}

#[test]
fn vertical_focus_stops_at_the_boundary_when_configured() {
    let (mut wm, [top, _, bottom]) = stacked_wm();
    wm.core.config.focus.vertical_edge = VerticalEdge::None;

    // The policy only governs the fallthrough, so an ordinary step between
    // two windows still moves.
    focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_ne!(
        wm.core.model.selected_win(),
        Some(top),
        "vertical_edge = \"none\" must not swallow the ordinary step"
    );

    // Park on the bottom-most window: there is nowhere below it, so the
    // press is consumed rather than wrapping back around to the top.
    wm.core.model.expect_selected_monitor_mut().selected = Some(bottom);
    focus_vertical(&mut wm.ctx(), VerticalDirection::Down);
    assert_eq!(wm.core.model.selected_win(), Some(bottom));
}

#[test]
fn horizontal_focus_ignores_the_vertical_edge_policy() {
    // The two policies are read by different actions. Pinning this keeps
    // `vertical_edge = "none"` from silently disabling tag overflow too.
    for edge in [VerticalEdge::Wrap, VerticalEdge::None] {
        let windows = [WindowId(1), WindowId(2)];
        let mut wm = tiled_row_wm(&windows, WindowId(2));
        wm.core.config.focus.vertical_edge = edge;
        let tag2 = TagMask::single(2).unwrap();

        focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag2,
            "{edge:?} changed horizontal navigation"
        );
    }
}

/// `windows` tiled on tag 1 in tree order, `selected` focused, with enough
/// tags that an overflow has somewhere to go.
fn tiled_row_wm(windows: &[WindowId], selected: WindowId) -> Wm {
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 3;
    let tag = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1200, 800))
            .tag_count(3)
            .build(),
    );
    wm.core.model.monitors.set_selected(monitor_id);
    // Real geometry laid out left to right, in slice order. Without it every
    // window would share one centre and a geometric wrap would have nothing
    // to move across.
    let width = 1200 / windows.len() as i32;
    for (index, &win) in windows.iter().enumerate() {
        add_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win,
                tags: tag,
                mode: ClientMode::tiled(),
                geo: Rect::new(index as i32 * width, 0, width, 800),
                ..Client::default()
            },
        );
    }
    set_focus_order(&mut wm, monitor_id, windows);
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag);
    monitor.selected = Some(selected);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, windows, 1);
    wm
}

#[test]
fn horizontal_focus_overflows_into_the_adjacent_tag_by_default() {
    let windows = [WindowId(1), WindowId(2)];
    let mut wm = tiled_row_wm(&windows, WindowId(2));
    let tag2 = TagMask::single(2).unwrap();

    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag2
    );
}

#[test]
fn horizontal_focus_wraps_to_the_far_end_of_the_same_tag() {
    let windows = [WindowId(1), WindowId(2)];
    let mut wm = tiled_row_wm(&windows, WindowId(2));
    wm.core.config.focus.horizontal_edge = HorizontalEdge::Wrap;
    let tag1 = TagMask::single(1).unwrap();

    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

    assert_eq!(wm.core.model.selected_win(), Some(WindowId(1)));
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag1
    );

    // Wrapping is symmetric and does not need the tag edge to be reached.
    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Left);
    assert_eq!(wm.core.model.selected_win(), Some(WindowId(2)));
}

#[test]
fn horizontal_focus_wrap_follows_geometry_not_bar_order() {
    // Three windows across the screen, but created in an order that has
    // nothing to do with their positions: the bar/stack order is B, A, C
    // while the screen reads A B C. The tiling tree follows the screen, so
    // the only thing that can steer the wrap here is geometry.
    let (a, b, c) = (WindowId(1), WindowId(2), WindowId(3));
    let screen_order = [a, b, c];
    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    wm.core.model.tags.num_tags = 3;
    let tag1 = TagMask::single(1).unwrap();
    let monitor_id = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1200, 800))
            .tag_count(3)
            .build(),
    );
    wm.core.model.monitors.set_selected(monitor_id);
    for (win, x) in [(a, 0), (b, 400), (c, 800)] {
        add_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win,
                tags: tag1,
                mode: ClientMode::tiled(),
                geo: Rect::new(x, 0, 400, 800),
                ..Client::default()
            },
        );
    }
    let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag1);
    // Scrambled bar order is the thing the wrap must ignore.
    assert!(monitor.set_focus_order(vec![b, a, c]));
    monitor.selected = Some(c);
    monitor
        .per_tag_state()
        .layout_tree
        .apply_preset(Preset::MasterStack, &screen_order, 1);
    wm.core.config.focus.horizontal_edge = HorizontalEdge::Wrap;

    // Off the right edge: the leftmost window is A, even though A is not the
    // head of the bar. A bar-order cycle would have landed on B.
    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);
    assert_eq!(wm.core.model.selected_win(), Some(a));

    // Off the left edge: symmetrically, the rightmost window is C.
    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Left);
    assert_eq!(wm.core.model.selected_win(), Some(c));
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag1
    );
}

#[test]
fn horizontal_focus_wrap_does_not_change_tags() {
    // A lone window on the middle tag: wrapping has nowhere to go, and
    // because tag 2 is not an edge, a stray overflow would have moved the
    // view to tag 3 and made this fail.
    let mut wm = tiled_row_wm(&[WindowId(1)], WindowId(1));
    let tag2 = TagMask::single(2).unwrap();
    let monitor_id = wm.core.model.selected_monitor_id();
    wm.core
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .set_selected_tags(tag2);
    wm.core
        .model
        .client_mut(WindowId(1))
        .unwrap()
        .update_tag_mask(|tags| tags | tag2);
    wm.core.config.focus.horizontal_edge = HorizontalEdge::Wrap;

    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);
    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Left);

    assert_eq!(wm.core.model.selected_win(), Some(WindowId(1)));
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag2
    );
}

#[test]
fn horizontal_focus_wrap_cannot_reach_a_window_on_another_tag() {
    let mut wm = tiled_row_wm(&[WindowId(1), WindowId(2)], WindowId(2));
    wm.core.config.focus.horizontal_edge = HorizontalEdge::Wrap;
    let monitor_id = wm.core.model.selected_monitor_id();
    // Park a window on the next tag, positioned further left than anything
    // on this one. If the tag filter were dropped it would win the wrap, so
    // this pins down that the wrap is both tag-local and geometric.
    add_client(
        &mut wm.core.model,
        monitor_id,
        Client {
            win: WindowId(3),
            tags: TagMask::single(2).unwrap(),
            mode: ClientMode::tiled(),
            geo: Rect::new(-400, 0, 400, 800),
            ..Client::default()
        },
    );
    // Keep the off-tag window after the two visible windows in focus order.
    assert!(
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_focus_order(vec![WindowId(1), WindowId(2), WindowId(3)])
    );

    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);
    assert_eq!(wm.core.model.selected_win(), Some(WindowId(1)));

    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Left);
    assert_eq!(wm.core.model.selected_win(), Some(WindowId(2)));
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        TagMask::single(1).unwrap()
    );
}

#[test]
fn horizontal_focus_stays_put_at_the_edge_when_configured() {
    let windows = [WindowId(1), WindowId(2)];
    let mut wm = tiled_row_wm(&windows, WindowId(2));
    wm.core.config.focus.horizontal_edge = HorizontalEdge::None;
    let tag1 = TagMask::single(1).unwrap();

    focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

    assert_eq!(wm.core.model.selected_win(), Some(WindowId(2)));
    assert_eq!(
        wm.core.model.expect_selected_monitor().selected_tags(),
        tag1
    );
}

#[test]
fn horizontal_focus_still_moves_to_a_neighbour_before_the_edge() {
    // The policy is only consulted at the boundary, so an ordinary step
    // between two windows must behave identically in every mode.
    for edge in [
        HorizontalEdge::Overflow,
        HorizontalEdge::Wrap,
        HorizontalEdge::None,
    ] {
        let windows = [WindowId(1), WindowId(2)];
        let mut wm = tiled_row_wm(&windows, WindowId(1));
        wm.core.config.focus.horizontal_edge = edge;
        let tag1 = TagMask::single(1).unwrap();

        focus_horizontal(&mut wm.ctx(), HorizontalDirection::Right);

        assert_eq!(wm.core.model.selected_win(), Some(WindowId(2)));
        assert_eq!(
            wm.core.model.expect_selected_monitor().selected_tags(),
            tag1
        );
    }
}

#[test]
fn maximized_horizontal_focus_follows_the_configured_edge_policy() {
    let windows = [WindowId(1), WindowId(2), WindowId(3), WindowId(4)];

    // Maximized presentation cycles in bar order, so the last window is the
    // boundary the policy has to resolve.
    let mut wrap = maximized_tiled_wm(&windows, WindowId(4));
    wrap.core.config.focus.horizontal_edge = HorizontalEdge::Wrap;
    focus_horizontal(&mut wrap.ctx(), HorizontalDirection::Right);
    assert_eq!(wrap.core.model.selected_win(), Some(WindowId(1)));

    let mut overflow = maximized_tiled_wm(&windows, WindowId(4));
    overflow.core.config.focus.horizontal_edge = HorizontalEdge::Overflow;
    focus_horizontal(&mut overflow.ctx(), HorizontalDirection::Right);
    assert_eq!(
        overflow
            .core
            .model
            .expect_selected_monitor()
            .selected_tags(),
        TagMask::single(2).unwrap()
    );

    let mut none = maximized_tiled_wm(&windows, WindowId(4));
    none.core.config.focus.horizontal_edge = HorizontalEdge::None;
    focus_horizontal(&mut none.ctx(), HorizontalDirection::Right);
    assert_eq!(none.core.model.selected_win(), Some(WindowId(4)));
}
