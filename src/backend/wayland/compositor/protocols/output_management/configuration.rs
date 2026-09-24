use std::sync::Mutex;

use smithay::output::{Output, WeakOutput};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::ZwlrOutputHeadV1,
    zwlr_output_manager_v1::ZwlrOutputManagerV1,
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Physical, Point, Size, Transform};

use crate::backend::output::{
    AdaptiveSyncPolicy, OutputHeadConfiguration, OutputId, OutputMode as TransactionOutputMode,
    OutputTransaction,
};
use crate::backend::wayland::output::from_smithay_transform;

use super::OutputManagementState;
use super::head::{OutputModeData, transaction_mode};

/// Inner state for a `zwlr_output_configuration_v1` resource.
#[derive(Debug)]
pub struct PendingConfigurationInner {
    pub(super) serial: u32,
    pub(super) used: bool,
    pub(super) manager: ZwlrOutputManagerV1,
    /// (head resource, optional per-head config) for each head the client
    /// touched.  `Some` = enable_head, `None` = disable_head.
    pub(super) heads: Vec<(ZwlrOutputHeadV1, Option<ZwlrOutputConfigurationHeadV1>)>,
}

impl PendingConfigurationInner {
    pub(super) fn check_head_available(
        &self,
        object: &ZwlrOutputConfigurationV1,
        head: &ZwlrOutputHeadV1,
    ) -> bool {
        if self.used {
            object.post_error(
                zwlr_output_configuration_v1::Error::AlreadyUsed,
                "configuration object was already applied or tested".to_string(),
            );
            return false;
        }
        if self.heads.iter().any(|(configured, _)| configured == head) {
            object.post_error(
                zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                "head was already configured".to_string(),
            );
            return false;
        }
        true
    }
}

/// Mutex-wrapped pending configuration — stored as resource user data.
pub type PendingConfiguration = Mutex<PendingConfigurationInner>;

/// Inner state for a `zwlr_output_configuration_head_v1` resource.
#[derive(Debug, Clone)]
pub struct PendingOutputConfigurationInner {
    pub(super) output: WeakOutput,
    pub(super) mode: Option<ModeConfiguration>,
    pub(super) position: Option<Point<i32, Logical>>,
    pub(super) transform: Option<Transform>,
    pub(super) scale: Option<f64>,
    pub(super) adaptive_sync: Option<bool>,
}

/// Mutex-wrapped per-head pending config.
pub type PendingOutputConfiguration = Mutex<PendingOutputConfigurationInner>;

/// How the client wants the mode set.
#[derive(Debug, Clone)]
pub(super) enum ModeConfiguration {
    /// Use an existing `zwlr_output_mode_v1` resource (carries the `Mode` as
    /// its user data).
    Mode(ZwlrOutputModeV1),
    /// A custom mode not in the output's mode list.
    Custom {
        size: Size<i32, Physical>,
        refresh: Option<i32>,
    },
}

pub(super) fn valid_custom_mode(width: i32, height: i32, refresh: i32) -> bool {
    width > 0 && height > 0 && refresh >= 0
}

pub(super) fn valid_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrepareError {
    StaleSerial,
    UnconfiguredHead,
    Incomplete,
    StaleMode,
    UnsupportedCustomMode,
}

fn resolve_custom_mode(
    modes: impl IntoIterator<Item = TransactionOutputMode>,
    size: Size<i32, Physical>,
    refresh: Option<i32>,
) -> Result<TransactionOutputMode, PrepareError> {
    modes
        .into_iter()
        .find(|candidate| {
            candidate.width == size.w
                && candidate.height == size.h
                && refresh.is_none_or(|value| candidate.refresh_millihertz == value)
        })
        .ok_or(PrepareError::UnsupportedCustomMode)
}

