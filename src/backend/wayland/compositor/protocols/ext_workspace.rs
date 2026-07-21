//! ext-workspace protocol implementation.
//!
//! Maps ext-workspace-v1 to instantWM tag sets:
//! - Workspace groups map directly to physical outputs/monitors.
//! - Workspaces map to individual tags (1 to 21).
//! - Active/Urgent state is synchronized from monitor selected_tags bitmasks and client urgency flags.
//! - Client requests to activate a workspace are queued into `WmCommand` tag switch actions.

use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::Mutex;

use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::super::state::WaylandState;

const VERSION: u32 = 1;

enum Action {
    Activate(String, usize), // (output_name, tag_index)
}

/// UserData stored with each ExtWorkspaceHandleV1 resource to avoid O(N) scans.
pub struct ExtWorkspaceUserData {
    pub manager: ExtWorkspaceManagerV1,
    pub output_name: String,
    pub tag_index: usize,
}

/// UserData stored with each ExtWorkspaceGroupHandleV1 resource.
pub struct ExtWorkspaceGroupUserData {
    pub manager: ExtWorkspaceManagerV1,
    pub output_name: String,
    pub sent_outputs: Mutex<HashSet<smithay::reexports::wayland_server::backend::ObjectId>>,
}

pub struct ExtWorkspaceManagerState {
    display: DisplayHandle,
    instances: HashMap<ExtWorkspaceManagerV1, Vec<Action>>,
    pub(crate) workspace_groups: HashMap<String, ExtWorkspaceGroupData>, // output_name -> group
    pub(crate) workspaces: HashMap<(String, usize), ExtWorkspaceData>, // (output_name, tag_index) -> workspace

    // Performance: Caches to support O(1) fast-path early-return during tick checks.
    last_tags: HashMap<String, crate::types::TagMask>,
    last_urgent_tags: HashMap<String, crate::types::TagMask>,
    last_occupied_tags: HashMap<String, crate::types::TagMask>,
    last_output_names: Vec<String>,
}

pub(crate) struct ExtWorkspaceGroupData {
    pub(crate) instances: Vec<ExtWorkspaceGroupHandleV1>,
}

