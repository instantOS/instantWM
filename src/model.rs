//! Window manager authoritative model state.
//!
//! `WmModel` owns the core client/monitor/tag graph that represents the
//! window manager's authoritative state.  This graph is backend-neutral
//! and can be tested without constructing a backend.

use crate::monitor::MonitorManager;
use crate::types::{Client, Monitor, MonitorId, Rect, SnapPosition, TagSet, WindowId};

/// A managed client together with the monitor that owns it.
///
/// The fields are intentionally public within the crate: this view resolves
/// the model relationship once, while callers remain free to select exactly
/// the state they need without a matrix of projection helpers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientView<'a> {
    pub client: &'a Client,
    pub monitor: &'a Monitor,
}

/// Authoritative window-manager model state.
///
/// Clients, monitors, and tags form a cross-referenced graph and are
/// kept together so their invariants have a single owner.
///
/// Each [`Monitor`] owns the clients assigned to it. That ownership is the
/// single source of truth for the client/monitor relationship, so a client can
/// never name a monitor other than the one holding it and no assignment can go
/// stale. Monitor count is small and `MonitorManager` already resolves monitors
/// by linear scan. A global client lookup costs one hash lookup per monitor in
/// the worst case; monitor-local operations need only their owner's lookup.
pub struct WmModel {
    /// All monitors/screens, each owning its clients.
    pub(crate) monitors: MonitorManager,
    /// Shared tag metadata.
    pub(crate) tags: TagSet,
}

impl WmModel {
    pub fn new() -> Self {
        Self {
            monitors: MonitorManager::new(),
            tags: TagSet::default(),
        }
    }

    // -------------------------------------------------------------------------
    // Client lookup
    // -------------------------------------------------------------------------

    /// Return a managed client by window ID.
    pub fn client(&self, win: WindowId) -> Option<&Client> {
        self.monitors
            .iter_all()
            .find_map(|monitor| monitor.clients().get(&win))
    }

    /// Return a managed client mutably by window ID.
    pub fn client_mut(&mut self, win: WindowId) -> Option<&mut Client> {
        self.monitors
            .iter_all_mut()
            .find_map(|monitor| monitor.client_mut(win))
    }

    /// Return the monitor that owns `win`.
    pub fn monitor_of_client(&self, win: WindowId) -> Option<MonitorId> {
        self.monitors
            .iter_all()
            .find(|monitor| monitor.has_client(win))
            .map(|monitor| monitor.id())
    }

    /// Iterate every managed client together with its owning monitor's ID.
    pub fn clients_iter_all(&self) -> impl Iterator<Item = (MonitorId, &Client)> {
        self.monitors.iter().flat_map(|(monitor_id, monitor)| {
            monitor
                .clients()
                .values()
                .map(move |client| (monitor_id, client))
        })
    }

    /// Count every managed client across all monitors.
    pub fn client_count(&self) -> usize {
        self.monitors
            .iter_all()
            .map(|monitor| monitor.clients().len())
            .sum()
    }

    /// Synchronize a client's authoritative geometry and floating placement.
    ///
    /// Backends call this after the WM knows the rectangle that actually
    /// applies. A normal floating client records that rectangle as its saved
    /// placement unless it is snapped; other client modes only update current
    /// geometry.
    pub fn sync_client_geometry(&mut self, win: WindowId, rect: Rect) {
        let work_area = self.client_view(win).map(|view| view.monitor.work_rect());
        if let Some(client) = self.client_mut(win) {
            client.update_geometry(rect);
            if client.mode().is_normal_floating()
                && client.snap_status == SnapPosition::None
                && let Some(work_area) = work_area
            {
                client.save_floating_placement(rect, work_area);
            }
        }
    }

    /// Add a client to `monitor_id`, adopting it into every monitor-owned
    /// collection at once.
    ///
    /// This is the only way a client enters the model: there is no detached
    /// state in which a client exists without an owner. Returns `false` when
    /// the monitor is unknown or the window is already managed.
    pub(crate) fn add_client(&mut self, monitor_id: MonitorId, client: Client) -> bool {
        self.adopt_client(monitor_id, client, false)
    }

    /// Re-home a client that previously held another monitor's selection,
    /// restoring that selection on its new monitor.
    ///
    /// Output removal uses this so unplugging a monitor does not silently drop
    /// which of its windows the user was focused on.
    pub(crate) fn readopt_client(
        &mut self,
        monitor_id: MonitorId,
        client: Client,
        was_selected: bool,
    ) -> bool {
        self.adopt_client(monitor_id, client, was_selected)
    }

