use super::{
    BackendRefresh, FocusBackendOps, FocusProjection, apply_focus_transition, stack_focus_target,
};
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::contexts::CoreCtx;
use crate::core_state::{CoreState, PendingWork};
use crate::test_support::{add_client, add_selected_client, push_monitor};
use crate::types::{Client, Monitor, MonitorId, StackDirection, TagMask, WindowId};
use std::cell::Cell;

/// Build a monitor whose focus stack is exactly `stack` and which owns
/// `clients`.
///
/// The order is assigned directly rather than arriving through
/// [`crate::model::WmModel::add_client`], which prepends to the stack. The
/// navigation tests below deliberately make the focus order disagree with
/// screen geometry, and that disagreement is the thing under test — a helper
/// that imposed insertion order would erase it.
fn monitor_with_stack(
    stack: &[WindowId],
    clients: impl IntoIterator<Item = (WindowId, Client)>,
) -> Monitor {
    Monitor {
        stack: stack.to_vec(),
        clients: clients.into_iter().collect(),
        ..Monitor::default()
    }
}

#[test]
fn directional_focus_prefers_the_aligned_client() {
    let source = WindowId(1);
    let aligned = WindowId(2);
    let diagonal = WindowId(3);
    let tags = TagMask::single(1).unwrap();
    let client = |win: WindowId, geo: crate::types::Rect| {
        (
            win,
            Client {
                win,
                tags,
                geo,
                ..Client::default()
            },
        )
    };
    let monitor = monitor_with_stack(
        &[source, diagonal, aligned],
        [
            client(source, crate::types::Rect::new(0, 0, 100, 100)),
            client(aligned, crate::types::Rect::new(120, 0, 100, 100)),
            client(diagonal, crate::types::Rect::new(100, 300, 100, 100)),
        ],
    );
    assert_eq!(
        super::get_directional_candidate(
            &monitor,
            tags,
            source,
            crate::types::Point::new(50, 50),
            crate::types::Direction::Right,
        ),
        Some(aligned)
    );
}

#[test]
fn wrapping_focus_lands_on_the_far_edge_not_the_bar_order() {
    // Three windows in a row, but listed by the monitor as B, A, C so that a
    // bar-order cycle answers B where the geometry answers A.
    let (a, b, c) = (WindowId(1), WindowId(2), WindowId(3));
    let tags = TagMask::single(1).unwrap();
    let client = |win: WindowId, x: i32| {
        (
            win,
            Client {
                win,
                tags,
                geo: crate::types::Rect::new(x, 0, 400, 800),
                ..Client::default()
            },
        )
    };
    let bar_order = [b, a, c];
    let monitor = monitor_with_stack(&bar_order, [client(a, 0), client(b, 400), client(c, 800)]);

    // Off the right edge: the leftmost window is A.
    assert_eq!(
        super::get_wrapping_window(
            &monitor,
            tags,
            c,
            crate::types::Point::new(1000, 400),
            crate::types::Direction::Right,
        ),
        Some(a)
    );
    // Off the left edge: the rightmost window is C.
    assert_eq!(
        super::get_wrapping_window(
            &monitor,
            tags,
            a,
            crate::types::Point::new(200, 400),
            crate::types::Direction::Left,
        ),
        Some(c)
    );
}

#[test]
fn wrapping_focus_refuses_a_degenerate_axis() {
    // One column: every window shares an x, so a horizontal wrap has nowhere
    // to go and must not quietly become a vertical move.
    let column = [WindowId(1), WindowId(2)];
    let tags = TagMask::single(1).unwrap();
    let column_monitor = monitor_with_stack(
        &column,
        [column[0], column[1]].map(|win| {
            (
                win,
                Client {
                    win,
                    tags,
                    geo: crate::types::Rect::new(0, win.0 as i32 * 400, 1200, 400),
                    ..Client::default()
                },
            )
        }),
    );
    let column_source = crate::types::Point::new(600, 200);

    for direction in [
        crate::types::Direction::Left,
        crate::types::Direction::Right,
    ] {
        assert_eq!(
            super::get_wrapping_window(&column_monitor, tags, column[0], column_source, direction),
            None
        );
    }

    // One row: every window shares a y, so a vertical wrap has nowhere to go
    // and must not quietly become a horizontal move.
    let row = [WindowId(1), WindowId(2)];
    let row_monitor = monitor_with_stack(
        &row,
        [row[0], row[1]].map(|win| {
            (
                win,
                Client {
                    win,
                    tags,
                    geo: crate::types::Rect::new(win.0 as i32 * 400, 0, 400, 800),
                    ..Client::default()
                },
            )
        }),
    );
    let row_source = crate::types::Point::new(200, 400);

    for direction in [crate::types::Direction::Up, crate::types::Direction::Down] {
        assert_eq!(
            super::get_wrapping_window(&row_monitor, tags, row[0], row_source, direction),
            None
        );
    }
}

