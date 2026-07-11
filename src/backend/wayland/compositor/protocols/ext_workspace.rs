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

pub fn refresh(state: &mut WaylandState) {
    // Performance Optimization: Zero-Allocation Fast-Path Early-Return if nothing changed.
    let mut changed = false;
    {
        let protocol_state = &state.ext_workspace_state;
        let outputs_count = state.space.outputs().count();

        if outputs_count != protocol_state.last_output_names.len() {
            changed = true;
        } else {
            // Check if any output name differs
            for (i, output) in state.space.outputs().enumerate() {
                if output.name() != protocol_state.last_output_names[i] {
                    changed = true;
                    break;
                }
            }
        }

        if !changed {
            if let Some(globals) = state.globals() {
                for output in state.space.outputs() {
                    let output_name = output.name();
                    let last_tag = protocol_state.last_tags.get(&output_name).copied();
                    let last_urgent = protocol_state.last_urgent_tags.get(&output_name).copied();
                    let last_occupied =
                        protocol_state.last_occupied_tags.get(&output_name).copied();

                    if let Some(mon) = globals
                        .model
                        .monitors
                        .iter_all()
                        .find(|m| m.name == output_name)
                    {
                        // Compute urgent_mask for this monitor
                        let mut urgent_mask = crate::types::TagMask::EMPTY;
                        for &win in &mon.clients {
                            if let Some(c) = globals.model.clients.get(&win)
                                && c.is_urgent
                            {
                                urgent_mask = urgent_mask | c.tags;
                            }
                        }

                        let occupied_mask = mon.occupied_tags(globals.model.clients.map());

                        if Some(mon.selected_tags()) != last_tag
                            || Some(urgent_mask) != last_urgent
                            || Some(occupied_mask) != last_occupied
                        {
                            changed = true;
                            break;
                        }
                    } else {
                        changed = true;
                        break;
                    }
                }
            } else {
                changed = true;
            }
        }
        if !changed {
            for group_data in protocol_state.workspace_groups.values() {
                for group in &group_data.instances {
                    if let Some(client) = group.client()
                        && let Some(ud) = group.data::<ExtWorkspaceGroupUserData>()
                        && let Some(output) =
                            state.space.outputs().find(|o| o.name() == ud.output_name)
                    {
                        for wl_output in output.client_outputs(&client) {
                            let already_sent =
                                ud.sent_outputs.lock().unwrap().contains(&wl_output.id());
                            if !already_sent {
                                changed = true;
                                break;
                            }
                        }
                    }
                    if changed {
                        break;
                    }
                }
                if changed {
                    break;
                }
            }
        }
    }

    if !changed {
        return;
    }

    // Now that we know a change occurred, gather all physical monitors and construct caches
    let active_outputs: Vec<Output> = state.space.outputs().cloned().collect();
    let active_output_names: Vec<String> = active_outputs.iter().map(|o| o.name()).collect();

    let mut current_tags = HashMap::new();
    let mut current_urgent_tags = HashMap::new();
    let mut current_occupied_tags = HashMap::new();

    if let Some(globals) = state.globals() {
        for output_name in &active_output_names {
            if let Some(mon) = globals
                .model
                .monitors
                .iter_all()
                .find(|m| m.name == *output_name)
            {
                current_tags.insert(output_name.clone(), mon.selected_tags());

                // Urgency mapping: A tag is urgent if any client placed on it has is_urgent == true.
                let mut urgent_mask = crate::types::TagMask::EMPTY;
                for &win in &mon.clients {
                    if let Some(c) = globals.model.clients.get(&win)
                        && c.is_urgent
                    {
                        urgent_mask = urgent_mask | c.tags;
                    }
                }
                current_urgent_tags.insert(output_name.clone(), urgent_mask);

                // Occupied tags mapping
                let occupied = mon.occupied_tags(globals.model.clients.map());
                current_occupied_tags.insert(output_name.clone(), occupied);
            }
        }
    }

    // Now that we know a change occurred, build monitors_info to update protocol resources
    let mut monitors_info = HashMap::new();
    if let Some(globals) = state.globals() {
        for output_name in &active_output_names {
            if let Some(mon) = globals
                .model
                .monitors
                .iter_all()
                .find(|m| m.name == *output_name)
            {
                let selected_tags = mon.selected_tags();
                let tag_names: Vec<String> = mon.tags.iter().map(|t| t.name.clone()).collect();
                monitors_info.insert(output_name.clone(), (selected_tags, tag_names));
            }
        }
    }

    let protocol_state = &mut state.ext_workspace_state;

    // Cache current state for the next tick
    protocol_state.last_output_names = active_output_names.clone();
    protocol_state.last_tags = current_tags.clone();
    protocol_state.last_urgent_tags = current_urgent_tags.clone();
    protocol_state.last_occupied_tags = current_occupied_tags.clone();

    let mut changed = false;

    // 2. Remove workspace groups for outputs that no longer exist.
    // NOTE: This assumes protocol_state.workspace_groups and protocol_state.workspaces
    // are stored as separate disjoint fields in ExtWorkspaceManagerState, allowing us to
    // read workspaces while performing a retain on workspace_groups.
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
                            if workspace
                                .data::<ExtWorkspaceUserData>()
                                .map(|ud| &ud.manager)
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

    // 3. Remove workspaces for outputs that no longer exist
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

    // 4. For each active monitor, update workspaces and workspace groups
    for output in active_outputs {
        let output_name = output.name();

        let (selected_tags, tag_names) =
            monitors_info.get(&output_name).cloned().unwrap_or_else(|| {
                (
                    crate::types::TagMask::EMPTY,
                    (1..=9).map(|i| i.to_string()).collect(),
                )
            });

        let urgent_tags = current_urgent_tags
            .get(&output_name)
            .cloned()
            .unwrap_or(crate::types::TagMask::EMPTY);

        let occupied_tags = current_occupied_tags
            .get(&output_name)
            .cloned()
            .unwrap_or(crate::types::TagMask::EMPTY);

        // Add/Refresh workspaces for this output
        for (tag_idx, name) in tag_names.into_iter().enumerate() {
            let is_active = selected_tags.contains(tag_idx + 1);
            let is_urgent = urgent_tags.contains(tag_idx + 1);
            let is_occupied = occupied_tags.contains(tag_idx + 1);

            let mut ws_state = ext_workspace_handle_v1::State::empty();
            if is_active {
                ws_state |= ext_workspace_handle_v1::State::Active;
            }
            if is_urgent {
                ws_state |= ext_workspace_handle_v1::State::Urgent;
            }
            if !is_active && !is_occupied {
                ws_state |= ext_workspace_handle_v1::State::Hidden;
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
                            ws_data.add_instance(
                                &protocol_state.display,
                                &client,
                                manager,
                                &output_name,
                                tag_idx,
                            );
                        }
                    }
                    if let Some(group_data) = protocol_state.workspace_groups.get(&output_name) {
                        for group in &group_data.instances {
                            if let Some(manager) = group.data::<ExtWorkspaceManagerV1>() {
                                for workspace in &ws_data.instances {
                                    if workspace
                                        .data::<ExtWorkspaceUserData>()
                                        .map(|ud| &ud.manager)
                                        == Some(manager)
                                    {
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
                                if workspace
                                    .data::<ExtWorkspaceUserData>()
                                    .map(|ud| &ud.manager)
                                    == Some(manager)
                                {
                                    group.workspace_enter(workspace);
                                }
                            }
                        }
                    }
                }
            }
            protocol_state
                .workspace_groups
                .insert(output_name.clone(), group_data);
            changed = true;
        }

        if let Some(group_data) = protocol_state.workspace_groups.get_mut(&output_name) {
            for group in &group_data.instances {
                if let Some(client) = group.client() {
                    let mut group_changed = false;
                    for wl_output in output.client_outputs(&client) {
                        let already_sent =
                            if let Some(ud) = group.data::<ExtWorkspaceGroupUserData>() {
                                ud.sent_outputs.lock().unwrap().contains(&wl_output.id())
                            } else {
                                false
                            };
                        if !already_sent {
                            group.output_enter(&wl_output);
                            if let Some(ud) = group.data::<ExtWorkspaceGroupUserData>() {
                                ud.sent_outputs.lock().unwrap().insert(wl_output.id());
                            }
                            group_changed = true;
                        }
                    }
                    if group_changed {
                        changed = true;
                    }
                }
            }
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
