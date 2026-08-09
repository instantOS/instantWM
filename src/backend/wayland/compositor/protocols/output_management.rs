//! Server-side implementation of `wlr-output-management-unstable-v1`.
//!
//! This protocol allows clients like `wdisplays` or `wlr-randr` to enumerate
//! outputs and apply display configurations (mode, position, scale, transform,
//! enable/disable, adaptive sync).
//!
//! Smithay does not ship a built-in handler for this protocol, so we implement
//! the `GlobalDispatch` / `Dispatch` traits ourselves, following the pattern
//! used by cosmic-comp.

use std::collections::HashMap;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use smithay::output::{Mode, Output, WeakOutput};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, GlobalId},
};
use smithay::utils::{Logical, Physical, Point, Size, Transform};

use crate::backend::output::{
    AdaptiveSyncPolicy, OutputHeadConfiguration, OutputId, OutputMode as TransactionOutputMode,
    OutputTransaction, OutputTransactionId, OutputTransactionKind, OutputTransform,
};

// ---------------------------------------------------------------------------
// Public state types
// ---------------------------------------------------------------------------

/// Top-level state for the wlr-output-management protocol.
///
/// Owns the global, tracks all connected outputs ("heads"), and maintains a
/// serial counter so clients can detect stale configurations.
pub struct OutputManagementState {
    /// All outputs currently advertised to clients.
    outputs: Vec<Output>,
    /// Per-client manager instances (one per `zwlr_output_manager_v1` bind).
    instances: Vec<OutputMngrInstance>,
    /// Monotonically increasing serial, bumped whenever heads change.
    serial_counter: u32,
    /// The Wayland global for `zwlr_output_manager_v1`.  Held so we can
    /// remove it later if we ever need to tear the protocol down cleanly.
    #[allow(dead_code)]
    global: GlobalId,
    /// Cached display handle (for creating resources outside of bind).
    dh: DisplayHandle,
    pending_transactions: HashMap<OutputTransactionId, ZwlrOutputConfigurationV1>,
}

#[derive(Debug)]
pub struct OutputManagementOutputState {
    enabled: AtomicBool,
    adaptive_sync: AtomicBool,
}

impl Default for OutputManagementOutputState {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            adaptive_sync: AtomicBool::new(false),
        }
    }
}

impl OutputManagementOutputState {
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn adaptive_sync(&self) -> bool {
        self.adaptive_sync.load(Ordering::Relaxed)
    }

    pub fn set(&self, enabled: bool, adaptive_sync: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.adaptive_sync.store(adaptive_sync, Ordering::Relaxed);
    }
}

