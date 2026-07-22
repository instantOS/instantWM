//! Window manager authoritative model state.
//!
//! `WmModel` owns the core client/monitor/tag graph that represents the
//! window manager's authoritative state.  This graph is backend-neutral
//! and can be tested without constructing a backend.

use crate::monitor::MonitorManager;
use crate::types::{Client, Monitor, MonitorId, TagSet, WindowId};
use std::collections::HashMap;

/// A managed client together with the monitor it is assigned to.
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
pub struct WmModel {
    /// All managed clients.
    pub(crate) clients: HashMap<WindowId, Client>,
    /// All monitors/screens.
    pub(crate) monitors: MonitorManager,
    /// Shared tag metadata.
    pub(crate) tags: TagSet,
}

impl WmModel {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            monitors: MonitorManager::new(),
            tags: TagSet::default(),
        }
    }

    // -------------------------------------------------------------------------
    // Client lookup
    // -------------------------------------------------------------------------

    /// Return a managed client by window ID.
    pub fn client(&self, win: WindowId) -> Option<&Client> {
        self.clients.get(&win)
    }

    /// Return a managed client mutably by window ID.
    pub fn client_mut(&mut self, win: WindowId) -> Option<&mut Client> {
        self.clients.get_mut(&win)
    }

    /// Add a new client without allowing an existing graph node to be replaced.
    pub(crate) fn insert_client(&mut self, client: Client) -> bool {
        use std::collections::hash_map::Entry;
        match self.clients.entry(client.win) {
            Entry::Vacant(entry) => {
                entry.insert(client);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    /// Remove a managed client and every monitor-owned reference to it.
    ///
    /// Backend teardown must happen before this call when it needs client
    /// metadata. Once this returns, the model cannot contain a partial client.
    pub(crate) fn remove_client(&mut self, win: WindowId) -> Option<Client> {
        let client = self.clients.remove(&win)?;
        self.remove_monitor_references(win);
        self.debug_assert_client_graph();
        Some(client)
    }

    fn remove_monitor_references(&mut self, win: WindowId) -> bool {
        let mut was_selected = false;
        for monitor in self.monitors.iter_all_mut() {
            monitor.clients.retain(|candidate| *candidate != win);
            monitor.z_order.remove(win);
            if monitor.selected == Some(win) {
                was_selected = true;
                monitor.selected = None;
            }
            monitor
                .tag_focus_history
                .retain(|_, candidate| *candidate != win);
            monitor
                .tag_tiled_focus_history
                .retain(|_, candidate| *candidate != win);
        }
        was_selected
    }

    /// Resolve a managed client and its assigned monitor as one coherent view.
    ///
    /// Returns `None` when either the client is unknown or its monitor
    /// assignment is stale. Callers that only need client state should use
    /// [`Self::client`] so those two cases remain distinguishable.
    pub(crate) fn client_view(&self, win: WindowId) -> Option<ClientView<'_>> {
        let client = self.client(win)?;
        let monitor = self.monitor(client.monitor_id)?;
        Some(ClientView { client, monitor })
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
        let selected_tags = self.expect_selected_monitor().selected_tags();
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
        self.expect_selected_monitor().overview_state.is_some()
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

        for c in self.clients.values() {
            if c.is_scratchpad() && c.scratchpad.as_ref().is_some_and(|sp| sp.name == name) {
                return Some(c.win);
            }
        }
        None
    }

    // -------------------------------------------------------------------------
    // Client graph mutations
    // -------------------------------------------------------------------------

    /// Attach a stored client to both ordered monitor collections.
    ///
    /// Existing references are removed first, making this safe for initial
    /// adoption as well as defensive graph reconciliation.
    pub(crate) fn attach_client(&mut self, win: WindowId) -> bool {
        let Some(monitor_id) = self.client(win).map(|client| client.monitor_id) else {
            return false;
        };
        if self.monitor(monitor_id).is_none() {
            return false;
        }

        let was_selected = self.remove_monitor_references(win);
        let monitor = self
            .monitor_mut(monitor_id)
            .expect("validated client monitor must remain present");
        monitor.clients.insert(0, win);
        monitor.z_order.attach_top(win);
        if was_selected {
            monitor.selected = Some(win);
        }
        self.debug_assert_client_graph();
        true
    }

    /// Change client ownership and rebuild every monitor-owned reference as one
    /// model transaction.
    pub(crate) fn reassign_client_monitor(
        &mut self,
        win: WindowId,
        target_monitor: MonitorId,
    ) -> bool {
        if self.client(win).is_none() || self.monitor(target_monitor).is_none() {
            return false;
        }

        let was_selected = self.remove_monitor_references(win);
        self.client_mut(win)
            .expect("validated client must remain present")
            .monitor_id = target_monitor;
        let monitor = self
            .monitor_mut(target_monitor)
            .expect("validated target monitor must remain present");
        monitor.clients.insert(0, win);
        monitor.z_order.attach_top(win);
        if was_selected {
            monitor.selected = Some(win);
        }
        self.debug_assert_client_graph();
        true
    }

    #[cfg(debug_assertions)]
    fn debug_assert_client_graph(&self) {
        let mut memberships = std::collections::HashMap::<WindowId, MonitorId>::new();
        for monitor in self.monitors.iter_all() {
            let monitor_id = monitor.id();
            for win in monitor.clients.iter().copied() {
                let client = self
                    .client(win)
                    .unwrap_or_else(|| panic!("monitor {monitor_id:?} references missing {win:?}"));
                assert_eq!(
                    client.monitor_id, monitor_id,
                    "monitor {monitor_id:?} contains client {win:?} owned by {:?}",
                    client.monitor_id
                );
                assert_eq!(
                    memberships.insert(win, monitor_id),
                    None,
                    "client {win:?} belongs to multiple monitor focus lists"
                );
            }

            for win in monitor.z_order.iter_bottom_to_top() {
                let client = self.client(win).unwrap_or_else(|| {
                    panic!("monitor {monitor_id:?} z-order references missing {win:?}")
                });
                assert_eq!(
                    client.monitor_id, monitor_id,
                    "monitor {monitor_id:?} z-order contains client {win:?} owned by {:?}",
                    client.monitor_id
                );
                assert!(
                    monitor.clients.contains(&win),
                    "monitor {monitor_id:?} z-order client {win:?} is absent from its focus list"
                );
            }

            for (source, win) in std::iter::once(("selection", monitor.selected))
                .chain(
                    monitor
                        .tag_focus_history
                        .values()
                        .copied()
                        .map(|win| ("focus history", Some(win))),
                )
                .chain(
                    monitor
                        .tag_tiled_focus_history
                        .values()
                        .copied()
                        .map(|win| ("tiled focus history", Some(win))),
                )
            {
                let Some(win) = win else { continue };
                let client = self.client(win).unwrap_or_else(|| {
                    panic!("monitor {monitor_id:?} {source} references missing {win:?}")
                });
                assert_eq!(
                    client.monitor_id, monitor_id,
                    "monitor {monitor_id:?} {source} references client {win:?} owned by {:?}",
                    client.monitor_id
                );
                assert!(
                    monitor.clients.contains(&win),
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
        if let Some(mid) = self.client(win).map(|client| client.monitor_id)
            && let Some(mon) = self.monitors.get_mut(mid)
            && mon.z_order.raise(win)
        {
            return;
        }

        // Fallback: search all monitors if the client's monitor assignment is
        // stale during a transfer or teardown path.
        for mon in self.monitors.iter_all_mut() {
            if mon.z_order.raise(win) {
                return;
            }
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
        let sel_mon_id = self.selected_monitor_id();
        if let Some(mon) = self.monitors.get_mut(sel_mon_id) {
            mon.move_client_in_stack(win, direction, &self.clients)
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
        let client = self.client(win)?;
        let is_scratchpad = client.is_scratchpad();
        let target_monitor = self.monitors.get(target_mon)?;
        let target_tags = if is_scratchpad {
            crate::types::TagMask::EMPTY
        } else {
            target_monitor.selected_tags()
        };
        let target_tag_idx = target_monitor.current_tag_number();

        {
            let client = self.client_mut(win)?;
            if !is_scratchpad {
                client.set_tag_mask(target_tags);
                client.reset_sticky(target_tag_idx);
            }
        }
        let needs_arrange = self
            .client(win)
            .is_some_and(|client| !client.mode().is_floating());
        let reassigned = self.reassign_client_monitor(win, target_mon);
        debug_assert!(reassigned, "validated transfer must succeed");
        Some(ClientTransferOutcome {
            is_scratchpad,
            needs_arrange,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClientTransferOutcome {
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
    use crate::types::{Rect, TagMask};

    #[test]
    fn client_view_resolves_client_and_assigned_monitor() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor {
            monitor_rect: Rect::new(1920, 0, 2560, 1440),
            ..Monitor::default()
        });
        let win = WindowId(42);
        model.insert_client(Client {
            win,
            monitor_id,
            geo: Rect::new(2000, 100, 800, 600),
            ..Client::default()
        });

        let view = model.client_view(win).expect("client view");

        assert_eq!(view.client.win, win);
        assert_eq!(view.client.geo, Rect::new(2000, 100, 800, 600));
        assert_eq!(view.monitor.id(), monitor_id);
        assert_eq!(view.monitor.monitor_rect, Rect::new(1920, 0, 2560, 1440));
    }

    #[test]
    fn client_view_requires_a_valid_client_monitor_relationship() {
        let mut model = WmModel::new();
        let win = WindowId(7);
        model.insert_client(Client {
            win,
            monitor_id: MonitorId::from_raw(999),
            ..Client::default()
        });

        assert!(model.client(win).is_some());
        assert!(model.client_view(win).is_none());
        assert!(model.client_view(WindowId(8)).is_none());
    }

    #[test]
    fn selected_view_visibility_is_resolved_as_one_model_query() {
        let mut model = WmModel::new();
        let visible_tags = TagMask::single(2).unwrap();
        let selected_monitor = model.monitors.push(Monitor::default());
        let other_monitor = model.monitors.push(Monitor::default());
        model.monitors.set_selected(selected_monitor);
        model
            .monitor_mut(selected_monitor)
            .unwrap()
            .set_selected_tags(visible_tags);

        let visible = WindowId(1);
        let hidden = WindowId(2);
        let elsewhere = WindowId(3);
        for (win, monitor_id, tags) in [
            (visible, selected_monitor, visible_tags),
            (hidden, selected_monitor, TagMask::single(1).unwrap()),
            (elsewhere, other_monitor, visible_tags),
        ] {
            model.insert_client(Client {
                win,
                monitor_id,
                tags,
                ..Client::default()
            });
        }

        assert!(model.client_is_visible_on_selected_monitor(visible));
        assert!(!model.client_is_visible_on_selected_monitor(hidden));
        assert!(!model.client_is_visible_on_selected_monitor(elsewhere));
    }

    #[test]
    fn invalid_transfer_target_does_not_modify_client_assignment() {
        let mut model = WmModel::new();
        let source = model.monitors.push(Monitor::default());
        let win = WindowId(9);
        model.insert_client(Client {
            win,
            monitor_id: source,
            ..Client::default()
        });

        let outcome = model.move_client_to_monitor(win, MonitorId::from_raw(999));

        assert!(outcome.is_none());
        assert_eq!(
            model.client(win).map(|client| client.monitor_id),
            Some(source)
        );
    }

    #[test]
    fn removing_client_clears_every_monitor_owned_reference() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor::default());
        let win = WindowId(10);
        let other = WindowId(11);
        let tags = TagMask::single(1).unwrap();
        model.insert_client(Client {
            win,
            monitor_id,
            tags,
            ..Client::default()
        });
        model.insert_client(Client {
            win: other,
            monitor_id,
            tags,
            ..Client::default()
        });

        let monitor = model.monitor_mut(monitor_id).unwrap();
        monitor.clients = vec![other, win];
        monitor.z_order.attach_top(other);
        monitor.z_order.attach_top(win);
        monitor.selected = Some(win);
        monitor.tag_focus_history.insert(tags, win);
        monitor.tag_tiled_focus_history.insert(tags, win);

        let removed = model.remove_client(win);

        assert_eq!(removed.map(|client| client.win), Some(win));
        assert!(model.client(win).is_none());
        let monitor = model.monitor(monitor_id).unwrap();
        assert_eq!(monitor.clients, vec![other]);
        assert_eq!(monitor.z_order.as_slice(), &[other]);
        assert_eq!(monitor.selected, None);
        assert!(
            !monitor
                .tag_focus_history
                .values()
                .any(|candidate| *candidate == win)
        );
        assert!(
            !monitor
                .tag_tiled_focus_history
                .values()
                .any(|candidate| *candidate == win)
        );
    }

    #[test]
    fn attaching_client_updates_focus_order_and_z_order_together() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor::default());
        let win = WindowId(12);
        model.insert_client(Client {
            win,
            monitor_id,
            ..Client::default()
        });

        assert!(model.attach_client(win));
        assert!(model.attach_client(win));

        let monitor = model.monitor(monitor_id).unwrap();
        assert_eq!(monitor.clients, vec![win]);
        assert_eq!(monitor.z_order.as_slice(), &[win]);
    }

    #[test]
    fn inserting_duplicate_client_cannot_replace_existing_state() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor::default());
        let win = WindowId(14);
        assert!(model.insert_client(Client {
            win,
            monitor_id,
            name: "original".to_string(),
            ..Client::default()
        }));

        assert!(!model.insert_client(Client {
            win,
            monitor_id,
            name: "replacement".to_string(),
            ..Client::default()
        }));
        assert_eq!(
            model.client(win).map(|client| client.name.as_str()),
            Some("original")
        );
    }

    #[test]
    fn reassigning_client_clears_source_references_and_attaches_target() {
        let mut model = WmModel::new();
        let source = model.monitors.push(Monitor::default());
        let target = model.monitors.push(Monitor::default());
        let win = WindowId(13);
        let tags = TagMask::single(1).unwrap();
        model.insert_client(Client {
            win,
            monitor_id: source,
            tags,
            ..Client::default()
        });
        assert!(model.attach_client(win));
        let source_monitor = model.monitor_mut(source).unwrap();
        source_monitor.selected = Some(win);
        source_monitor.tag_focus_history.insert(tags, win);
        source_monitor.tag_tiled_focus_history.insert(tags, win);

        assert!(model.reassign_client_monitor(win, target));

        let source_monitor = model.monitor(source).unwrap();
        assert!(source_monitor.clients.is_empty());
        assert!(source_monitor.z_order.as_slice().is_empty());
        assert_eq!(source_monitor.selected, None);
        assert!(source_monitor.tag_focus_history.is_empty());
        assert!(source_monitor.tag_tiled_focus_history.is_empty());
        let target_monitor = model.monitor(target).unwrap();
        assert_eq!(target_monitor.clients, vec![win]);
        assert_eq!(target_monitor.z_order.as_slice(), &[win]);
        assert_eq!(target_monitor.selected, Some(win));
        assert_eq!(
            model.client(win).map(|client| client.monitor_id),
            Some(target)
        );
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
