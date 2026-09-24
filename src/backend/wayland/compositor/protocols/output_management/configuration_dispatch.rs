use smithay::output::WeakOutput;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, Resource, backend::ClientId,
};
use smithay::utils::{Point, Size};

use crate::backend::output::OutputTransactionKind;

use super::configuration::{
    ModeConfiguration, PendingConfiguration, PendingOutputConfiguration,
    PendingOutputConfigurationInner, prepare_transaction, report_prepare_error, valid_custom_mode,
    valid_scale,
};
use super::head::OutputModeData;
use super::{OutputManagementDispatch, OutputManagementState};

impl<D> Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration, D> for OutputManagementState
where
    D: OutputManagementDispatch,
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
                if !pending.check_head_available(obj, &head) {
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
                if !pending.check_head_available(obj, &head) {
                    return;
                }

                pending.heads.push((head, None));
            }
            x @ zwlr_output_configuration_v1::Request::Apply
            | x @ zwlr_output_configuration_v1::Request::Test => {
                let transaction = {
                    let mut pending = data.lock().unwrap();
                    if pending.used {
                        obj.post_error(
                            zwlr_output_configuration_v1::Error::AlreadyUsed,
                            "configuration object was already applied or tested".to_string(),
                        );
                        return;
                    }
                    pending.used = true;
                    prepare_transaction(state.output_management_state(), &pending)
                };
                let transaction = match transaction {
                    Ok(transaction) => transaction,
                    Err(error) => {
                        report_prepare_error(obj, error);
                        return;
                    }
                };
                let kind = if matches!(x, zwlr_output_configuration_v1::Request::Test) {
                    OutputTransactionKind::Test
                } else {
                    OutputTransactionKind::Apply
                };
                state.submit_output_transaction(kind, transaction, obj.clone());
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
    D: OutputManagementDispatch,
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