/// Global data attached to the `zwlr_output_manager_v1` global.
pub struct OutputManagementGlobalData {
    /// Filter closure — currently always-true (all clients may bind).
    _filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// One per `zwlr_output_manager_v1` resource a client has bound.
#[derive(Debug)]
struct OutputMngrInstance {
    obj: ZwlrOutputManagerV1,
    /// Heads advertised to this particular instance.
    heads: Vec<OutputHeadInstance>,
}

/// Tracks a single `zwlr_output_head_v1` resource and its associated output.
#[derive(Debug)]
struct OutputHeadInstance {
    obj: ZwlrOutputHeadV1,
    /// The Smithay output this head represents.
    output: Output,
    /// Mode resources created for this head.
    modes: Vec<ZwlrOutputModeV1>,
    initialized: bool,
    /// Whether `finished` has already been sent.  Held so we can avoid
    /// emitting `finished` twice if `release` and `destroyed` both fire.
    #[allow(dead_code)]
    finished: bool,
}

// ---------------------------------------------------------------------------
// Pending configuration types (used as resource user-data)
// ---------------------------------------------------------------------------

/// Inner state for a `zwlr_output_configuration_v1` resource.
#[derive(Debug)]
pub struct PendingConfigurationInner {
    serial: u32,
    used: bool,
    manager: ZwlrOutputManagerV1,
    /// (head resource, optional per-head config) for each head the client
    /// touched.  `Some` = enable_head, `None` = disable_head.
    heads: Vec<(ZwlrOutputHeadV1, Option<ZwlrOutputConfigurationHeadV1>)>,
}

/// Mutex-wrapped pending configuration — stored as resource user data.
pub type PendingConfiguration = Mutex<PendingConfigurationInner>;

/// Inner state for a `zwlr_output_configuration_head_v1` resource.
#[derive(Debug, Clone)]
pub struct PendingOutputConfigurationInner {
    output: WeakOutput,
    mode: Option<ModeConfiguration>,
    position: Option<Point<i32, Logical>>,
    transform: Option<Transform>,
    scale: Option<f64>,
    adaptive_sync: Option<bool>,
}

/// Mutex-wrapped per-head pending config.
pub type PendingOutputConfiguration = Mutex<PendingOutputConfigurationInner>;

/// How the client wants the mode set.
#[derive(Debug, Clone)]
enum ModeConfiguration {
    /// Use an existing `zwlr_output_mode_v1` resource (carries the `Mode` as
    /// its user data).
    Mode(ZwlrOutputModeV1),
    /// A custom mode not in the output's mode list.
    Custom {
        size: Size<i32, Physical>,
        refresh: Option<i32>,
    },
}

#[derive(Debug, Clone)]
pub struct OutputModeData {
    output: WeakOutput,
    mode: TransactionOutputMode,
}

fn valid_custom_mode(width: i32, height: i32, refresh: i32) -> bool {
    width > 0 && height > 0 && refresh >= 0
}

fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

fn transaction_mode(mode: Mode) -> TransactionOutputMode {
    TransactionOutputMode {
        width: mode.size.w,
        height: mode.size.h,
        refresh_millihertz: mode.refresh,
    }
}

fn transaction_transform(transform: Transform) -> OutputTransform {
    match transform {
        Transform::Normal => OutputTransform::Normal,
        Transform::_90 => OutputTransform::Rotate90,
        Transform::_180 => OutputTransform::Rotate180,
        Transform::_270 => OutputTransform::Rotate270,
        Transform::Flipped => OutputTransform::Flipped,
        Transform::Flipped90 => OutputTransform::Flipped90,
        Transform::Flipped180 => OutputTransform::Flipped180,
        Transform::Flipped270 => OutputTransform::Flipped270,
    }
}

fn build_transaction(
    configurations: &[(Output, Option<PendingOutputConfigurationInner>)],
) -> OutputTransaction {
    let heads = configurations
        .iter()
        .map(|(output, configuration)| match configuration {
            None => OutputHeadConfiguration {
                id: OutputId(output.name()),
                enabled: false,
                mode: output.current_mode().map(transaction_mode),
                position: crate::types::Point::new(
                    output.current_location().x,
                    output.current_location().y,
                ),
                transform: transaction_transform(output.current_transform()),
                scale: output.current_scale().fractional_scale(),
                adaptive_sync: None,
            },
            Some(configuration) => {
                let selected_mode = match &configuration.mode {
                    Some(ModeConfiguration::Mode(resource)) => {
                        resource.data::<OutputModeData>().map(|data| data.mode)
                    }
                    Some(ModeConfiguration::Custom { size, refresh }) => output
                        .modes()
                        .into_iter()
                        .map(transaction_mode)
                        .find(|candidate| {
                            candidate.width == size.w
                                && candidate.height == size.h
                                && refresh.is_none_or(|value| candidate.refresh_millihertz == value)
                        }),
                    None => output.current_mode().map(transaction_mode),
                };
                let position = configuration
                    .position
                    .unwrap_or_else(|| output.current_location());
                OutputHeadConfiguration {
                    id: OutputId(output.name()),
                    enabled: true,
                    mode: selected_mode,
                    position: crate::types::Point::new(position.x, position.y),
                    transform: transaction_transform(
                        configuration
                            .transform
                            .unwrap_or_else(|| output.current_transform()),
                    ),
                    scale: configuration
                        .scale
                        .unwrap_or_else(|| output.current_scale().fractional_scale()),
                    adaptive_sync: configuration.adaptive_sync.map(|enabled| {
                        if enabled {
                            AdaptiveSyncPolicy::Enabled
                        } else {
                            AdaptiveSyncPolicy::Disabled
                        }
                    }),
                }
            }
        })
        .collect();
    OutputTransaction { heads }
}

// ---------------------------------------------------------------------------
// Trait that WaylandState must implement
// ---------------------------------------------------------------------------

/// Handler trait bridging the protocol to instantWM's output management.
pub trait OutputManagementHandler {
    /// Access the mutable `OutputManagementState`.
    fn output_management_state(&mut self) -> &mut OutputManagementState;