#[test]
fn vertical_wrapping_focus_lands_on_the_opposite_edge() {
    // Three windows stacked down the screen. There is no window above A, so
    // running off the top edge has to land on the bottom-most one — and the
    // answer must come from geometry, not from bar order.
    let (top, middle, bottom) = (WindowId(1), WindowId(2), WindowId(3));
    let tags = TagMask::single(1).unwrap();
    // Bar order deliberately disagrees with the screen, which reads
    // top, middle, bottom.
    let bar_order = [middle, top, bottom];
    let monitor = monitor_with_stack(
        &bar_order,
        [(top, 0), (middle, 400), (bottom, 800)].map(|(win, y)| {
            (
                win,
                Client {
                    win,
                    tags,
                    geo: crate::types::Rect::new(0, y, 400, 400),
                    ..Client::default()
                },
            )
        }),
    );

    // Off the top edge: the bottom-most window.
    assert_eq!(
        super::get_wrapping_window(
            &monitor,
            tags,
            top,
            crate::types::Point::new(200, 200),
            crate::types::Direction::Up,
        ),
        Some(bottom)
    );
    // Off the bottom edge: the topmost one.
    assert_eq!(
        super::get_wrapping_window(
            &monitor,
            tags,
            bottom,
            crate::types::Point::new(200, 1000),
            crate::types::Direction::Down,
        ),
        Some(top)
    );
}

#[test]
fn wrapping_focus_skips_windows_on_other_tags() {
    let (visible, hidden) = (WindowId(1), WindowId(2));
    let tags = TagMask::single(1).unwrap();
    let other = TagMask::single(2).unwrap();
    let monitor = monitor_with_stack(
        &[visible, hidden],
        [
            (
                visible,
                Client {
                    win: visible,
                    tags,
                    geo: crate::types::Rect::new(400, 0, 400, 800),
                    ..Client::default()
                },
            ),
            (
                hidden,
                Client {
                    win: hidden,
                    tags: other,
                    // Further left than the visible window, so ignoring the tag
                    // filter would pick it.
                    geo: crate::types::Rect::new(0, 0, 400, 800),
                    ..Client::default()
                },
            ),
        ],
    );

    assert_eq!(
        super::get_wrapping_window(
            &monitor,
            tags,
            visible,
            crate::types::Point::new(600, 400),
            crate::types::Direction::Right,
        ),
        None
    );
}

/// Records what a focus transition projected into the backend.
///
/// The trait is implemented over `&self`, so the counters need interior
/// mutability to observe a call that already happened. The recording handle
/// itself does not need to be held mutably by the caller.
#[derive(Default)]
struct RecordingBackend {
    focused: Cell<usize>,
    binding_refreshes: Cell<usize>,
    previous: Cell<Option<WindowId>>,
    current: Cell<Option<WindowId>>,
}

impl FocusBackendOps for RecordingBackend {
    fn project_focus(&self, _: &mut CoreCtx<'_>, projection: FocusProjection) {
        self.focused.set(self.focused.get() + 1);
        self.previous.set(projection.previous);
        self.current.set(projection.current);
    }
    fn on_desktop_binding_state_changed(&self, _: &CoreState) {
        self.binding_refreshes.set(self.binding_refreshes.get() + 1);
    }
}

/// Run a focus transition using the model's current selection as the previous
/// backend focus, which is what [`super::focus`] does for `WmCtx` holders.
fn focus_from_current_selection(
    core: &mut CoreCtx<'_>,
    win: Option<WindowId>,
    backend: &dyn FocusBackendOps,
    refresh: BackendRefresh,
) -> anyhow::Result<Option<MonitorId>> {
    let previous = core.model().selected_win();
    Ok(apply_focus_transition(
        core, win, previous, backend, refresh,
    ))
}

fn core_with_selected_client() -> (CoreState, PendingWork, bool, BarState, FocusState) {
    let mut state = CoreState::default();
    let monitor_id = push_monitor(&mut state.model);
    let win = WindowId(1);
    let tag = TagMask::single(1).unwrap();
    add_selected_client(
        &mut state.model,
        monitor_id,
        Client {
            win,
            tags: tag,
            ..Client::default()
        },
    );
    state
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .set_selected_tags(tag);
    (
        state,
        PendingWork::default(),
        true,
        BarState::default(),
        FocusState::default(),
    )
}

