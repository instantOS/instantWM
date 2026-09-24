//! Server-side implementation of `wlr-output-management-unstable-v1`.
//!
//! This protocol advertises Wayland outputs and translates client configuration
//! requests into backend-neutral output transactions.

mod configuration;
mod configuration_dispatch;
mod dispatch;
mod head;
mod state;

pub use configuration::{PendingConfiguration, PendingOutputConfiguration};
pub use state::{OutputManagementGlobalData, OutputManagementOutputState, OutputManagementState};

use smithay::output::WeakOutput;
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::ZwlrOutputConfigurationV1, zwlr_output_head_v1::ZwlrOutputHeadV1,
    zwlr_output_manager_v1::ZwlrOutputManagerV1, zwlr_output_mode_v1::ZwlrOutputModeV1,
};
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};

use crate::backend::output::{OutputTransaction, OutputTransactionKind};

pub use head::OutputModeData;

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

/// Dispatch requirements shared by resource creation and every protocol handler.
pub trait OutputManagementDispatch:
    GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
    + Dispatch<ZwlrOutputManagerV1, ()>
    + Dispatch<ZwlrOutputHeadV1, WeakOutput>
    + Dispatch<ZwlrOutputModeV1, OutputModeData>
    + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
    + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
    + OutputManagementHandler
    + 'static
{
}

impl<T> OutputManagementDispatch for T where
    T: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, WeakOutput>
        + Dispatch<ZwlrOutputModeV1, OutputModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingOutputConfiguration>
        + OutputManagementHandler
        + 'static
{
}

/// Forward the protocol's Wayland dispatch implementations to this module.
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
    use super::OutputManagementOutputState;
    use super::configuration::{valid_custom_mode, valid_scale};

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