    fn adopt_client(&mut self, monitor_id: MonitorId, client: Client, selected: bool) -> bool {
        let win = client.win;
        if self.client(win).is_some() || self.monitor(monitor_id).is_none() {
            return false;
        }
        self.attach_client(monitor_id, client, selected);
        self.debug_assert_client_graph();
        true
    }

    /// Remove a managed client and every monitor-owned reference to it.
    ///
    /// Backend teardown must happen before this call when it needs client
    /// metadata. Once this returns, the model cannot contain a partial client.
    pub(crate) fn remove_client(&mut self, win: WindowId) -> Option<Client> {
        let (client, _) = self.detach_client(win)?;
        self.debug_assert_client_graph();
        Some(client)
    }

    /// Take `win` out of its owning monitor, returning the client and whether
    /// it held that monitor's selection.
    ///
    /// The client leaves the model entirely: callers re-home it through
    /// [`Self::readopt_client`], hand it to a composed transaction such as
    /// [`Self::move_client_to_monitor`], or drop it.
    fn detach_client(&mut self, win: WindowId) -> Option<(Client, bool)> {
        let monitor_id = self.monitor_of_client(win)?;
        let monitor = self.monitors.get_mut(monitor_id)?;
        monitor.take_client(win)
    }

    /// Place an owned client into `monitor_id`, rebuilding its monitor-owned
    /// references and restoring selection when the client held it.
    fn attach_client(&mut self, monitor_id: MonitorId, client: Client, selected: bool) {
        let monitor = self
            .monitors
            .get_mut(monitor_id)
            .expect("validated target monitor must still exist");
        monitor.adopt_client(client, selected);
    }