#[test]
fn forced_refresh_reapplies_unchanged_backend_focus() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let backend = RecordingBackend::default();

    focus_from_current_selection(&mut core, None, &backend, BackendRefresh::IfNeeded).unwrap();
    assert_eq!(backend.focused.get(), 0);
    assert_eq!(backend.binding_refreshes.get(), 0);

    focus_from_current_selection(&mut core, None, &backend, BackendRefresh::Force).unwrap();
    assert_eq!(backend.focused.get(), 1);
    assert_eq!(backend.binding_refreshes.get(), 1);
    assert_eq!(core.focus.take_pending_selection(), None);
    assert_eq!(backend.previous.get(), Some(WindowId(1)));
    assert_eq!(backend.current.get(), Some(WindowId(1)));
}

#[test]
fn projection_uses_focus_from_before_a_precommitted_model_change() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let actual_previous_focus = WindowId(99);
    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let backend = RecordingBackend::default();

    apply_focus_transition(
        &mut core,
        None,
        Some(actual_previous_focus),
        &backend,
        BackendRefresh::Force,
    )
    .unwrap();

    assert_eq!(core.model().selected_win(), Some(WindowId(1)));
    assert_eq!(backend.previous.get(), Some(actual_previous_focus));
    assert_eq!(backend.current.get(), Some(WindowId(1)));
    assert_eq!(core.focus.take_pending_selection(), None);
}

#[test]
fn monitor_switch_records_the_global_window_transition() {
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::wm::Wm;

    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let tag = TagMask::single(1).unwrap();
    let first = WindowId(1);
    let second = WindowId(2);
    let first_monitor = wm.core.model.monitors.push(Monitor::default());
    let second_monitor = wm.core.model.monitors.push(Monitor::default());
    for (monitor_id, win) in [(first_monitor, first), (second_monitor, second)] {
        add_selected_client(
            &mut wm.core.model,
            monitor_id,
            Client {
                win,
                tags: tag,
                ..Client::default()
            },
        );
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected_tags(tag);
    }
    wm.core.model.monitors.set_selected(first_monitor);

    assert!(super::select_monitor(&mut wm.ctx(), second_monitor));
    assert_eq!(
        wm.focus.take_pending_selection(),
        Some(crate::client::focus::SelectionTransition {
            previous: Some(first),
            current: Some(second),
        })
    );
}

#[test]
fn missing_monitor_is_rejected_before_selection_changes() {
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::wm::Wm;

    let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
    let selected = wm.core.model.monitors.push(Monitor::default());
    let missing = MonitorId::from_raw(999);

    assert!(!super::select_monitor(&mut wm.ctx(), missing));
    assert_eq!(wm.core.model.selected_monitor_id(), selected);
    assert_eq!(wm.focus.take_pending_selection(), None);
}

#[test]
fn changing_focus_does_not_change_persistent_z_order() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let monitor_id = state.model.selected_monitor_id();
    let tag = TagMask::single(1).unwrap();
    let upper = WindowId(2);
    add_selected_client(
        &mut state.model,
        monitor_id,
        Client {
            win: upper,
            tags: tag,
            ..Client::default()
        },
    );

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let backend = RecordingBackend::default();
    focus_from_current_selection(
        &mut core,
        Some(WindowId(1)),
        &backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();

    assert_eq!(
        core.focus.take_pending_selection(),
        Some(crate::client::focus::SelectionTransition {
            previous: Some(upper),
            current: Some(WindowId(1)),
        })
    );
    assert_eq!(backend.previous.get(), Some(upper));
    assert_eq!(backend.current.get(), Some(WindowId(1)));

    assert_eq!(
        core.model().expect_selected_monitor().z_order.as_slice(),
        &[WindowId(1), WindowId(2)]
    );
}

#[test]
fn closing_floating_window_in_maximized_presentation_restores_tiled_focus() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let monitor_id = state.model.selected_monitor_id();
    let tag = TagMask::single(1).unwrap();
    let previously_focused = WindowId(1);
    let newer_tiled = WindowId(2);
    let popup = WindowId(3);

    for (win, floating) in [(newer_tiled, false), (popup, true)] {
        let mut client = Client {
            win,
            tags: tag,
            ..Client::default()
        };
        if floating {
            client.set_placement(crate::types::ClientPlacement::Floating);
            client.transient_for = Some(previously_focused);
        }
        add_client(&mut state.model, monitor_id, client);
    }

    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
    monitor.selected = Some(previously_focused);
    monitor.record_focus(tag, previously_focused);

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let backend = RecordingBackend::default();
    focus_from_current_selection(&mut core, Some(popup), &backend, BackendRefresh::IfNeeded)
        .unwrap();
    assert_eq!(core.model().selected_win(), Some(popup));

    core.mutate_selection(|model| model.remove_client(popup))
        .unwrap();
    focus_from_current_selection(&mut core, None, &backend, BackendRefresh::Force).unwrap();

    assert_eq!(
        core.model().selected_win(),
        Some(previously_focused),
        "the tiled window visible beneath the popup should regain focus"
    );
}

