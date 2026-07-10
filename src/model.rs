//! Window manager authoritative model state.
//!
//! `WmModel` owns the core client/monitor/tag graph that represents the
//! window manager's authoritative state.  This graph is backend-neutral
//! and can be tested without constructing a backend.

use crate::client::manager::ClientManager;
use crate::monitor::MonitorManager;
use crate::types::{MonitorId, TagSet, WindowId};

/// Authoritative window-manager model state.
///
/// Clients, monitors, and tags form a cross-referenced graph and are
/// kept together so their invariants have a single owner.
pub struct WmModel {
    /// All managed clients.
    pub(crate) clients: ClientManager,
    /// All monitors/screens.
    pub(crate) monitors: MonitorManager,
    /// Shared tag metadata.
    pub(crate) tags: TagSet,
}

impl WmModel {
    pub fn new() -> Self {
        Self {
            clients: ClientManager::new(),
            monitors: MonitorManager::new(),
            tags: TagSet::default(),
        }
    }

    // -------------------------------------------------------------------------
    // Selected-monitor convenience helpers
    // -------------------------------------------------------------------------

    /// Return the window currently selected on the selected monitor, if any.
    #[inline]
    pub fn selected_win(&self) -> Option<WindowId> {
        self.monitors.sel().and_then(|m| m.sel)
    }

    /// Return the ID of the currently selected monitor.
    pub fn selected_monitor_id(&self) -> MonitorId {
        self.monitors.sel_idx()
    }

    /// Change the currently selected monitor.
    pub fn set_selected_monitor(&mut self, id: MonitorId) {
        self.monitors.set_sel_idx(id);
    }

    /// Shorthand to get the selected monitor.
    pub fn selected_monitor(&self) -> &crate::types::Monitor {
        self.monitors.sel_unchecked()
    }

    /// Shorthand to get the selected monitor mutably.
    pub fn selected_monitor_mut(&mut self) -> &mut crate::types::Monitor {
        self.monitors.sel_mut_unchecked()
    }

    /// Shorthand to get the selected monitor (Option version).
    pub fn selected_monitor_opt(&self) -> Option<&crate::types::Monitor> {
        self.monitors.sel()
    }

    /// Shorthand to get the selected monitor mutably (Option version).
    pub fn selected_monitor_mut_opt(&mut self) -> Option<&mut crate::types::Monitor> {
        self.monitors.sel_mut()
    }

    /// Return `true` if overview mode is active on the selected monitor.
    pub fn is_overview_active(&self) -> bool {
        self.selected_monitor().overview_state.is_some()
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

    /// Clear the maximized reference on any monitor that holds `win`.
    pub fn clear_maximized_for(&mut self, win: WindowId) {
        for mon in self.monitors.iter_all_mut() {
            if mon.maximized == Some(win) {
                mon.maximized = None;
            }
        }
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
    // Client List Management (Attach/Detach)
    // -------------------------------------------------------------------------

    /// Attach `win` to its assigned monitor's focus list.
    pub fn attach(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.monitor_id(win)
            && let Some(mon) = self.monitors.get_mut(mid)
        {
            mon.clients.insert(0, win);
        }
    }

    /// Detach `win` from its assigned monitor's focus list.
    pub fn detach(&mut self, win: WindowId) {
        let monitor_id = self.clients.monitor_id(win);
        if let Some(mid) = monitor_id
            && let Some(mon) = self.monitors.get_mut(mid)
            && mon.clients.contains(&win)
        {
            mon.clients.retain(|&w| w != win);
            return;
        }

        // Fallback: search all monitors if not found on the assigned one.
        for mon in self.monitors.iter_all_mut() {
            if mon.clients.contains(&win) {
                mon.clients.retain(|&w| w != win);
            }
        }
    }

    /// Attach `win` to the top of its assigned monitor's persistent z-order.
    pub fn attach_z_order_top(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.monitor_id(win)
            && let Some(mon) = self.monitors.get_mut(mid)
        {
            mon.z_order.attach_top(win);
        }
    }

    /// Detach `win` from its assigned monitor's persistent z-order.
    pub fn detach_z_order(&mut self, win: WindowId) {
        let monitor_id = self.clients.monitor_id(win);

        let handle_monitor = |mon: &mut crate::types::Monitor| -> bool { mon.z_order.remove(win) };

        if let Some(mid) = monitor_id
            && let Some(mon) = self.monitors.get_mut(mid)
            && handle_monitor(mon)
        {
            return;
        }

        // Fallback: search all monitors if not found on the assigned one.
        for mon in self.monitors.iter_all_mut() {
            if handle_monitor(mon) {
                return;
            }
        }
    }

    /// Move `win` to the top of its monitor's persistent z-order.
    pub fn raise_client_in_z_order(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.monitor_id(win)
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

    /// Move a client window to a target monitor in the data model.
    pub fn move_client_to_monitor(
        &mut self,
        win: WindowId,
        target_mon: MonitorId,
    ) -> Option<ClientTransferOutcome> {
        let client = self.clients.get(&win)?;
        let is_scratchpad = client.is_scratchpad();
        let target_tags = if is_scratchpad {
            crate::types::TagMask::EMPTY
        } else {
            self.monitors
                .get(target_mon)
                .map(|m| m.selected_tags())
                .unwrap_or(crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY))
        };
        let target_tag_idx = self
            .monitors
            .get(target_mon)
            .and_then(|m| m.current_tag_number());

        self.detach(win);
        self.detach_z_order(win);
        let client = self.clients.get_mut(&win)?;
        client.monitor_id = target_mon;
        if !is_scratchpad {
            client.set_tag_mask(target_tags);
            client.reset_sticky(target_tag_idx);
        }
        let needs_arrange = !client.mode.is_floating();
        self.attach(win);
        self.attach_z_order_top(win);
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