pub(crate) struct ExtWorkspaceData {
    pub(crate) name: String,
    pub(crate) coordinates: [u32; 2],
    pub(crate) state: ext_workspace_handle_v1::State,
    pub(crate) instances: Vec<ExtWorkspaceHandleV1>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExtWorkspaceGlobalData;

impl ExtWorkspaceManagerState {
    pub fn new(display: &DisplayHandle) -> Self {
        display.create_global::<WaylandState, ExtWorkspaceManagerV1, _>(
            VERSION,
            ExtWorkspaceGlobalData,
        );
        Self {
            display: display.clone(),
            instances: HashMap::new(),
            workspace_groups: HashMap::new(),
            workspaces: HashMap::new(),
            last_tags: HashMap::new(),
            last_urgent_tags: HashMap::new(),
            last_occupied_tags: HashMap::new(),
            last_output_names: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct WorkspaceMonitorSnapshot {
    output: Output,
    output_name: String,
    selected_tags: crate::types::TagMask,
    urgent_tags: crate::types::TagMask,
    occupied_tags: crate::types::TagMask,
    tag_names: Vec<String>,
}

struct WorkspaceSnapshot {
    monitors: Vec<WorkspaceMonitorSnapshot>,
}

impl WorkspaceSnapshot {
    fn capture(state: &WaylandState) -> Self {
        let globals = state.globals();
        let monitors = state
            .space
            .outputs()
            .cloned()
            .map(|output| {
                let output_name = output.name();
                let monitor = globals.and_then(|globals| {
                    globals
                        .model
                        .monitors
                        .iter_all()
                        .find(|monitor| monitor.name == output_name)
                });
                let selected_tags = monitor.map_or(
                    crate::types::TagMask::EMPTY,
                    crate::types::Monitor::selected_tags,
                );
                let urgent_tags = monitor.map_or(crate::types::TagMask::EMPTY, |monitor| {
                    monitor
                        .clients
                        .iter()
                        .fold(crate::types::TagMask::EMPTY, |urgent, window| {
                            let client = globals.and_then(|globals| globals.model.client(*window));
                            client
                                .filter(|client| client.is_urgent)
                                .map_or(urgent, |client| urgent | client.tags)
                        })
                });
                let occupied_tags = monitor.map_or(crate::types::TagMask::EMPTY, |monitor| {
                    globals.map_or(crate::types::TagMask::EMPTY, |globals| {
                        monitor.occupied_tags(&globals.model.clients)
                    })
                });
                let tag_names = monitor.map_or_else(
                    || (1..=9).map(|index| index.to_string()).collect(),
                    |monitor| monitor.tags.iter().map(|tag| tag.name.clone()).collect(),
                );
                WorkspaceMonitorSnapshot {
                    output,
                    output_name,
                    selected_tags,
                    urgent_tags,
                    occupied_tags,
                    tag_names,
                }
            })
            .collect();
        Self { monitors }
    }

    fn output_names(&self) -> Vec<String> {
        self.monitors
            .iter()
            .map(|monitor| monitor.output_name.clone())
            .collect()
    }

    fn update_cache(&self, protocol: &mut ExtWorkspaceManagerState) {
        protocol.last_output_names = self.output_names();
        protocol.last_tags = self
            .monitors
            .iter()
            .map(|monitor| (monitor.output_name.clone(), monitor.selected_tags))
            .collect();
        protocol.last_urgent_tags = self
            .monitors
            .iter()
            .map(|monitor| (monitor.output_name.clone(), monitor.urgent_tags))
            .collect();
        protocol.last_occupied_tags = self
            .monitors
            .iter()
            .map(|monitor| (monitor.output_name.clone(), monitor.occupied_tags))
            .collect();
    }
}

pub fn refresh(state: &mut WaylandState) {
    if !refresh_needed(state) {
        return;
    }
    let snapshot = WorkspaceSnapshot::capture(state);
    let protocol = &mut state.ext_workspace_state;
    snapshot.update_cache(protocol);
    let mut changed = remove_stale_outputs(protocol, &snapshot.output_names());
    for monitor in &snapshot.monitors {
        changed |= reconcile_workspaces(protocol, monitor);
        changed |= reconcile_workspace_group(protocol, monitor);
        changed |= announce_client_outputs(protocol, monitor);
    }

    if changed {
        protocol.instances.keys().for_each(|manager| manager.done());
    }
}

fn refresh_needed(state: &WaylandState) -> bool {
    let protocol = &state.ext_workspace_state;
    if state.space.outputs().count() != protocol.last_output_names.len()
        || state
            .space
            .outputs()
            .enumerate()
            .any(|(index, output)| protocol.last_output_names.get(index) != Some(&output.name()))
    {
        return true;
    }
    let Some(globals) = state.globals() else {
        return true;
    };
    for output in state.space.outputs() {
        let output_name = output.name();
        let Some(monitor) = globals
            .model
            .monitors
            .iter_all()
            .find(|monitor| monitor.name == output_name)
        else {
            return true;
        };
        let urgent_tags =
            monitor
                .clients
                .iter()
                .fold(crate::types::TagMask::EMPTY, |urgent, window| {
                    globals
                        .model
                        .client(*window)
                        .filter(|client| client.is_urgent)
                        .map_or(urgent, |client| urgent | client.tags)
                });
        if protocol.last_tags.get(&output_name) != Some(&monitor.selected_tags())
            || protocol.last_urgent_tags.get(&output_name) != Some(&urgent_tags)
            || protocol.last_occupied_tags.get(&output_name)
                != Some(&monitor.occupied_tags(&globals.model.clients))
        {
            return true;
        }
    }

    protocol.workspace_groups.values().any(|group_data| {
        group_data.instances.iter().any(|group| {
            let Some(client) = group.client() else {
                return false;
            };
            let Some(user_data) = group.data::<ExtWorkspaceGroupUserData>() else {
                return false;
            };
            let Some(output) = state
                .space
                .outputs()
                .find(|output| output.name() == user_data.output_name)
            else {
                return false;
            };
            output.client_outputs(&client).any(|output| {
                !user_data
                    .sent_outputs
                    .lock()
                    .unwrap()
                    .contains(&output.id())
            })
        })
    })
}

fn remove_stale_outputs(
    protocol: &mut ExtWorkspaceManagerState,
    active_output_names: &[String],
) -> bool {
    let active = active_output_names.iter().collect::<HashSet<_>>();
    let mut changed = false;
    protocol.workspace_groups.retain(|output_name, data| {
        if active.contains(output_name) {
            return true;
        }
        for group in &data.instances {
            if let Some(manager) = group.data::<ExtWorkspaceManagerV1>() {
                for ((workspace_output, _), workspace_data) in &protocol.workspaces {
                    if workspace_output == output_name {
                        for workspace in &workspace_data.instances {
                            if workspace
                                .data::<ExtWorkspaceUserData>()
                                .map(|data| &data.manager)
                                == Some(manager)
                            {
                                group.workspace_leave(workspace);
                            }
                        }
                    }
                }
            }
            group.removed();
        }
        changed = true;
        false
    });
    protocol.workspaces.retain(|(output_name, _), workspace| {
        if active.contains(output_name) {
            return true;
        }
        workspace
            .instances
            .iter()
            .for_each(|instance| instance.removed());
        changed = true;
        false
    });
    changed
}

fn workspace_state(
    monitor: &WorkspaceMonitorSnapshot,
    tag_index: usize,
) -> ext_workspace_handle_v1::State {
    let tag_number = tag_index + 1;
    let active = monitor.selected_tags.contains(tag_number);
    let mut state = ext_workspace_handle_v1::State::empty();
    if active {
        state |= ext_workspace_handle_v1::State::Active;
    }
    if monitor.urgent_tags.contains(tag_number) {
        state |= ext_workspace_handle_v1::State::Urgent;
    }
    if !active && !monitor.occupied_tags.contains(tag_number) {
        state |= ext_workspace_handle_v1::State::Hidden;
    }
    state
}

fn reconcile_workspaces(
    protocol: &mut ExtWorkspaceManagerState,
    monitor: &WorkspaceMonitorSnapshot,
) -> bool {
    let mut changed = false;
    for (tag_index, name) in monitor.tag_names.iter().cloned().enumerate() {
        let state = workspace_state(monitor, tag_index);
        let key = (monitor.output_name.clone(), tag_index);
        match protocol.workspaces.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let workspace = entry.get_mut();
                let name_changed = workspace.name != name;
                let state_changed = workspace.state != state;
                if name_changed {
                    workspace.name = name;
                }
                if state_changed {
                    workspace.state = state;
                }
                if name_changed || state_changed {
                    for instance in &workspace.instances {
                        if name_changed {
                            instance.name(workspace.name.clone());
                        }
                        if state_changed {
                            instance.state(workspace.state);
                        }
                    }
                    changed = true;
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut workspace = ExtWorkspaceData {
                    name,
                    coordinates: [0, tag_index as u32],
                    state,
                    instances: Vec::new(),
                };
                for manager in protocol.instances.keys() {
                    if let Some(client) = manager.client() {
                        workspace.add_instance(
                            &protocol.display,
                            &client,
                            manager,
                            &monitor.output_name,
                            tag_index,
                        );
                    }
                }
                enter_workspace_in_matching_groups(
                    protocol.workspace_groups.get(&monitor.output_name),
                    &workspace,
                );
                entry.insert(workspace);
                changed = true;
            }
        }
    }
    changed
}

fn enter_workspace_in_matching_groups(
    group_data: Option<&ExtWorkspaceGroupData>,
    workspace: &ExtWorkspaceData,
) {
    let Some(group_data) = group_data else {
        return;
    };
    for group in &group_data.instances {
        let Some(manager) = group.data::<ExtWorkspaceManagerV1>() else {
            continue;
        };
        for instance in &workspace.instances {
            if instance
                .data::<ExtWorkspaceUserData>()
                .map(|data| &data.manager)
                == Some(manager)
            {
                group.workspace_enter(instance);
            }
        }
    }
}

fn reconcile_workspace_group(
    protocol: &mut ExtWorkspaceManagerState,
    monitor: &WorkspaceMonitorSnapshot,
) -> bool {
    if protocol.workspace_groups.contains_key(&monitor.output_name) {
        return false;
    }

    let mut group_data = ExtWorkspaceGroupData {
        instances: Vec::new(),
    };
    for manager in protocol.instances.keys() {
        if let Some(client) = manager.client() {
            group_data.add_instance(&protocol.display, &client, manager, &monitor.output);
        }
    }
    for group in &group_data.instances {
        let Some(manager) = group.data::<ExtWorkspaceManagerV1>() else {
            continue;
        };
        for ((output_name, _), workspace) in &protocol.workspaces {
            if output_name == &monitor.output_name {
                for instance in &workspace.instances {
                    if instance
                        .data::<ExtWorkspaceUserData>()
                        .map(|data| &data.manager)
                        == Some(manager)
                    {
                        group.workspace_enter(instance);
                    }
                }
            }
        }
    }
    protocol
        .workspace_groups
        .insert(monitor.output_name.clone(), group_data);
    true
}

fn announce_client_outputs(
    protocol: &mut ExtWorkspaceManagerState,
    monitor: &WorkspaceMonitorSnapshot,
) -> bool {
    let Some(group_data) = protocol.workspace_groups.get_mut(&monitor.output_name) else {
        return false;
    };
    let mut changed = false;
    for group in &group_data.instances {
        let Some(client) = group.client() else {
            continue;
        };
        let Some(user_data) = group.data::<ExtWorkspaceGroupUserData>() else {
            continue;
        };
        for output in monitor.output.client_outputs(&client) {
            let mut sent_outputs = user_data.sent_outputs.lock().unwrap();
            if sent_outputs.insert(output.id()) {
                group.output_enter(&output);
                changed = true;
            }
        }
    }
    changed
}

impl ExtWorkspaceGroupData {
    fn add_instance(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
        output: &Output,
    ) -> Option<&ExtWorkspaceGroupHandleV1> {
        let group = match client.create_resource::<ExtWorkspaceGroupHandleV1, _, WaylandState>(
            handle,
            manager.version(),
            ExtWorkspaceGroupUserData {
                manager: manager.clone(),
                output_name: output.name(),
                sent_outputs: Mutex::new(HashSet::new()),
            },
        ) {
            Ok(g) => g,
            Err(e) => {
                log::error!(
                    "Failed to create ExtWorkspaceGroupHandleV1 resource safely: {}",
                    e
                );
                return None;
            }
        };
        manager.workspace_group(&group);

        // GroupCapabilities::empty() is intentional as instantWM manages tag sets locally via its internal configs.
        group.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());

        for wl_output in output.client_outputs(client) {
            group.output_enter(&wl_output);
            if let Some(ud) = group.data::<ExtWorkspaceGroupUserData>() {
                ud.sent_outputs.lock().unwrap().insert(wl_output.id());
            }
        }

        self.instances.push(group);
        self.instances.last()
    }
}

impl ExtWorkspaceData {
    fn add_instance(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
        output_name: &str,
        tag_index: usize,
    ) -> Option<&ExtWorkspaceHandleV1> {
        let workspace = match client.create_resource::<ExtWorkspaceHandleV1, _, WaylandState>(
            handle,
            manager.version(),
            ExtWorkspaceUserData {
                manager: manager.clone(),
                output_name: output_name.to_string(),
                tag_index,
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                log::error!(
                    "Failed to create ExtWorkspaceHandleV1 resource safely: {}",
                    e
                );
                return None;
            }
        };
        manager.workspace(&workspace);

        workspace.name(self.name.clone());

        // Spec mandates `coordinates` is a `vec<u32>` of native endianness.
        // Rust's `wayland-server` maps Wayland's `array` type to a `Vec<u8>`.
        // We serialize `u32` to native endian bytes to meet this signature.
        workspace.coordinates(
            self.coordinates
                .iter()
                .flat_map(|x| x.to_ne_bytes())
                .collect(),
        );
        workspace.state(self.state);
        workspace.capabilities(ext_workspace_handle_v1::WorkspaceCapabilities::Activate);

        self.instances.push(workspace);
        self.instances.last()
    }
}

impl GlobalDispatch<ExtWorkspaceManagerV1, ExtWorkspaceGlobalData, WaylandState> for WaylandState {
    fn bind(
        state: &mut WaylandState,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _global_data: &ExtWorkspaceGlobalData,
        data_init: &mut DataInit<'_, WaylandState>,
    ) {
        let manager = data_init.init(resource, ());

        let manager_state = &mut state.ext_workspace_state;

        let mut new_workspaces: HashMap<_, Vec<_>> = HashMap::new();
        for ((ws_output, ws_idx), ws_data) in &mut manager_state.workspaces {
            if let Some(workspace) =
                ws_data.add_instance(handle, client, &manager, ws_output, *ws_idx)
            {
                new_workspaces
                    .entry(ws_output.clone())
                    .or_default()
                    .push(workspace.clone());
            }
        }

        for (output_name, group_data) in &mut manager_state.workspace_groups {
            let output = state
                .space
                .outputs()
                .find(|o| o.name() == *output_name)
                .cloned();
            if let Some(output) = output
                && let Some(group) = group_data.add_instance(handle, client, &manager, &output)
                && let Some(workspaces) = new_workspaces.get(output_name)
            {
                for workspace in workspaces {
                    group.workspace_enter(workspace);
                }
            }
        }

        manager.done();
        manager_state.instances.insert(manager, Vec::new());
    }
}

impl Dispatch<ExtWorkspaceManagerV1, (), WaylandState> for WaylandState {
    fn request(
        state: &mut WaylandState,
        _client: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: <ExtWorkspaceManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, WaylandState>,
    ) {
        match request {
            ext_workspace_manager_v1::Request::Commit => {
                let manager_state = &mut state.ext_workspace_state;
                if let Some(actions) = manager_state.instances.get_mut(resource) {
                    let actions = mem::take(actions);
                    for action in actions {
                        match action {
                            Action::Activate(output_name, tag_index) => {
                                state.push_command(
                                    crate::backend::wayland::commands::WmCommand::SelectTag {
                                        monitor_name: output_name,
                                        tag_index,
                                    },
                                );
                            }
                        }
                    }
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                resource.finished();
                let manager_state = &mut state.ext_workspace_state;
                manager_state.instances.retain(|x, _| x != resource);
                for group_data in manager_state.workspace_groups.values_mut() {
                    group_data.instances.retain(|instance| {
                        instance
                            .data::<ExtWorkspaceGroupUserData>()
                            .map(|ud| &ud.manager)
                            != Some(resource)
                    });
                }
                for ws_data in manager_state.workspaces.values_mut() {
                    ws_data.instances.retain(|instance| {
                        instance
                            .data::<ExtWorkspaceUserData>()
                            .map(|ud| &ud.manager)
                            != Some(resource)
                    });
                }
            }
            _ => {
                log::debug!("unhandled ext_workspace_manager request: {:?}", request);
            }
        }
    }

    fn destroyed(
        state: &mut WaylandState,
        _client: ClientId,
        resource: &ExtWorkspaceManagerV1,
        _data: &(),
    ) {
        let manager_state = &mut state.ext_workspace_state;
        manager_state.instances.retain(|x, _| x != resource);
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceUserData, WaylandState> for WaylandState {
    fn request(
        state: &mut WaylandState,
        _client: &Client,
        _resource: &ExtWorkspaceHandleV1,
        request: <ExtWorkspaceHandleV1 as Resource>::Request,
        data: &ExtWorkspaceUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, WaylandState>,
    ) {
        let manager_state = &mut state.ext_workspace_state;

        if let ext_workspace_handle_v1::Request::Activate = request
            && let Some(actions) = manager_state.instances.get_mut(&data.manager)
        {
            actions.push(Action::Activate(data.output_name.clone(), data.tag_index));
        }
    }

    fn destroyed(
        state: &mut WaylandState,
        _client: ClientId,
        resource: &ExtWorkspaceHandleV1,
        data: &ExtWorkspaceUserData,
    ) {
        let manager_state = &mut state.ext_workspace_state;
        if let Some(ws_data) = manager_state
            .workspaces
            .get_mut(&(data.output_name.clone(), data.tag_index))
        {
            ws_data.instances.retain(|instance| instance != resource);
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceGroupUserData, WaylandState> for WaylandState {
    fn request(
        _state: &mut WaylandState,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        request: <ExtWorkspaceGroupHandleV1 as Resource>::Request,
        _data: &ExtWorkspaceGroupUserData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, WaylandState>,
    ) {
        log::debug!("unhandled ext_workspace_group request: {:?}", request);
    }

    fn destroyed(
        state: &mut WaylandState,
        _client: ClientId,
        resource: &ExtWorkspaceGroupHandleV1,
        data: &ExtWorkspaceGroupUserData,
    ) {
        let manager_state = &mut state.ext_workspace_state;
        if let Some(group_data) = manager_state.workspace_groups.get_mut(&data.output_name) {
            group_data.instances.retain(|instance| instance != resource);
        }
    }
}