    /// Resolve a managed client and its owning monitor as one coherent view.
    ///
    /// Returns `None` only when the client is unknown: a managed client always
    /// has an owner, so this cannot fail for a stale assignment.
    pub(crate) fn client_view(&self, win: WindowId) -> Option<ClientView<'_>> {
        for monitor in self.monitors.iter_all() {
            if let Some(client) = monitor.clients().get(&win) {
                return Some(ClientView { client, monitor });
            }
        }
        None
    }

    // -------------------------------------------------------------------------
    // Selected-monitor convenience helpers
    // -------------------------------------------------------------------------

    /// Return the window currently selected on the selected monitor, if any.
    #[inline]
    pub fn selected_win(&self) -> Option<WindowId> {
        self.monitors.selected_monitor().and_then(|m| m.selected)
    }

    /// Return the ID of the currently selected monitor.
    pub fn selected_monitor_id(&self) -> MonitorId {
        self.monitors.selected()
    }

    /// Whether `monitor_id` identifies a connected monitor other than the
    /// currently selected one.
    ///
    /// Callers use this before performing selection side effects, while
    /// [`Self::set_selected_monitor`] provides the guarded mutation primitive.
    pub(crate) fn can_change_selected_monitor(&self, monitor_id: MonitorId) -> bool {
        self.monitor(monitor_id).is_some() && self.selected_monitor_id() != monitor_id
    }

    /// Change the currently selected monitor.
    pub fn set_selected_monitor(&mut self, id: MonitorId) {
        self.monitors.set_selected(id);
    }

    /// Get the selected monitor, if outputs have been initialized.
    pub fn selected_monitor(&self) -> Option<&crate::types::Monitor> {
        self.monitors.selected_monitor()
    }

    /// Get the selected monitor when the caller's lifecycle guarantees one.
    pub fn expect_selected_monitor(&self) -> &crate::types::Monitor {
        self.monitors.selected_monitor_unchecked()
    }

    /// Get the selected monitor mutably when lifecycle guarantees one.
    pub fn expect_selected_monitor_mut(&mut self) -> &mut crate::types::Monitor {
        self.monitors.selected_monitor_mut_unchecked()
    }

    /// Whether `win` belongs to the selected monitor and is visible in its
    /// current tag view. This resolves the client/monitor relationship and
    /// selected view as one model query.
    pub fn client_is_visible_on_selected_monitor(&self, win: WindowId) -> bool {
        let selected_monitor_id = self.selected_monitor_id();
        let selected_tags = self.expect_selected_monitor().visible_tags();
        self.client_view(win).is_some_and(|view| {
            view.monitor.id() == selected_monitor_id && view.client.is_visible(selected_tags)
        })
    }

    /// Shorthand to get the selected monitor mutably (Option version).
    pub fn selected_monitor_mut(&mut self) -> Option<&mut crate::types::Monitor> {
        self.monitors.selected_monitor_mut()
    }

    /// Return `true` if overview mode is active on the selected monitor.
    pub fn is_overview_active(&self) -> bool {
        self.selected_monitor()
            .is_some_and(|monitor| monitor.overview_state.is_some())
    }

    /// Return `true` if overview mode is active on the given monitor.
    pub fn is_overview_active_on(&self, monitor: &crate::types::Monitor) -> bool {
        monitor.overview_state.is_some() && self.selected_monitor_id() == monitor.id()
    }

    /// Delegation to get a monitor by index.
    pub fn monitor(&self, id: MonitorId) -> Option<&crate::types::Monitor> {
        self.monitors.get(id)
    }

    /// Delegation to get a mutable monitor by index.
    pub fn monitor_mut(&mut self, id: MonitorId) -> Option<&mut crate::types::Monitor> {
        self.monitors.get_mut(id)
    }

    /// Delegation to iterate over monitors.
    pub fn monitors_iter(&self) -> impl Iterator<Item = (MonitorId, &crate::types::Monitor)> {
        self.monitors.iter()
    }

    /// Iterate over all monitors (without index).
    pub fn monitors_iter_all(&self) -> impl Iterator<Item = &crate::types::Monitor> {
        self.monitors.iter_all()
    }

    /// Delegation to iterate over monitors mutably.
    pub fn monitors_iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (MonitorId, &mut crate::types::Monitor)> {
        self.monitors.iter_mut()
    }

    /// Iterate over all monitors mutably (without index).
    pub fn monitors_iter_all_mut(&mut self) -> impl Iterator<Item = &mut crate::types::Monitor> {
        self.monitors.iter_all_mut()
    }

    /// Find a scratchpad by name.
    pub fn scratchpad_find(&self, name: &str) -> Option<WindowId> {
        if name.is_empty() {
            return None;
        }

        self.clients_iter_all().find_map(|(_, c)| {
            c.scratchpad()
                .is_some_and(|sp| sp.name() == name)
                .then_some(c.win)
        })
    }

    // -------------------------------------------------------------------------
    // Client graph mutations
    // -------------------------------------------------------------------------

    /// Move a client to `target_monitor`, rebuilding every monitor-owned
    /// reference as one model transaction.
    ///
    /// Returns whether the client is now owned by `target_monitor`. Callers
    /// that need to know whether the client held its *source* monitor's
    /// selection — which only a composed transfer like
    /// [`Self::move_client_to_monitor`] can act on — read that from the
    /// returned [`ClientTransferOutcome`] instead.
    pub(crate) fn reassign_client_monitor(
        &mut self,
        win: WindowId,
        target_monitor: MonitorId,
    ) -> bool {
        if self.monitor(target_monitor).is_none() || self.monitor_of_client(win).is_none() {
            return false;
        }
        let Some((client, was_selected)) = self.detach_client(win) else {
            return false;
        };
        self.attach_client(target_monitor, client, was_selected);
        self.debug_assert_client_graph();
        true
    }

    #[cfg(debug_assertions)]
    fn debug_assert_client_graph(&self) {
        let mut owned = std::collections::HashSet::new();
        for monitor in self.monitors.iter_all() {
            let monitor_id = monitor.id();
            let mut stacked = std::collections::HashSet::new();
            for win in monitor.focus_order().iter().copied() {
                assert!(
                    monitor.has_client(win),
                    "monitor {monitor_id:?} focus stack references unowned {win:?}"
                );
                assert!(
                    stacked.insert(win),
                    "monitor {monitor_id:?} has duplicate focus entry {win:?}"
                );
            }
            for &win in monitor.clients().keys() {
                assert!(
                    owned.insert(win),
                    "client {win:?} is owned by multiple monitors"
                );
                assert!(
                    stacked.contains(&win),
                    "monitor {monitor_id:?} owns {win:?} outside its focus stack"
                );
            }

            let mut layered = std::collections::HashSet::new();
            for win in monitor.z_order().iter_bottom_to_top() {
                assert!(
                    monitor.has_client(win),
                    "monitor {monitor_id:?} z-order references unowned {win:?}"
                );
                assert!(
                    monitor.focus_order().contains(&win),
                    "monitor {monitor_id:?} z-order client {win:?} is absent from its focus list"
                );
                assert!(
                    layered.insert(win),
                    "monitor {monitor_id:?} has duplicate z-order entry {win:?}"
                );
            }
            assert_eq!(
                layered, stacked,
                "monitor {monitor_id:?} client orderings differ"
            );

            for (source, win) in std::iter::once(("selection", monitor.selected)).chain(
                monitor
                    .focus_history_windows()
                    .map(|win| ("focus history", Some(win))),
            ) {
                let Some(win) = win else { continue };
                assert!(
                    monitor.has_client(win),
                    "monitor {monitor_id:?} {source} references unowned {win:?}"
                );
                assert!(
                    monitor.focus_order().contains(&win),
                    "monitor {monitor_id:?} {source} client {win:?} is absent from its focus list"
                );
            }
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn debug_assert_client_graph(&self) {}

    /// Move `win` to the top of its monitor's persistent z-order.
    pub fn raise_client_in_z_order(&mut self, win: WindowId) {
        let Some(monitor_id) = self.monitor_of_client(win) else {
            return;
        };
        if let Some(monitor) = self.monitors.get_mut(monitor_id) {
            monitor.raise_client(win);
        }
    }

    /// Move a client within its monitor's focus list (stack order).
    ///
    /// Returns true if the position changed, false otherwise.
    pub fn move_client_in_stack(
        &mut self,
        win: WindowId,
        direction: crate::types::StackDirection,
    ) -> bool {
        if let Some(mon) = self.monitors.selected_monitor_mut() {
            mon.move_client_in_stack(win, direction)
        } else {
            false
        }
    }

    /// Move a client window to a target monitor in the data model.
    pub fn move_client_to_monitor(
        &mut self,
        win: WindowId,
        target_mon: MonitorId,
    ) -> Option<ClientTransferOutcome> {
        let source_monitor = self.monitor_of_client(win)?;
        if source_monitor == target_mon {
            return None;
        }
        let is_scratchpad = self.client(win)?.is_scratchpad();
        let needs_arrange = !self.client(win)?.mode().is_normal_floating();
        let target_monitor = self.monitors.get(target_mon)?;
        let target_tags = if is_scratchpad {
            crate::types::TagMask::EMPTY
        } else {
            target_monitor.selected_tags()
        };
        let target_tag_idx = target_monitor.current_tag_number();

        let (mut client, was_selected) = self.detach_client(win)?;
        if !is_scratchpad {
            client.set_tag_mask(target_tags);
            client.reset_sticky(target_tag_idx);
        }
        self.attach_client(target_mon, client, was_selected);
        self.debug_assert_client_graph();
        Some(ClientTransferOutcome {
            source_monitor,
            target_monitor: target_mon,
            was_selected,
            is_scratchpad,
            needs_arrange,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct ClientTransferOutcome {
    pub source_monitor: MonitorId,
    pub target_monitor: MonitorId,
    pub was_selected: bool,
    pub is_scratchpad: bool,
    pub needs_arrange: bool,
}

impl Default for WmModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{add_client, add_selected_client, push_monitor, push_monitor_with};
    use crate::types::{ClientMode, Rect, TagMask};

    #[test]
    fn monitor_selection_requires_a_connected_non_current_monitor() {
        let mut model = WmModel::new();
        let first = push_monitor(&mut model);
        let second = push_monitor(&mut model);
        model.set_selected_monitor(first);

        assert!(model.can_change_selected_monitor(second));
        assert!(!model.can_change_selected_monitor(first));
        assert!(!model.can_change_selected_monitor(MonitorId::from_raw(999)));
    }

    #[test]
    fn client_view_resolves_client_and_owning_monitor() {
        let mut model = WmModel::new();
        let monitor_id = push_monitor_with(&mut model, |monitor| {
            monitor.monitor_rect = Rect::new(1920, 0, 2560, 1440);
        });
        let win = add_client(
            &mut model,
            monitor_id,
            Client {
                geo: Rect::new(2000, 100, 800, 600),
                ..Client::default()
            },
        );

        let view = model.client_view(win).expect("client view");

        assert_eq!(view.client.win, win);
        assert_eq!(view.client.geo, Rect::new(2000, 100, 800, 600));
        assert_eq!(view.monitor.id(), monitor_id);
        assert_eq!(view.monitor.monitor_rect, Rect::new(1920, 0, 2560, 1440));
    }

    #[test]
    fn a_managed_client_always_has_an_owner() {
        let mut model = WmModel::new();
        let monitor_id = push_monitor(&mut model);
        let win = add_client(&mut model, monitor_id, Client::default());

        // A managed client cannot fail to resolve its monitor, so `client_view`
        // and `client` agree on existence instead of disagreeing about a stale
        // assignment.
        assert_eq!(
            model.client_view(win).is_some(),
            model.client(win).is_some()
        );
        assert_eq!(model.monitor_of_client(win), Some(monitor_id));
        assert!(model.client_view(WindowId(8)).is_none());
        assert_eq!(model.monitor_of_client(WindowId(8)), None);
    }

    #[test]
    fn adding_to_an_unknown_monitor_leaves_the_client_unmanaged() {
        let mut model = WmModel::new();
        let win = WindowId(7);

        let added = model.add_client(
            MonitorId::from_raw(999),
            Client {
                win,
                ..Client::default()
            },
        );

        assert!(!added);
        assert!(model.client(win).is_none());
        assert_eq!(model.client_count(), 0);
    }

    #[test]
    fn selected_view_visibility_is_resolved_as_one_model_query() {
        let mut model = WmModel::new();
        let visible_tags = TagMask::single(2).unwrap();
        let selected_monitor = push_monitor_with(&mut model, |monitor| {
            monitor.set_selected_tags(visible_tags);
        });
        let other_monitor = push_monitor(&mut model);
        model.monitors.set_selected(selected_monitor);

        let visible = add_client(
            &mut model,
            selected_monitor,
            Client {
                win: WindowId(1),
                tags: visible_tags,
                ..Client::default()
            },
        );
        let hidden = add_client(
            &mut model,
            selected_monitor,
            Client {
                win: WindowId(2),
                tags: TagMask::single(1).unwrap(),
                ..Client::default()
            },
        );
        let elsewhere = add_client(
            &mut model,
            other_monitor,
            Client {
                win: WindowId(3),
                tags: visible_tags,
                ..Client::default()
            },
        );

        assert!(model.client_is_visible_on_selected_monitor(visible));
        assert!(!model.client_is_visible_on_selected_monitor(hidden));
        assert!(!model.client_is_visible_on_selected_monitor(elsewhere));
    }

    #[test]
    fn invalid_transfer_target_does_not_modify_client_ownership() {
        let mut model = WmModel::new();
        let source = push_monitor(&mut model);
        let win = add_client(&mut model, source, Client::default());

        let outcome = model.move_client_to_monitor(win, MonitorId::from_raw(999));

        assert!(outcome.is_none());
        assert_eq!(model.monitor_of_client(win), Some(source));
        assert!(model.monitor(source).unwrap().has_client(win));
    }

    #[test]
    fn transfer_outcome_preserves_the_resolved_transaction_state() {
        let mut model = WmModel::new();
        let source = push_monitor(&mut model);
        let target = push_monitor(&mut model);
        model.monitors.set_selected(source);
        let mut client = Client::default();
        client.set_mode_for_test(ClientMode::tiled());
        let win = add_selected_client(&mut model, source, client);

        let outcome = model.move_client_to_monitor(win, target).unwrap();

        assert_eq!(outcome.source_monitor, source);
        assert_eq!(outcome.target_monitor, target);
        assert!(outcome.was_selected);
        assert!(outcome.needs_arrange);
        assert!(!outcome.is_scratchpad);
        assert_eq!(model.monitor_of_client(win), Some(target));
        assert!(!model.monitor(source).unwrap().has_client(win));
        assert_eq!(model.monitor(source).unwrap().selected, None);
        assert_eq!(model.monitor(target).unwrap().selected, Some(win));
    }

    #[test]
    fn transfer_to_current_monitor_is_a_noop() {
        let mut model = WmModel::new();
        let source = push_monitor(&mut model);
        let win = add_client(&mut model, source, Client::default());

        assert!(model.move_client_to_monitor(win, source).is_none());
        assert_eq!(model.monitor_of_client(win), Some(source));
        assert!(model.monitor(source).unwrap().has_client(win));
    }

    #[test]
    fn removing_client_clears_every_monitor_owned_reference() {
        let mut model = WmModel::new();
        let monitor_id = push_monitor(&mut model);
        let tags = TagMask::single(1).unwrap();
        let other = add_client(
            &mut model,
            monitor_id,
            Client {
                win: WindowId(1),
                tags,
                ..Client::default()
            },
        );
        let win = add_selected_client(
            &mut model,
            monitor_id,
            Client {
                win: WindowId(2),
                tags,
                ..Client::default()
            },
        );
        model
            .monitor_mut(monitor_id)
            .unwrap()
            .record_focus(tags, win);

        let removed = model.remove_client(win);

        assert_eq!(removed.map(|client| client.win), Some(win));
        assert!(model.client(win).is_none());
        let monitor = model.monitor(monitor_id).unwrap();
        assert!(!monitor.has_client(win));
        assert_eq!(monitor.focus_order(), &[other]);
        assert_eq!(monitor.z_order().as_slice(), &[other]);
        assert_eq!(monitor.selected, None);
        assert!(
            !monitor
                .focus_history_windows()
                .any(|candidate| candidate == win)
        );
    }

    #[test]
    fn adding_a_client_adopts_it_into_every_monitor_collection_at_once() {
        let mut model = WmModel::new();
        let monitor_id = push_monitor(&mut model);
        let win = add_client(&mut model, monitor_id, Client::default());

        let monitor = model.monitor(monitor_id).unwrap();
        assert!(monitor.has_client(win));
        assert_eq!(monitor.focus_order(), &[win]);
        assert_eq!(monitor.z_order().as_slice(), &[win]);
        assert_eq!(model.client_count(), 1);
    }

    #[test]
    fn adding_a_duplicate_client_cannot_replace_existing_state() {
        let mut model = WmModel::new();
        let monitor_id = push_monitor(&mut model);
        let win = WindowId(14);
        assert!(model.add_client(
            monitor_id,
            Client {
                win,
                name: "original".to_string(),
                ..Client::default()
            },
        ));

        assert!(!model.add_client(
            monitor_id,
            Client {
                win,
                name: "replacement".to_string(),
                ..Client::default()
            },
        ));
        assert_eq!(
            model.client(win).map(|client| client.name.as_str()),
            Some("original")
        );
    }

    #[test]
    fn reassigning_client_clears_source_references_and_attaches_target() {
        let mut model = WmModel::new();
        let source = push_monitor(&mut model);
        let target = push_monitor(&mut model);
        let tags = TagMask::single(1).unwrap();
        let win = add_selected_client(
            &mut model,
            source,
            Client {
                tags,
                ..Client::default()
            },
        );
        model.monitor_mut(source).unwrap().record_focus(tags, win);

        assert!(model.reassign_client_monitor(win, target));

        let source_monitor = model.monitor(source).unwrap();
        assert!(!source_monitor.has_client(win));
        assert!(source_monitor.focus_order().is_empty());
        assert!(source_monitor.z_order().as_slice().is_empty());
        assert_eq!(source_monitor.selected, None);
        assert!(source_monitor.focus_history_windows().next().is_none());
        let target_monitor = model.monitor(target).unwrap();
        assert!(target_monitor.has_client(win));
        assert_eq!(target_monitor.focus_order(), &[win]);
        assert_eq!(target_monitor.z_order().as_slice(), &[win]);
        assert_eq!(target_monitor.selected, Some(win));
        assert_eq!(model.monitor_of_client(win), Some(target));
    }

    #[test]
    fn clients_iter_all_reports_each_client_with_its_owner() {
        let mut model = WmModel::new();
        let first = push_monitor(&mut model);
        let second = push_monitor(&mut model);
        let on_first = add_client(
            &mut model,
            first,
            Client {
                win: WindowId(1),
                ..Client::default()
            },
        );
        let on_second = add_client(
            &mut model,
            second,
            Client {
                win: WindowId(2),
                ..Client::default()
            },
        );

        let mut owned: Vec<(WindowId, MonitorId)> = model
            .clients_iter_all()
            .map(|(monitor_id, client)| (client.win, monitor_id))
            .collect();
        owned.sort();

        assert_eq!(owned, vec![(on_first, first), (on_second, second)]);
        assert_eq!(model.client_count(), 2);
    }
}
#[test]
fn selected_monitor_query_is_empty_before_output_initialization() {
    let model = WmModel::new();

    assert!(model.selected_monitor().is_none());
}

#[test]
#[should_panic(expected = "no monitors")]
fn expect_selected_monitor_documents_the_operational_invariant() {
    let model = WmModel::new();

    let _ = model.expect_selected_monitor();
}