    fn submit_output_transaction(
        &mut self,
        kind: OutputTransactionKind,
        transaction: OutputTransaction,
        configuration: ZwlrOutputConfigurationV1,
    );
}

// ---------------------------------------------------------------------------
// OutputManagementState implementation
// ---------------------------------------------------------------------------

impl OutputManagementState {
    /// Create the state and register the `zwlr_output_manager_v1` global.
    pub fn new<D>(dh: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
            + Dispatch<ZwlrOutputManagerV1, ()>
            + Dispatch<ZwlrOutputHeadV1, WeakOutput>
            + Dispatch<ZwlrOutputModeV1, OutputModeData>
            + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
            + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
            + OutputManagementHandler
            + 'static,
    {
        let global = dh.create_global::<D, ZwlrOutputManagerV1, _>(
            4, // max version we support
            OutputManagementGlobalData {
                _filter: Box::new(|_| true),
            },
        );

        OutputManagementState {
            outputs: Vec::new(),
            instances: Vec::new(),
            serial_counter: 0,
            global,
            dh: dh.clone(),
            pending_transactions: HashMap::new(),
        }
    }

    /// Add outputs to the advertised head list.  Sends `head` + `done` events
    /// to all connected clients.
    pub fn add_heads<'a, D>(&mut self, outputs: impl Iterator<Item = &'a Output>)
    where
        D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
            + Dispatch<ZwlrOutputManagerV1, ()>
            + Dispatch<ZwlrOutputHeadV1, WeakOutput>
            + Dispatch<ZwlrOutputModeV1, OutputModeData>
            + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
            + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
            + OutputManagementHandler
            + 'static,
    {
        let mut changed = false;
        for output in outputs {
            if self.outputs.iter().any(|o| o == output) {
                continue;
            }
            output
                .user_data()
                .insert_if_missing_threadsafe(OutputManagementOutputState::default);
            self.outputs.push(output.clone());
            changed = true;

            for instance in &mut self.instances {
                send_head_to_client::<D>(&self.dh, instance, output);
            }
        }

        if changed {
            self.serial_counter = self.serial_counter.wrapping_add(1);
            for instance in &self.instances {
                instance.obj.done(self.serial_counter);
            }
        }
    }

    /// Remove outputs from the advertised head list.  Sends `finished` events
    /// for the affected heads and a `done` event with a new serial.
    pub fn remove_heads<'a, D>(&mut self, outputs: impl Iterator<Item = &'a Output>)
    where
        D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
            + Dispatch<ZwlrOutputManagerV1, ()>
            + Dispatch<ZwlrOutputHeadV1, WeakOutput>
            + Dispatch<ZwlrOutputModeV1, OutputModeData>
            + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
            + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
            + OutputManagementHandler
            + 'static,
    {
        let to_remove: Vec<Output> = outputs
            .filter(|output| self.outputs.iter().any(|o| o == *output))
            .cloned()
            .collect();

        for output in &to_remove {
            self.outputs.retain(|o| o != output);

            for instance in &mut self.instances {
                if let Some(pos) = instance.heads.iter().position(|h| &h.output == output) {
                    let head = instance.heads.remove(pos);
                    for mode in &head.modes {
                        mode.finished();
                    }
                    head.obj.finished();
                }
            }
        }

        if !to_remove.is_empty() {
            self.serial_counter = self.serial_counter.wrapping_add(1);
            for instance in &self.instances {
                instance.obj.done(self.serial_counter);
            }
        }
    }

