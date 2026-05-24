//! ext-workspace protocol implementation.
//!
//! Maps ext-workspace-v1 to instantWM tag sets:
//! - Workspace groups map directly to physical outputs/monitors.
//! - Workspaces map to individual tags (1 to 21).
//! - Active/Urgent state is synchronized from monitor selected_tags bitmasks and client urgency flags.
//! - Client requests to activate a workspace are queued into `WmCommand` tag switch actions.

use std::collections::HashMap;
use std::mem;

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

pub struct ExtWorkspaceManagerState {
    display: DisplayHandle,
    instances: HashMap<ExtWorkspaceManagerV1, Vec<Action>>,
    pub(crate) workspace_groups: HashMap<String, ExtWorkspaceGroupData>, // output_name -> group
    pub(crate) workspaces: HashMap<(String, usize), ExtWorkspaceData>,    // (output_name, tag_index) -> workspace
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
        display.create_global::<WaylandState, ExtWorkspaceManagerV1, _>(VERSION, ExtWorkspaceGlobalData);
        Self {
            display: display.clone(),
            instances: HashMap::new(),
            workspace_groups: HashMap::new(),
            workspaces: HashMap::new(),
        }
    }
}

pub fn refresh(state: &mut WaylandState) {
    let mut changed = false;

    // Get list of active outputs (monitors)
    let active_outputs: Vec<Output> = state.space.outputs().cloned().collect();
    let active_output_names: Vec<String> = active_outputs.iter().map(|o| o.name()).collect();

    // Map output name -> (selected_tags, tag_names)
    let mut monitors_info = HashMap::new();
    if let Some(globals) = state.globals() {
        for output_name in &active_output_names {
            if let Some(mon) = globals.monitors.monitors.iter().find(|m| m.name == *output_name) {
                let selected_tags = mon.selected_tags();
                let tag_names: Vec<String> = mon.tags.iter().map(|t| t.name.clone()).collect();
                monitors_info.insert(output_name.clone(), (selected_tags, tag_names));
            }
        }
    }

    let protocol_state = &mut state.ext_workspace_state;

    // 1. Remove workspace groups for outputs that no longer exist
    protocol_state.workspace_groups.retain(|output_name, data| {
        if active_output_names.contains(output_name) {
            return true;
        }
        for group in &data.instances {
            if let Some(manager) = group.data::<ExtWorkspaceManagerV1>() {
                // Send workspace_leave for all workspaces in this group
                for ((ws_output, _), ws) in &protocol_state.workspaces {
                    if ws_output == output_name {
                        for workspace in &ws.instances {
                            if workspace.data::<ExtWorkspaceManagerV1>() == Some(manager) {
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

    // 2. Remove workspaces for outputs that no longer exist
    protocol_state.workspaces.retain(|(output_name, _), ws| {
        if active_output_names.contains(output_name) {
            return true;
        }
        for workspace in &ws.instances {
            workspace.removed();
        }
        changed = true;
        false
    });

    // 3. For each active monitor, update workspaces and workspace groups
    for output in active_outputs {
        let output_name = output.name();

        let (selected_tags, tag_names) = monitors_info.get(&output_name)
            .cloned()
            .unwrap_or_else(|| {
                (crate::types::TagMask::EMPTY, (1..=9).map(|i| i.to_string()).collect())
            });

        // Add/Refresh workspaces for this output
        for (tag_idx, name) in tag_names.into_iter().enumerate() {
            let is_active = selected_tags.contains(tag_idx + 1);
            let mut ws_state = ext_workspace_handle_v1::State::empty();
            if is_active {
                ws_state |= ext_workspace_handle_v1::State::Active;
            }

            let key = (output_name.clone(), tag_idx);
            match protocol_state.workspaces.entry(key) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let ws_data = entry.get_mut();
                    let mut name_changed = false;
                    if ws_data.name != name {
                        ws_data.name = name.clone();
                        name_changed = true;
                    }
                    let mut state_changed = false;
                    if ws_data.state != ws_state {
                        ws_data.state = ws_state;
                        state_changed = true;
                    }

                    if name_changed || state_changed {
                        for instance in &ws_data.instances {
                            if name_changed {
                                instance.name(ws_data.name.clone());
                            }
                            if state_changed {
                                instance.state(ws_data.state);
                            }
                        }
                        changed = true;
                    }
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let mut ws_data = ExtWorkspaceData {
                        name,
                        coordinates: [0, tag_idx as u32],
                        state: ws_state,
                        instances: Vec::new(),
                    };
                    for manager in protocol_state.instances.keys() {
                        if let Some(client) = manager.client() {
                            ws_data.add_instance(&protocol_state.display, &client, manager);
                        }
                    }
                    if let Some(group_data) = protocol_state.workspace_groups.get(&output_name) {
                        for group in &group_data.instances {
                            if let Some(manager) = group.data::<ExtWorkspaceManagerV1>() {
                                for workspace in &ws_data.instances {
                                    if workspace.data::<ExtWorkspaceManagerV1>() == Some(manager) {
                                        group.workspace_enter(workspace);
                                    }
                                }
                            }
                        }
                    }
                    entry.insert(ws_data);
                    changed = true;
                }
            }
        }

        // Add/Refresh workspace group for this output
        if !protocol_state.workspace_groups.contains_key(&output_name) {
            let mut group_data = ExtWorkspaceGroupData {
                instances: Vec::new(),
            };
            for manager in protocol_state.instances.keys() {
                if let Some(client) = manager.client() {
                    group_data.add_instance(&protocol_state.display, &client, manager, &output);
                }
            }
            for group in &group_data.instances {
                if let Some(manager) = group.data::<ExtWorkspaceManagerV1>() {
                    for ((ws_output, _), ws) in &protocol_state.workspaces {
                        if ws_output == &output_name {
                            for workspace in &ws.instances {
                                if workspace.data::<ExtWorkspaceManagerV1>() == Some(manager) {
                                    group.workspace_enter(workspace);
                                }
                            }
                        }
                    }
                }
            }
            protocol_state.workspace_groups.insert(output_name, group_data);
            changed = true;
        }
    }

    if changed {
        for manager in protocol_state.instances.keys() {
            manager.done();
        }
    }
}

impl ExtWorkspaceGroupData {
    fn add_instance(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
        output: &Output,
    ) -> &ExtWorkspaceGroupHandleV1 {
        let group = client
            .create_resource::<ExtWorkspaceGroupHandleV1, _, WaylandState>(
                handle,
                manager.version(),
                manager.clone(),
            )
            .unwrap();
        manager.workspace_group(&group);

        group.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());

        for wl_output in output.client_outputs(client) {
            group.output_enter(&wl_output);
        }

        self.instances.push(group);
        self.instances.last().unwrap()
    }
}

impl ExtWorkspaceData {
    fn add_instance(
        &mut self,
        handle: &DisplayHandle,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
    ) -> &ExtWorkspaceHandleV1 {
        let workspace = client
            .create_resource::<ExtWorkspaceHandleV1, _, WaylandState>(
                handle,
                manager.version(),
                manager.clone(),
            )
            .unwrap();
        manager.workspace(&workspace);

        workspace.name(self.name.clone());
        workspace.coordinates(
            self.coordinates
                .iter()
                .flat_map(|x| x.to_ne_bytes())
                .collect(),
        );
        workspace.state(self.state);
        workspace.capabilities(ext_workspace_handle_v1::WorkspaceCapabilities::Activate);

        self.instances.push(workspace);
        self.instances.last().unwrap()
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
        for ((ws_output, _), ws_data) in &mut manager_state.workspaces {
            let workspace = ws_data.add_instance(handle, client, &manager);
            new_workspaces.entry(ws_output.clone()).or_default().push(workspace.clone());
        }

        for (output_name, group_data) in &mut manager_state.workspace_groups {
            let output = state.space.outputs().find(|o| o.name() == *output_name).cloned();
            if let Some(output) = output {
                let group = group_data.add_instance(handle, client, &manager, &output);
                if let Some(workspaces) = new_workspaces.get(output_name) {
                    for workspace in workspaces {
                        group.workspace_enter(workspace);
                    }
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
                                state.push_command(crate::backend::wayland::commands::WmCommand::SelectTag {
                                    monitor_name: output_name,
                                    tag_index,
                                });
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
                    group_data.instances.retain(|instance| instance.data::<ExtWorkspaceManagerV1>() != Some(resource));
                }
                for ws_data in manager_state.workspaces.values_mut() {
                    ws_data.instances.retain(|instance| instance.data::<ExtWorkspaceManagerV1>() != Some(resource));
                }
            }
            _ => unreachable!(),
        }
    }

    fn destroyed(state: &mut WaylandState, _client: ClientId, resource: &ExtWorkspaceManagerV1, _data: &()) {
        let manager_state = &mut state.ext_workspace_state;
        manager_state.instances.retain(|x, _| x != resource);
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ExtWorkspaceManagerV1, WaylandState> for WaylandState {
    fn request(
        state: &mut WaylandState,
        _client: &Client,
        resource: &ExtWorkspaceHandleV1,
        request: <ExtWorkspaceHandleV1 as Resource>::Request,
        data: &ExtWorkspaceManagerV1,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, WaylandState>,
    ) {
        let manager_state = &mut state.ext_workspace_state;
        let Some((key, _)) = manager_state
            .workspaces
            .iter()
            .find(|(_, ws_data)| ws_data.instances.contains(resource))
        else {
            return;
        };
        let key = key.clone();

        match request {
            ext_workspace_handle_v1::Request::Activate => {
                if let Some(actions) = manager_state.instances.get_mut(data) {
                    actions.push(Action::Activate(key.0, key.1));
                }
            }
            ext_workspace_handle_v1::Request::Deactivate => (),
            ext_workspace_handle_v1::Request::Assign { .. } => (),
            ext_workspace_handle_v1::Request::Remove => (),
            ext_workspace_handle_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut WaylandState,
        _client: ClientId,
        resource: &ExtWorkspaceHandleV1,
        _data: &ExtWorkspaceManagerV1,
    ) {
        let manager_state = &mut state.ext_workspace_state;
        for ws_data in manager_state.workspaces.values_mut() {
            ws_data.instances.retain(|instance| instance != resource);
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ExtWorkspaceManagerV1, WaylandState> for WaylandState {
    fn request(
        _state: &mut WaylandState,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        request: <ExtWorkspaceGroupHandleV1 as Resource>::Request,
        _data: &ExtWorkspaceManagerV1,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, WaylandState>,
    ) {
        match request {
            ext_workspace_group_handle_v1::Request::CreateWorkspace { .. } => (),
            ext_workspace_group_handle_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut WaylandState,
        _client: ClientId,
        resource: &ExtWorkspaceGroupHandleV1,
        _data: &ExtWorkspaceManagerV1,
    ) {
        let manager_state = &mut state.ext_workspace_state;
        for group_data in manager_state.workspace_groups.values_mut() {
            group_data.instances.retain(|instance| instance != resource);
        }
    }
}