#[test]
fn closing_temporary_tiled_window_in_maximized_presentation_restores_previous_focus() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let monitor_id = state.model.selected_monitor_id();
    let tag = TagMask::single(1).unwrap();
    let previously_focused = WindowId(1);
    let other_group_window = WindowId(2);
    let temporary_terminal = WindowId(3);

    for win in [other_group_window, temporary_terminal] {
        add_client(
            &mut state.model,
            monitor_id,
            Client {
                win,
                tags: tag,
                ..Client::default()
            },
        );
    }

    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[previously_focused, other_group_window, temporary_terminal],
        1,
    );
    monitor.selected = Some(previously_focused);

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let backend = RecordingBackend::default();

    // Establish A as the maximized window visible immediately before the
    // short-lived terminal takes focus.
    focus_from_current_selection(
        &mut core,
        Some(previously_focused),
        &backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    focus_from_current_selection(
        &mut core,
        Some(temporary_terminal),
        &backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    assert_eq!(core.model().selected_win(), Some(temporary_terminal));

    core.mutate_selection(|model| model.remove_client(temporary_terminal))
        .unwrap();
    focus_from_current_selection(&mut core, None, &backend, BackendRefresh::Force).unwrap();

    assert_eq!(
        core.model().selected_win(),
        Some(previously_focused),
        "closing a short-lived tiled window should reveal the maximized window that preceded it"
    );
}

#[test]
fn closing_repeated_temporary_tiled_windows_unwinds_focus_in_mru_order() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let monitor_id = state.model.selected_monitor_id();
    let tag = TagMask::single(1).unwrap();
    let previously_focused = WindowId(1);
    let other_group_window = WindowId(2);
    let terminals = [WindowId(3), WindowId(4), WindowId(5)];

    for win in std::iter::once(other_group_window).chain(terminals) {
        add_client(
            &mut state.model,
            monitor_id,
            Client {
                win,
                tags: tag,
                ..Client::default()
            },
        );
    }

    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
    monitor.selected = Some(previously_focused);

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let backend = RecordingBackend::default();
    focus_from_current_selection(
        &mut core,
        Some(previously_focused),
        &backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    for terminal in terminals {
        focus_from_current_selection(
            &mut core,
            Some(terminal),
            &backend,
            BackendRefresh::IfNeeded,
        )
        .unwrap();
    }

    for (closed, expected) in [
        (WindowId(5), WindowId(4)),
        (WindowId(4), WindowId(3)),
        (WindowId(3), previously_focused),
    ] {
        core.mutate_selection(|model| model.remove_client(closed))
            .unwrap();
        focus_from_current_selection(&mut core, None, &backend, BackendRefresh::Force).unwrap();
        assert_eq!(
            core.model().selected_win(),
            Some(expected),
            "closing {closed:?} should restore the preceding MRU client"
        );
    }
}

#[test]
fn bounded_stack_navigation_follows_order_and_stops_at_outer_edges() {
    let order = [WindowId(3), WindowId(1), WindowId(4)];

    assert_eq!(
        stack_focus_target(&order, Some(WindowId(1)), StackDirection::Previous, false,),
        Some(WindowId(3))
    );
    assert_eq!(
        stack_focus_target(&order, Some(WindowId(1)), StackDirection::Next, false,),
        Some(WindowId(4))
    );
    assert_eq!(
        stack_focus_target(&order, Some(WindowId(3)), StackDirection::Previous, false,),
        None
    );
    assert_eq!(
        stack_focus_target(&order, Some(WindowId(4)), StackDirection::Next, false,),
        None
    );
    assert_eq!(
        stack_focus_target(&order, Some(WindowId(3)), StackDirection::Previous, true,),
        Some(WindowId(4))
    );
    assert_eq!(
        stack_focus_target(
            &[WindowId(1)],
            Some(WindowId(1)),
            StackDirection::Next,
            false,
        ),
        None
    );
}