fn build_transaction(
    configurations: &[(Output, Option<PendingOutputConfigurationInner>)],
) -> Result<OutputTransaction, PrepareError> {
    let heads = configurations
        .iter()
        .map(|(output, configuration)| match configuration {
            None => Ok(OutputHeadConfiguration {
                id: OutputId(output.name()),
                enabled: false,
                mode: output.current_mode().map(transaction_mode),
                position: crate::types::Point::new(
                    output.current_location().x,
                    output.current_location().y,
                ),
                transform: from_smithay_transform(output.current_transform()),
                scale: output.current_scale().fractional_scale(),
                adaptive_sync: None,
            }),
            Some(configuration) => {
                let selected_mode = match &configuration.mode {
                    Some(ModeConfiguration::Mode(resource)) => Some(
                        resource
                            .data::<OutputModeData>()
                            .ok_or(PrepareError::StaleMode)?
                            .mode,
                    ),
                    Some(ModeConfiguration::Custom { size, refresh }) => Some(resolve_custom_mode(
                        output.modes().into_iter().map(transaction_mode),
                        *size,
                        *refresh,
                    )?),
                    None => output.current_mode().map(transaction_mode),
                };
                let position = configuration
                    .position
                    .unwrap_or_else(|| output.current_location());
                Ok(OutputHeadConfiguration {
                    id: OutputId(output.name()),
                    enabled: true,
                    mode: selected_mode,
                    position: crate::types::Point::new(position.x, position.y),
                    transform: from_smithay_transform(
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
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OutputTransaction { heads })
}

pub(super) fn prepare_transaction(
    state: &OutputManagementState,
    pending: &PendingConfigurationInner,
) -> Result<OutputTransaction, PrepareError> {
    if pending.serial != state.serial_counter {
        return Err(PrepareError::StaleSerial);
    }

    let instance = state
        .instances
        .iter()
        .find(|instance| instance.obj == pending.manager)
        .ok_or(PrepareError::UnconfiguredHead)?;
    let configurations = pending
        .heads
        .iter()
        .map(|(head, configuration)| {
            let output = instance
                .heads
                .iter()
                .find(|candidate| candidate.obj == *head)
                .map(|candidate| candidate.output.clone())
                .ok_or(PrepareError::UnconfiguredHead)?;
            let configuration = configuration
                .as_ref()
                .map(|resource| {
                    resource
                        .data::<PendingOutputConfiguration>()
                        .map(|data| data.lock().unwrap().clone())
                        .ok_or(PrepareError::UnconfiguredHead)
                })
                .transpose()?;
            Ok((output, configuration))
        })
        .collect::<Result<Vec<_>, PrepareError>>()?;

    if configurations.len() != state.outputs.len()
        || configurations
            .iter()
            .any(|(output, _)| !state.outputs.contains(output))
    {
        return Err(PrepareError::Incomplete);
    }

    for (output, configuration) in &configurations {
        if let Some(PendingOutputConfigurationInner {
            mode: Some(ModeConfiguration::Mode(resource)),
            ..
        }) = configuration
        {
            let Some(data) = resource.data::<OutputModeData>() else {
                return Err(PrepareError::StaleMode);
            };
            if data.output.upgrade().as_ref() != Some(output)
                || !output
                    .modes()
                    .into_iter()
                    .map(transaction_mode)
                    .any(|mode| mode == data.mode)
            {
                return Err(PrepareError::StaleMode);
            }
        }
    }

    build_transaction(&configurations)
}

pub(super) fn report_prepare_error(object: &ZwlrOutputConfigurationV1, error: PrepareError) {
    match error {
        PrepareError::StaleSerial | PrepareError::StaleMode => object.cancelled(),
        PrepareError::UnconfiguredHead => object.post_error(
            zwlr_output_configuration_v1::Error::UnconfiguredHead,
            "head is not part of this manager or is no longer available".to_string(),
        ),
        PrepareError::Incomplete => object.post_error(
            zwlr_output_configuration_v1::Error::UnconfiguredHead,
            "configuration must include every head".to_string(),
        ),
        PrepareError::UnsupportedCustomMode => object.failed(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PrepareError, resolve_custom_mode};
    use crate::backend::output::OutputMode;
    use smithay::utils::Size;

    #[test]
    fn custom_mode_resolution_requires_an_advertised_mode_and_honors_refresh() {
        let modes = [
            OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 60_000,
            },
            OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 120_000,
            },
        ];
        let size = Size::from((1920, 1080));

        assert_eq!(
            resolve_custom_mode(modes, size, Some(120_000)),
            Ok(modes[1])
        );
        assert_eq!(resolve_custom_mode(modes, size, None), Ok(modes[0]));
        assert_eq!(
            resolve_custom_mode(modes, size, Some(75_000)),
            Err(PrepareError::UnsupportedCustomMode),
        );
        assert_eq!(
            resolve_custom_mode(modes, Size::from((2560, 1440)), None),
            Err(PrepareError::UnsupportedCustomMode),
        );
    }
}