    /// Update the state of existing heads (e.g. after a mode or position
    /// change).  Re-sends head properties and bumps the serial.
    pub fn update_heads<'a, D>(&mut self, outputs: impl Iterator<Item = &'a Output>)
    where
        D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
            + Dispatch<ZwlrOutputManagerV1, ()>
            + Dispatch<ZwlrOutputHeadV1, WeakOutput>
            + Dispatch<ZwlrOutputModeV1, OutputModeData>
            + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
            + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
            + OutputManagementHandler
            + 'static,
    {
        let mut changed = false;
        for output in outputs {
            if !self.outputs.contains(output) {
                continue;
            }
            changed = true;
            for instance in &mut self.instances {
                if let Some(head) = instance.heads.iter_mut().find(|h| &h.output == output) {
                    update_head_state::<D>(&self.dh, head, output);
                }
            }
        }

        if changed {
            self.serial_counter = self.serial_counter.wrapping_add(1);
            for instance in &self.instances {
                instance.obj.done(self.serial_counter);
            }
        }
    }

    /// Return the list of currently tracked outputs.
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Mark a head as enabled in the protocol state.
    pub fn enable_head(&mut self, output: &Output) {
        if let Some(state) = output.user_data().get::<OutputManagementOutputState>() {
            state.enabled.store(true, Ordering::Relaxed);
        }
    }

    /// Mark a head as disabled in the protocol state.
    pub fn disable_head(&mut self, output: &Output) {
        if let Some(state) = output.user_data().get::<OutputManagementOutputState>() {
            state.enabled.store(false, Ordering::Relaxed);
        }
    }

    pub fn track_transaction(
        &mut self,
        id: OutputTransactionId,
        configuration: ZwlrOutputConfigurationV1,
    ) {
        self.pending_transactions.insert(id, configuration);
    }

    pub fn finish_transaction(&mut self, id: OutputTransactionId, succeeded: bool) {
        let Some(configuration) = self.pending_transactions.remove(&id) else {
            return;
        };
        if succeeded {
            configuration.succeeded();
        } else {
            configuration.failed();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers for sending head state to clients
// ---------------------------------------------------------------------------

/// Create a `zwlr_output_head_v1` resource for `output` on `instance` and
/// send all initial properties.
fn send_head_to_client<D>(dh: &DisplayHandle, instance: &mut OutputMngrInstance, output: &Output)
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    let Ok(client) = dh.get_client(instance.obj.id()) else {
        return;
    };

    let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, _, D>(
        dh,
        instance.obj.version(),
        output.downgrade(),
    ) else {
        return;
    };

    instance.obj.head(&head);

    let head_instance = OutputHeadInstance {
        obj: head,
        output: output.clone(),
        modes: Vec::new(),
        initialized: false,
        finished: false,
    };

    // Push the head instance first, then populate it via update_head_state.
    instance.heads.push(head_instance);
    let head_idx = instance.heads.len() - 1;
    let head = &mut instance.heads[head_idx];
    update_head_state::<D>(dh, head, output);
}

/// Send (or re-send) all head properties: name, description, physical size,
/// modes, current mode, enabled, position, transform, scale, adaptive sync.
///
/// `head` is a `&mut` to a single head inside `instance.heads`.  We deliberately
/// avoid taking `&mut OutputMngrInstance` here so callers can pass a head
/// borrowed from inside the instance without a double mutable borrow.
fn update_head_state<D>(dh: &DisplayHandle, head: &mut OutputHeadInstance, output: &Output)
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    let obj = &head.obj;

    if !head.initialized {
        obj.name(output.name());
        obj.description(output.description());

        let physical = output.physical_properties();
        if physical.size.w != 0 || physical.size.h != 0 {
            obj.physical_size(physical.size.w, physical.size.h);
        }

        if obj.version() >= zwlr_output_head_v1::EVT_MAKE_SINCE {
            if physical.make != "Unknown" {
                obj.make(physical.make.clone());
            }
            if physical.model != "Unknown" {
                obj.model(physical.model.clone());
            }
            if physical.serial_number != "Unknown" {
                obj.serial_number(physical.serial_number.clone());
            }
        }
        head.initialized = true;
    }

    // Modes — remove stale ones, add new ones
    let output_modes = output.modes();

    // Remove modes that no longer exist on the output
    head.modes.retain(|m| {
        let still_exists = m.data::<OutputModeData>().is_some_and(|data| {
            output_modes
                .iter()
                .copied()
                .map(transaction_mode)
                .any(|mode| mode == data.mode)
        });
        if !still_exists {
            m.finished();
        }
        still_exists
    });

    // Add or update modes. The current-mode event is emitted later because
    // the protocol forbids sending it for a disabled head.
    let mut current_mode = None;
    for output_mode in output_modes {
        let existing = head.modes.iter().find(|m| {
            m.data::<OutputModeData>()
                .is_some_and(|d| d.mode == transaction_mode(output_mode))
        });

        let mode_obj = if let Some(existing) = existing {
            existing
        } else {
            // Create a new mode resource
            let Ok(client) = dh.get_client(obj.id()) else {
                continue;
            };
            let Ok(mode) = client.create_resource::<ZwlrOutputModeV1, _, D>(
                dh,
                obj.version().min(3),
                OutputModeData {
                    output: output.downgrade(),
                    mode: transaction_mode(output_mode),
                },
            ) else {
                continue;
            };
            obj.mode(&mode);
            mode.size(output_mode.size.w, output_mode.size.h);
            mode.refresh(output_mode.refresh);
            if output.preferred_mode().is_some_and(|p| p == output_mode) {
                mode.preferred();
            }
            head.modes.push(mode);
            head.modes.last().unwrap()
        };

        // Send current_mode if this is the active one
        if output.current_mode().is_some_and(|c| c == output_mode) {
            current_mode = Some(mode_obj.clone());
        }
    }

    // Enabled state
    let output_state = output.user_data().get::<OutputManagementOutputState>();
    let enabled = output_state.is_none_or(OutputManagementOutputState::enabled);
    obj.enabled(if enabled { 1 } else { 0 });

    // Position, transform, scale (only if enabled)
    if enabled {
        if let Some(mode) = current_mode {
            obj.current_mode(&mode);
        }
        let loc = output.current_location();
        obj.position(loc.x, loc.y);
        obj.transform(output.current_transform().into());
        obj.scale(output.current_scale().fractional_scale());
    }

    // Adaptive sync (protocol version >= 4 for the event)
    if obj.version() >= zwlr_output_head_v1::EVT_ADAPTIVE_SYNC_SINCE {
        obj.adaptive_sync(
            if output_state.is_some_and(OutputManagementOutputState::adaptive_sync) {
                zwlr_output_head_v1::AdaptiveSyncState::Enabled
            } else {
                zwlr_output_head_v1::AdaptiveSyncState::Disabled
            },
        );
    }
}

