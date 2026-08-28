use super::{
    BackendRefresh, FocusBackendOps, FocusProjection, focus_generic as focus_generic_impl,
    get_visible_stack, stack_focus_target,
};
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::contexts::CoreCtx;
use crate::core_state::{CoreState, PendingWork};
use crate::types::{Client, Monitor, MonitorId, StackDirection, TagMask, WindowId};
use std::cell::Cell;

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

fn focus_generic(
    core: &mut CoreCtx<'_>,
    win: Option<WindowId>,
    backend: &mut dyn FocusBackendOps,
    refresh: BackendRefresh,
) -> anyhow::Result<Option<MonitorId>> {
    let previous = core.model().selected_win();
    focus_generic_impl(core, win, previous, backend, refresh)
}

fn core_with_selected_client() -> (CoreState, PendingWork, bool, BarState, FocusState) {
    let mut state = CoreState::default();
    let monitor_id = state.model.monitors.push(Monitor::default());
    let win = WindowId(1);
    let tag = TagMask::single(1).unwrap();
    state.model.insert_client(Client {
        win,
        monitor_id,
        tags: tag,
        ..Client::default()
    });
    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.set_selected_tags(tag);
    monitor.z_order.attach_top(win);
    monitor.selected = Some(win);
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
    let mut backend = RecordingBackend::default();

    focus_generic(&mut core, None, &mut backend, BackendRefresh::IfNeeded).unwrap();
    assert_eq!(backend.focused.get(), 0);
    assert_eq!(backend.binding_refreshes.get(), 0);

    focus_generic(&mut core, None, &mut backend, BackendRefresh::Force).unwrap();
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
    let mut backend = RecordingBackend::default();

    focus_generic_impl(
        &mut core,
        None,
        Some(actual_previous_focus),
        &mut backend,
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
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags: tag,
            ..Client::default()
        });
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tag);
        monitor.clients.push(win);
        monitor.z_order.attach_top(win);
        monitor.set_selected(Some(win));
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
fn changing_focus_does_not_change_persistent_z_order() {
    let (mut state, mut work, mut running, mut bar, mut focus) = core_with_selected_client();
    let monitor_id = state.model.selected_monitor_id();
    let tag = TagMask::single(1).unwrap();
    let upper = WindowId(2);
    state.model.insert_client(Client {
        win: upper,
        monitor_id,
        tags: tag,
        ..Client::default()
    });
    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.z_order.attach_top(upper);
    monitor.selected = Some(upper);

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let mut backend = RecordingBackend::default();
    focus_generic(
        &mut core,
        Some(WindowId(1)),
        &mut backend,
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
    state
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .clients
        .push(previously_focused);

    for (win, floating) in [(newer_tiled, false), (popup, true)] {
        let mut client = Client {
            win,
            monitor_id,
            tags: tag,
            ..Client::default()
        };
        if floating {
            client.set_placement(crate::types::ClientPlacement::Floating);
            client.transient_for = Some(previously_focused);
        }
        assert!(state.model.insert_client(client));
        assert!(state.model.attach_client(win));
    }

    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
    monitor.selected = Some(previously_focused);
    monitor.record_focus(tag, previously_focused);

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let mut backend = RecordingBackend::default();
    focus_generic(
        &mut core,
        Some(popup),
        &mut backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    assert_eq!(core.model().selected_win(), Some(popup));

    core.mutate_selection(|model| model.remove_client(popup))
        .unwrap();
    focus_generic(&mut core, None, &mut backend, BackendRefresh::Force).unwrap();

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
    state
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .clients
        .push(previously_focused);

    for win in [other_group_window, temporary_terminal] {
        assert!(state.model.insert_client(Client {
            win,
            monitor_id,
            tags: tag,
            ..Client::default()
        }));
        assert!(state.model.attach_client(win));
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
    let mut backend = RecordingBackend::default();

    // Establish A as the maximized window visible immediately before the
    // short-lived terminal takes focus.
    focus_generic(
        &mut core,
        Some(previously_focused),
        &mut backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    focus_generic(
        &mut core,
        Some(temporary_terminal),
        &mut backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    assert_eq!(core.model().selected_win(), Some(temporary_terminal));

    core.mutate_selection(|model| model.remove_client(temporary_terminal))
        .unwrap();
    focus_generic(&mut core, None, &mut backend, BackendRefresh::Force).unwrap();

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
    state
        .model
        .monitor_mut(monitor_id)
        .unwrap()
        .clients
        .push(previously_focused);

    for win in std::iter::once(other_group_window).chain(terminals) {
        assert!(state.model.insert_client(Client {
            win,
            monitor_id,
            tags: tag,
            ..Client::default()
        }));
        assert!(state.model.attach_client(win));
    }

    let monitor = state.model.monitor_mut(monitor_id).unwrap();
    monitor.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
    monitor.selected = Some(previously_focused);

    let mut core = CoreCtx::new(&mut state, &mut work, &mut running, &mut bar, &mut focus);
    let mut backend = RecordingBackend::default();
    focus_generic(
        &mut core,
        Some(previously_focused),
        &mut backend,
        BackendRefresh::IfNeeded,
    )
    .unwrap();
    for terminal in terminals {
        focus_generic(
            &mut core,
            Some(terminal),
            &mut backend,
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
        focus_generic(&mut core, None, &mut backend, BackendRefresh::Force).unwrap();
        assert_eq!(
            core.model().selected_win(),
            Some(expected),
            "closing {closed:?} should restore the preceding MRU client"
        );
    }
}

#[test]
fn maximized_stack_uses_tree_order_and_excludes_floating_clients() {
    let tag = TagMask::single(1).unwrap();
    let mut monitor = Monitor::default();
    monitor.set_selected_tags(tag);
    monitor.clients = vec![WindowId(1), WindowId(2), WindowId(3)];
    monitor.per_tag_state().layout_tree.apply_preset(
        crate::layouts::tree::Preset::MasterStack,
        &[WindowId(3), WindowId(1), WindowId(2)],
        1,
    );
    monitor.per_tag_state().presentation = crate::layouts::PresentationMode::Maximized;
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
            (win, client)
        })
        .collect();

    let cycle_order = get_visible_stack(&monitor, &clients);
    let bar_order = monitor.bar_client_order(&clients);
    assert_eq!(cycle_order, vec![WindowId(3), WindowId(1)]);
    assert_eq!(&bar_order[..cycle_order.len()], cycle_order);
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
