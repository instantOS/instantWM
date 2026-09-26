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
use crate::types::{Client, Monitor, MonitorId, WindowId};

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