// ---------------------------------------------------------------------------
// GlobalDispatch and Dispatch implementations
// ---------------------------------------------------------------------------

impl<D> GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData, D> for OutputManagementState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    fn bind(
        state: &mut D,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _global_data: &OutputManagementGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let mut instance = OutputMngrInstance {
            obj: data_init.init(resource, ()),
            heads: Vec::new(),
        };

        let mgmt_state = state.output_management_state();
        for output in &mgmt_state.outputs {
            send_head_to_client::<D>(dh, &mut instance, output);
        }
        instance.obj.done(mgmt_state.serial_counter);
        mgmt_state.instances.push(instance);
    }

    fn can_view(client: Client, global_data: &OutputManagementGlobalData) -> bool {
        (global_data._filter)(&client)
    }
}

impl<D> Dispatch<ZwlrOutputManagerV1, (), D> for OutputManagementState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                data_init.init(
                    id,
                    PendingConfiguration::new(PendingConfigurationInner {
                        serial,
                        used: false,
                        manager: obj.clone(),
                        heads: Vec::new(),
                    }),
                );
            }
            zwlr_output_manager_v1::Request::Stop => {
                let mgmt_state = state.output_management_state();
                mgmt_state.instances.retain(|i| i.obj != *obj);
                obj.finished();
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, obj: &ZwlrOutputManagerV1, _data: &()) {
        let mgmt_state = state.output_management_state();
        mgmt_state.instances.retain(|i| i.obj != *obj);
    }
}

