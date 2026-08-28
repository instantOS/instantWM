//! Server-side implementation of `wlr-output-power-management-unstable-v1`.
//!
//! The protocol controls physical output power without changing the logical
//! output layout. Backend-native work is submitted through the neutral output
//! power service; protocol events are sent only after that work completes.

use std::collections::HashMap;

use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
    zwlr_output_power_v1::{self, ZwlrOutputPowerV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, GlobalId},
};

use crate::backend::output::{
    CompletedOutputPowerRequest, OutputId, OutputPowerMode, OutputPowerRequestId,
};

const VERSION: u32 = 1;

pub trait OutputPowerHandler {
    fn output_power_state(&mut self) -> &mut OutputPowerState;
    fn output_power_mode(&self, output: &Output) -> Option<OutputPowerMode>;
    fn submit_output_power_request(
        &mut self,
        output: OutputId,
        mode: OutputPowerMode,
    ) -> OutputPowerRequestId;
    fn cancel_output_power_requests(&mut self, requests: &[OutputPowerRequestId]);
}

pub struct OutputPowerState {
    #[allow(dead_code)]
    global: GlobalId,
    controls: HashMap<ZwlrOutputPowerV1, OutputPowerControl>,
    pending: HashMap<OutputPowerRequestId, ZwlrOutputPowerV1>,
}

struct OutputPowerControl {
    output_name: String,
    mode: OutputPowerMode,
}

pub struct OutputPowerData {
    output: WeakOutput,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OutputPowerGlobalData;

impl OutputPowerState {
    pub fn new<D>(dh: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData> + 'static,
    {
        let global =
            dh.create_global::<D, ZwlrOutputPowerManagerV1, _>(VERSION, OutputPowerGlobalData);
        Self {
            global,
            controls: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    pub fn complete(
        &mut self,
        completed: CompletedOutputPowerRequest,
    ) -> Vec<OutputPowerRequestId> {
        let Some(resource) = self.pending.remove(&completed.id) else {
            return Vec::new();
        };
        match completed.result {
            Ok(mode) => {
                let Some(control) = self.controls.get_mut(&resource) else {
                    return Vec::new();
                };
                if control.output_name != completed.output.0 {
                    return Vec::new();
                }
                if control.mode != mode {
                    resource.mode(protocol_mode(mode));
                    control.mode = mode;
                }
                Vec::new()
            }
            Err(error) => {
                log::warn!(
                    "output power request for {} failed: {error}",
                    completed.output.0
                );
                resource.failed();
                self.remove_control(&resource)
            }
        }
    }

    /// Invalidate the exclusive controller when an output leaves compositor
    /// space through output-management.
    pub fn fail_output(&mut self, output_name: &str) -> Vec<OutputPowerRequestId> {
        let resources: Vec<_> = self
            .controls
            .iter()
            .filter_map(|(resource, control)| {
                (control.output_name == output_name).then_some(resource.clone())
            })
            .collect();
        let mut cancelled = Vec::new();
        for resource in resources {
            resource.failed();
            cancelled.extend(self.remove_control(&resource));
        }
        cancelled
    }

    fn remove_control(&mut self, resource: &ZwlrOutputPowerV1) -> Vec<OutputPowerRequestId> {
        self.controls.remove(resource);
        let cancelled = self
            .pending
            .iter()
            .filter_map(|(id, pending)| (pending == resource).then_some(*id))
            .collect();
        self.pending.retain(|_, pending| pending != resource);
        cancelled
    }
}

impl<D> GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData, D> for OutputPowerState
where
    D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData>
        + Dispatch<ZwlrOutputPowerManagerV1, ()>
        + 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _global_data: &OutputPowerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D> Dispatch<ZwlrOutputPowerManagerV1, (), D> for OutputPowerState
where
    D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerGlobalData>
        + Dispatch<ZwlrOutputPowerManagerV1, ()>
        + Dispatch<ZwlrOutputPowerV1, OutputPowerData>
        + OutputPowerHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _obj: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } => {
                let output = Output::from_resource(&output);
                let weak = output.as_ref().map(Output::downgrade).unwrap_or_default();
                let resource = data_init.init(id, OutputPowerData { output: weak });

                let Some(output) = output else {
                    resource.failed();
                    return;
                };
                let output_name = output.name();
                let duplicate = state
                    .output_power_state()
                    .controls
                    .values()
                    .any(|control| control.output_name == output_name);
                let Some(mode) = (!duplicate)
                    .then(|| state.output_power_mode(&output))
                    .flatten()
                else {
                    resource.failed();
                    return;
                };

                resource.mode(protocol_mode(mode));
                state
                    .output_power_state()
                    .controls
                    .insert(resource, OutputPowerControl { output_name, mode });
            }
            zwlr_output_power_manager_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

impl<D> Dispatch<ZwlrOutputPowerV1, OutputPowerData, D> for OutputPowerState
where
    D: Dispatch<ZwlrOutputPowerV1, OutputPowerData> + OutputPowerHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        obj: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        data: &OutputPowerData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_v1::Request::SetMode { mode } => {
                let mode = match mode.into_result() {
                    Ok(zwlr_output_power_v1::Mode::Off) => OutputPowerMode::Off,
                    Ok(zwlr_output_power_v1::Mode::On) => OutputPowerMode::On,
                    Ok(_) | Err(_) => {
                        obj.post_error(
                            zwlr_output_power_v1::Error::InvalidMode,
                            "invalid output power mode".to_string(),
                        );
                        return;
                    }
                };
                if !state.output_power_state().controls.contains_key(obj) {
                    return;
                }
                let Some(output) = data.output.upgrade() else {
                    obj.failed();
                    let cancelled = state.output_power_state().remove_control(obj);
                    state.cancel_output_power_requests(&cancelled);
                    return;
                };
                if state.output_power_mode(&output).is_none() {
                    obj.failed();
                    let cancelled = state.output_power_state().remove_control(obj);
                    state.cancel_output_power_requests(&cancelled);
                    return;
                }

                let id = state.submit_output_power_request(output.name().into(), mode);
                state.output_power_state().pending.insert(id, obj.clone());
            }
            zwlr_output_power_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        obj: &ZwlrOutputPowerV1,
        _data: &OutputPowerData,
    ) {
        let cancelled = state.output_power_state().remove_control(obj);
        state.cancel_output_power_requests(&cancelled);
    }
}

fn protocol_mode(mode: OutputPowerMode) -> zwlr_output_power_v1::Mode {
    match mode {
        OutputPowerMode::Off => zwlr_output_power_v1::Mode::Off,
        OutputPowerMode::On => zwlr_output_power_v1::Mode::On,
    }
}

#[macro_export]
macro_rules! delegate_output_power {
    ($(@< $( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+ >)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: $crate::backend::wayland::compositor::protocols::output_power::OutputPowerGlobalData
        ] => $crate::backend::wayland::compositor::protocols::output_power::OutputPowerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1: ()
        ] => $crate::backend::wayland::compositor::protocols::output_power::OutputPowerState);
        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1::ZwlrOutputPowerV1: $crate::backend::wayland::compositor::protocols::output_power::OutputPowerData
        ] => $crate::backend::wayland::compositor::protocols::output_power::OutputPowerState);
    };
}
