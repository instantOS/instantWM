//! Shared fixtures for inline `#[cfg(test)]` modules.
//!
//! Before monitors owned their clients, every test had to hand-build the same
//! three-part graph: insert the client into the model, then separately push it
//! onto the monitor's focus list, attach it to the z-order, and optionally set
//! the monitor's selection. Fixtures that did only part of that produced tests
//! asserting against a graph the model would have rejected in production.
//!
//! Ownership collapsed that sequence into one operation, so these helpers are
//! thin wrappers that exist purely to keep test setup to a single call.

use crate::model::WmModel;
use crate::types::{Client, Monitor, MonitorId, Rect, Tag, TagMask, WindowId};

/// Builder for a [`Monitor`] fixture.
///
/// A monitor's owned clients, focus order, and z-order are private, so struct
/// update syntax (`Monitor { .., ..Monitor::default() }`) no longer compiles
/// from outside the defining module. This is the replacement for fixtures that
/// used to spell their whole monitor out in one literal.
pub struct MonitorBuilder(Monitor);

impl MonitorBuilder {
    /// Start from a default monitor.
    pub fn new() -> Self {
        Self(Monitor::default())
    }

    /// Set the full monitor rectangle and the area left after exclusive zones.
    pub fn rect(mut self, monitor: Rect, available: Rect) -> Self {
        self.0.monitor_rect = monitor;
        self.0.available_rect = available;
        self
    }

    /// Set the full monitor rectangle, leaving `available_rect` equal to it.
    ///
    /// This sets **both** rectangles. If the fixture needs `monitor_rect` set
    /// while `available_rect` stays at its default — which changes the work
    /// area — use [`Self::configure`] instead rather than this.
    pub fn monitor_rect(mut self, monitor: Rect) -> Self {
        self.0.monitor_rect = monitor;
        self.0.available_rect = monitor;
        self
    }

    /// Name the output, as a real backend would report it.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.0.name = name.into();
        self
    }

    /// Set the bar height and whether the bar is shown.
    pub fn bar(mut self, height: i32, shown: bool) -> Self {
        self.0.bar_height = height;
        self.0.bar_default_show = shown;
        self
    }

    /// Set the bottom gesture strip's height and whether it is shown.
    pub fn bottom_bar(mut self, height: i32, shown: bool) -> Self {
        self.0.bottom_bar_height = height;
        self.0.show_bottom_bar = shown;
        self
    }

    /// Give the monitor `count` default tags.
    ///
    /// Required before [`Self::selected_tags`] can select anything: a monitor
    /// with no tag list resolves every selection to the empty mask.
    pub fn tag_count(mut self, count: usize) -> Self {
        self.0.tags = vec![Tag::default(); count];
        self
    }

    /// Select `tags` on this monitor. Call [`Self::tag_count`] first.
    pub fn selected_tags(mut self, tags: TagMask) -> Self {
        self.0.set_selected_tags(tags);
        self
    }

    /// Set the monitor's own window id, for tests about bar windows.
    pub fn bar_window(mut self, win: WindowId) -> Self {
        self.0.bar_win = win;
        self
    }

    /// Reach any public field the builder does not cover.
    pub fn configure(mut self, f: impl FnOnce(&mut Monitor)) -> Self {
        f(&mut self.0);
        self
    }

    /// Give the monitor its own clients, in `order` as the focus order.
    ///
    /// For a monitor built outside a model. Inside a model, prefer
    /// [`add_client`] / [`add_selected_client`], which report a bad fixture
    /// instead of silently dropping a window.
    pub fn owning(mut self, order: &[WindowId], clients: impl IntoIterator<Item = Client>) -> Self {
        for client in clients {
            self.0.adopt_client(client, false);
        }
        assert!(
            self.0.set_focus_order(order.to_vec()),
            "focus order must name exactly the monitor's own clients, without repeats"
        );
        self
    }

    /// Finish the monitor.
    pub fn build(self) -> Monitor {
        self.0
    }
}

impl Default for MonitorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Push a default monitor and return its id.
pub fn push_monitor(model: &mut WmModel) -> MonitorId {
    model.monitors.push(Monitor::default())
}

/// Push a monitor configured by `f`, and return its id.
pub fn push_monitor_with(model: &mut WmModel, f: impl FnOnce(&mut Monitor)) -> MonitorId {
    let id = model.monitors.push(Monitor::default());
    if let Some(monitor) = model.monitor_mut(id) {
        f(monitor);
    }
    id
}

/// Add `client` to `monitor_id` and return its window id.
///
/// The client becomes fully managed: present in the monitor's client map, focus
/// stack, and z-order at once.
pub fn add_client(model: &mut WmModel, monitor_id: MonitorId, client: Client) -> WindowId {
    let win = client.win;
    assert!(
        model.add_client(monitor_id, client),
        "test fixture must add {win:?} to an existing, unoccupied monitor slot"
    );
    win
}

/// Add `client` to `monitor_id` and make it that monitor's selection.
pub fn add_selected_client(model: &mut WmModel, monitor_id: MonitorId, client: Client) -> WindowId {
    let win = client.win;
    assert!(
        model.readopt_client(monitor_id, client, true),
        "test fixture must add selected {win:?} to an existing, unoccupied monitor slot"
    );
    win
}

/// Add a client to `monitor_id`, configuring it through `f`, and return its
/// window id.
pub fn add_client_with(
    model: &mut WmModel,
    monitor_id: MonitorId,
    f: impl FnOnce(&mut Client),
) -> WindowId {
    let mut client = Client::default();
    f(&mut client);
    add_client(model, monitor_id, client)
}

/// Add a client that becomes `monitor_id`'s selection, configuring it through
/// `f`, and return its window id.
pub fn add_selected_client_with(
    model: &mut WmModel,
    monitor_id: MonitorId,
    f: impl FnOnce(&mut Client),
) -> WindowId {
    let mut client = Client::default();
    f(&mut client);
    add_selected_client(model, monitor_id, client)
}