impl<D> Dispatch<ZwlrOutputHeadV1, WeakOutput, D> for OutputManagementState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputHeadV1,
        request: zwlr_output_head_v1::Request,
        _data: &WeakOutput,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let zwlr_output_head_v1::Request::Release = request {
            let mgmt_state = state.output_management_state();
            for instance in &mut mgmt_state.instances {
                instance.heads.retain(|h| &h.obj != obj);
            }
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, obj: &ZwlrOutputHeadV1, _data: &WeakOutput) {
        let mgmt_state = state.output_management_state();
        for instance in &mut mgmt_state.instances {
            instance.heads.retain(|h| &h.obj != obj);
        }
    }
}

impl<D> Dispatch<ZwlrOutputModeV1, OutputModeData, D> for OutputManagementState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputModeV1,
        request: zwlr_output_mode_v1::Request,
        _data: &OutputModeData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let zwlr_output_mode_v1::Request::Release = request {
            let mgmt_state = state.output_management_state();
            for instance in &mut mgmt_state.instances {
                for head in &mut instance.heads {
                    head.modes.retain(|m| m != obj);
                }
            }
        }
    }
}

impl<D> Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration, D> for OutputManagementState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &PendingConfiguration,
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let mut pending = data.lock().unwrap();
                if pending.used {
                    obj.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyUsed,
                        "configuration object was already applied or tested".to_string(),
                    );
                    return;
                }
                if pending.heads.iter().any(|(h, _)| *h == head) {
                    obj.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                        "head was already configured".to_string(),
                    );
                    return;
                }

                let Some(output) = head.data::<WeakOutput>().cloned() else {
                    obj.post_error(
                        zwlr_output_configuration_v1::Error::UnconfiguredHead,
                        "head is no longer available".to_string(),
                    );
                    return;
                };
                let conf_head = data_init.init(
                    id,
                    PendingOutputConfiguration::new(PendingOutputConfigurationInner {
                        output,
                        mode: None,
                        position: None,
                        transform: None,
                        scale: None,
                        adaptive_sync: None,
                    }),
                );
                pending.heads.push((head, Some(conf_head)));
            }
            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                let mut pending = data.lock().unwrap();
                if pending.used {
                    obj.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyUsed,
                        "configuration object was already applied or tested".to_string(),
                    );
                    return;
                }
                if pending.heads.iter().any(|(h, _)| *h == head) {
                    obj.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                        "head was already configured".to_string(),
                    );
                    return;
                }

                pending.heads.push((head, None));
            }
            x @ zwlr_output_configuration_v1::Request::Apply
            | x @ zwlr_output_configuration_v1::Request::Test => {
                let mut pending = data.lock().unwrap();

                if pending.used {
                    return obj.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyUsed,
                        "Configuration object was used already".to_string(),
                    );
                }
                pending.used = true;

                let mgmt_state = state.output_management_state();
                if pending.serial != mgmt_state.serial_counter {
                    obj.cancelled();
                    return;
                }

                // Build the final configuration list
                let final_conf = match pending
                    .heads
                    .iter()
                    .map(|(head, conf)| {
                        // Find the output for this head resource
                        let output = mgmt_state
                            .instances
                            .iter()
                            .filter(|inst| inst.obj == pending.manager)
                            .find_map(|inst| {
                                inst.heads
                                    .iter()
                                    .find(|h| h.obj == *head)
                                    .map(|h| h.output.clone())
                            })
                            .ok_or(zwlr_output_configuration_v1::Error::UnconfiguredHead)?;

                        match conf {
                            Some(conf_head) => {
                                let pending_inner =
                                    conf_head.data::<PendingOutputConfiguration>().unwrap();
                                let inner = pending_inner.lock().unwrap();
                                Ok((output, Some(inner.clone())))
                            }
                            None => Ok((output, None)),
                        }
                    })
                    .collect::<Result<
                        Vec<(Output, Option<PendingOutputConfigurationInner>)>,
                        zwlr_output_configuration_v1::Error,
                    >>() {
                    Ok(conf) => conf,
                    Err(code) => {
                        return obj
                            .post_error(code, "head is not part of this manager".to_string());
                    }
                };

                // Check that all outputs are configured
                let configured_outputs: Vec<&Output> = final_conf.iter().map(|(o, _)| o).collect();
                if configured_outputs.len() != mgmt_state.outputs.len()
                    || configured_outputs
                        .iter()
                        .any(|o| !mgmt_state.outputs.contains(o))
                {
                    return obj.post_error(
                        zwlr_output_configuration_v1::Error::UnconfiguredHead,
                        "configuration must include every head".to_string(),
                    );
                }

                // Check that selected modes still exist
                if final_conf.iter().any(|(o, c)| match c {
                    Some(PendingOutputConfigurationInner {
                        mode: Some(ModeConfiguration::Mode(m)),
                        ..
                    }) => {
                        let mode_data = m.data::<OutputModeData>();
                        mode_data.is_none_or(|data| {
                            data.output.upgrade().as_ref() != Some(o)
                                || !o
                                    .modes()
                                    .into_iter()
                                    .map(transaction_mode)
                                    .any(|mode| mode == data.mode)
                        })
                    }
                    _ => false,
                }) {
                    obj.cancelled();
                    return;
                }

                let kind = if matches!(x, zwlr_output_configuration_v1::Request::Test) {
                    OutputTransactionKind::Test
                } else {
                    OutputTransactionKind::Apply
                };
                state.submit_output_transaction(kind, build_transaction(&final_conf), obj.clone());
            }
            zwlr_output_configuration_v1::Request::Destroy => {
                let pending = data.lock().unwrap();
                for (_, head) in &pending.heads {
                    if let Some(head) = head {
                        let _ = dh.backend_handle().destroy_object::<D>(&head.id());
                    }
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        obj: &ZwlrOutputConfigurationV1,
        _data: &PendingConfiguration,
    ) {
        state
            .output_management_state()
            .pending_transactions
            .retain(|_, configuration| configuration != obj);
    }
}

