use smithay::output::{Mode, Output, WeakOutput};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};
use smithay::reexports::wayland_server::{DisplayHandle, Resource};

use crate::backend::output::OutputMode as TransactionOutputMode;

use super::OutputManagementDispatch;
use super::state::{OutputHeadInstance, OutputManagementOutputState, OutputMngrInstance};

#[derive(Debug, Clone)]
pub struct OutputModeData {
    pub(super) output: WeakOutput,
    pub(super) mode: TransactionOutputMode,
}

pub(super) fn transaction_mode(mode: Mode) -> TransactionOutputMode {
    mode.into()
}

/// Create a `zwlr_output_head_v1` resource for `output` on `instance` and
/// send all initial properties.
pub(super) fn send_head_to_client<D>(
    dh: &DisplayHandle,
    instance: &mut OutputMngrInstance,
    output: &Output,
) where
    D: OutputManagementDispatch,
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
    };

    // Keep the head before sending events so later requests can find it.
    instance.heads.push(head_instance);
    let head_idx = instance.heads.len() - 1;
    let head = &mut instance.heads[head_idx];
    send_initial_head_properties(&head.obj, output);
    update_head_state::<D>(dh, head, output);
}

fn send_initial_head_properties(obj: &ZwlrOutputHeadV1, output: &Output) {
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
}

/// Synchronize advertised mode resources and send changing head properties.
pub(super) fn update_head_state<D>(
    dh: &DisplayHandle,
    head: &mut OutputHeadInstance,
    output: &Output,
) where
    D: OutputManagementDispatch,
{
    let obj = &head.obj;

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
