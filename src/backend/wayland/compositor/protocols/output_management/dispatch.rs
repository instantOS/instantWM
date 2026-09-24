use smithay::output::WeakOutput;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, backend::ClientId,
};

use super::configuration::{PendingConfiguration, PendingConfigurationInner};
use super::head::{OutputModeData, send_head_to_client};
use super::state::OutputMngrInstance;
use super::{OutputManagementDispatch, OutputManagementGlobalData, OutputManagementState};

impl<D> GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData, D> for OutputManagementState
where
    D: OutputManagementDispatch,
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

    fn can_view(_client: Client, _global_data: &OutputManagementGlobalData) -> bool {
        true
    }
}

impl<D> Dispatch<ZwlrOutputManagerV1, (), D> for OutputManagementState
where
    D: OutputManagementDispatch,
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
    D: OutputManagementDispatch,
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
    D: OutputManagementDispatch,
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