impl<D> Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration, D>
    for OutputManagementState
where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &PendingOutputConfiguration,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let mut pending = data.lock().unwrap();
        match request {
            zwlr_output_configuration_head_v1::Request::SetMode { mode } => {
                if pending.mode.is_some() {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::AlreadySet,
                        "mode already set".to_string(),
                    );
                    return;
                }
                let valid = mode.data::<OutputModeData>().is_some_and(|data| {
                    data.output == pending.output && data.output.upgrade().is_some()
                });
                if !valid {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidMode,
                        "mode does not belong to this head".to_string(),
                    );
                    return;
                }
                pending.mode = Some(ModeConfiguration::Mode(mode));
            }
            zwlr_output_configuration_head_v1::Request::SetCustomMode {
                width,
                height,
                refresh,
            } => {
                if pending.mode.is_some() {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::AlreadySet,
                        "mode already set".to_string(),
                    );
                    return;
                }
                if !valid_custom_mode(width, height, refresh) {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidCustomMode,
                        "custom mode dimensions must be positive and refresh non-negative"
                            .to_string(),
                    );
                    return;
                }
                pending.mode = Some(ModeConfiguration::Custom {
                    size: Size::from((width, height)),
                    refresh: if refresh == 0 { None } else { Some(refresh) },
                });
            }
            zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => {
                if pending.position.is_some() {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::AlreadySet,
                        "position already set".to_string(),
                    );
                    return;
                }
                pending.position = Some(Point::from((x, y)));
            }
            zwlr_output_configuration_head_v1::Request::SetScale { scale } => {
                if pending.scale.is_some() {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::AlreadySet,
                        "scale already set".to_string(),
                    );
                    return;
                }
                if !valid_scale(scale) {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidScale,
                        "scale must be finite and greater than zero".to_string(),
                    );
                    return;
                }
                pending.scale = Some(scale);
            }
            zwlr_output_configuration_head_v1::Request::SetTransform { transform } => {
                if pending.transform.is_some() {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::AlreadySet,
                        "transform already set".to_string(),
                    );
                    return;
                }
                pending.transform = Some(match transform.into_result() {
                    Ok(t) => t.into(),
                    Err(err) => {
                        obj.post_error(
                            zwlr_output_configuration_head_v1::Error::InvalidTransform,
                            format!("Invalid transform: {err:?}"),
                        );
                        return;
                    }
                });
            }
            zwlr_output_configuration_head_v1::Request::SetAdaptiveSync { state: sync_state } => {
                if pending.adaptive_sync.is_some() {
                    obj.post_error(
                        zwlr_output_configuration_head_v1::Error::AlreadySet,
                        "adaptive sync already set".to_string(),
                    );
                    return;
                }
                pending.adaptive_sync = Some(match sync_state.into_result() {
                    Ok(zwlr_output_head_v1::AdaptiveSyncState::Enabled) => true,
                    Ok(zwlr_output_head_v1::AdaptiveSyncState::Disabled) => false,
                    Err(err) => {
                        obj.post_error(
                            zwlr_output_configuration_head_v1::Error::InvalidAdaptiveSyncState,
                            format!("invalid adaptive sync state: {err:?}"),
                        );
                        return;
                    }
                    Ok(_) => {
                        obj.post_error(
                            zwlr_output_configuration_head_v1::Error::InvalidAdaptiveSyncState,
                            "unsupported adaptive sync state".to_string(),
                        );
                        return;
                    }
                });
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Delegation macro
// ---------------------------------------------------------------------------
//
// Wire `WaylandState` (the main `D` of the smithay dispatching state) into
// `OutputManagementState` by generating the `Dispatch`/`GlobalDispatch`
// impls on `WaylandState` that forward to the impls on `OutputManagementState`.
//
// This is the missing piece that makes `OutputManagementState::new::<D>` and
// the `where D: Dispatch<...>` bounds on the dispatch helpers actually
// satisfied: once the macro expands, `WaylandState` does implement
// `Dispatch<ZwlrOutputManagerV1, ()>` etc.
//
// The pattern is straight from niri's `protocols/output_management.rs`.

//
// `#[macro_export]` puts the macro at the crate root so the absolute paths
// below resolve no matter where the macro is invoked from.  This matches
// niri's `delegate_output_management!` pattern.

#[macro_export]
macro_rules! delegate_output_management {
    ($(@< $( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+ >)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1: $crate::backend::wayland::compositor::protocols::output_management::OutputManagementGlobalData
        ] => $crate::backend::wayland::compositor::protocols::output_management::OutputManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1: ()
        ] => $crate::backend::wayland::compositor::protocols::output_management::OutputManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_head_v1::ZwlrOutputHeadV1: smithay::output::WeakOutput
        ] => $crate::backend::wayland::compositor::protocols::output_management::OutputManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_mode_v1::ZwlrOutputModeV1: $crate::backend::wayland::compositor::protocols::output_management::OutputModeData
        ] => $crate::backend::wayland::compositor::protocols::output_management::OutputManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_v1::ZwlrOutputConfigurationV1: $crate::backend::wayland::compositor::protocols::output_management::PendingConfiguration
        ] => $crate::backend::wayland::compositor::protocols::output_management::OutputManagementState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1: $crate::backend::wayland::compositor::protocols::output_management::PendingOutputConfiguration
        ] => $crate::backend::wayland::compositor::protocols::output_management::OutputManagementState);
    };
}

