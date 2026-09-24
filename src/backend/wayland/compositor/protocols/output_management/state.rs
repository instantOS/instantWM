use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_v1::ZwlrOutputConfigurationV1, zwlr_output_head_v1::ZwlrOutputHeadV1,
    zwlr_output_manager_v1::ZwlrOutputManagerV1, zwlr_output_mode_v1::ZwlrOutputModeV1,
};
use smithay::reexports::wayland_server::{DisplayHandle, backend::GlobalId};

use crate::backend::output::OutputTransactionId;

use super::OutputManagementDispatch;
use super::head::{send_head_to_client, update_head_state};

/// Top-level state for the wlr-output-management protocol.
///
/// Owns the global, tracks all connected outputs ("heads"), and maintains a
/// serial counter so clients can detect stale configurations.
pub struct OutputManagementState {
    /// All outputs currently advertised to clients.
    pub(super) outputs: Vec<Output>,
    /// Per-client manager instances (one per `zwlr_output_manager_v1` bind).
    pub(super) instances: Vec<OutputMngrInstance>,
    /// Monotonically increasing serial, bumped whenever heads change.
    pub(super) serial_counter: u32,
    /// The Wayland global for `zwlr_output_manager_v1`.  Held so we can
    /// remove it later if we ever need to tear the protocol down cleanly.
    #[allow(dead_code)]
    global: GlobalId,
    /// Cached display handle (for creating resources outside of bind).
    dh: DisplayHandle,
    pub(super) pending_transactions: HashMap<OutputTransactionId, ZwlrOutputConfigurationV1>,
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
pub struct OutputManagementGlobalData;

/// One per `zwlr_output_manager_v1` resource a client has bound.
#[derive(Debug)]
pub(super) struct OutputMngrInstance {
    pub(super) obj: ZwlrOutputManagerV1,
    /// Heads advertised to this particular instance.
    pub(super) heads: Vec<OutputHeadInstance>,
}

/// Tracks a single `zwlr_output_head_v1` resource and its associated output.
#[derive(Debug)]
pub(super) struct OutputHeadInstance {
    pub(super) obj: ZwlrOutputHeadV1,
    /// The Smithay output this head represents.
    pub(super) output: Output,
    /// Mode resources created for this head.
    pub(super) modes: Vec<ZwlrOutputModeV1>,
}

impl OutputManagementState {
    fn notify_changed(&mut self) {
        self.serial_counter = self.serial_counter.wrapping_add(1);
        for instance in &self.instances {
            instance.obj.done(self.serial_counter);
        }
    }

    /// Create the state and register the `zwlr_output_manager_v1` global.
    pub fn new<D>(dh: &DisplayHandle) -> Self
    where
        D: OutputManagementDispatch,
    {
        let global = dh.create_global::<D, ZwlrOutputManagerV1, _>(
            4, // max version we support
            OutputManagementGlobalData,
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
        D: OutputManagementDispatch,
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
            self.notify_changed();
        }
    }

    /// Remove outputs from the advertised head list.  Sends `finished` events
    /// for the affected heads and a `done` event with a new serial.
    pub fn remove_heads<'a, D>(&mut self, outputs: impl Iterator<Item = &'a Output>)
    where
        D: OutputManagementDispatch,
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
            self.notify_changed();
        }
    }

    /// Update the state of existing heads (e.g. after a mode or position
    /// change).  Re-sends head properties and bumps the serial.
    pub fn update_heads<'a, D>(&mut self, outputs: impl Iterator<Item = &'a Output>)
    where
        D: OutputManagementDispatch,
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
            self.notify_changed();
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