#[cfg(test)]
mod tests {
    use super::{OutputManagementOutputState, valid_custom_mode, valid_scale};

    #[test]
    fn custom_modes_require_positive_dimensions_and_non_negative_refresh() {
        assert!(valid_custom_mode(1920, 1080, 0));
        assert!(valid_custom_mode(1920, 1080, 60_000));
        assert!(!valid_custom_mode(0, 1080, 60_000));
        assert!(!valid_custom_mode(1920, -1, 60_000));
        assert!(!valid_custom_mode(1920, 1080, -1));
    }

    #[test]
    fn scales_must_be_finite_and_positive() {
        assert!(valid_scale(1.0));
        assert!(valid_scale(1.25));
        assert!(!valid_scale(0.0));
        assert!(!valid_scale(-1.0));
        assert!(!valid_scale(f64::NAN));
        assert!(!valid_scale(f64::INFINITY));
    }

    #[test]
    fn enabled_and_adaptive_sync_are_tracked_independently_of_output_mode() {
        let state = OutputManagementOutputState::default();
        assert!(state.enabled());
        assert!(!state.adaptive_sync());

        state.set(false, false);
        assert!(!state.enabled());
        state.set(true, true);
        assert!(state.enabled());
        assert!(state.adaptive_sync());
    }
}
