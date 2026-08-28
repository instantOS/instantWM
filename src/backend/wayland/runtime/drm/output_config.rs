//! Application of wlr-output-management transactions and output power
//! requests to the DRM backend.

use smithay::backend::renderer::gles::GlesRenderer;
use std::sync::{Arc, Mutex};

use crate::backend::BackendVrrSupport;
use crate::backend::output::{
    AdaptiveSyncPolicy, OutputHeadCapabilities, OutputHeadConfiguration, OutputHeadSnapshot,
    OutputId, OutputMode as TransactionOutputMode, OutputPowerError, OutputPowerMode,
    OutputSnapshot, OutputTransaction, OutputTransactionError, OutputTransactionKind,
};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::render::drm::{ManagedDrmOutputManager, OutputSurfaceEntry};
use crate::config::config_toml::VrrMode;

use super::DrmLoopState;

fn requested_mode(
    entry: &OutputSurfaceEntry,
    config: &OutputHeadConfiguration,
) -> Option<smithay::reexports::drm::control::Mode> {
    let requested = config.mode?;
    entry.modes.iter().find_map(|(output_mode, drm_mode)| {
        (output_mode.size.w == requested.width
            && output_mode.size.h == requested.height
            && output_mode.refresh == requested.refresh_millihertz)
            .then_some(*drm_mode)
    })
}

fn transaction_snapshot(
    transaction: &OutputTransaction,
    output_surfaces: &[OutputSurfaceEntry],
) -> OutputSnapshot {
    OutputSnapshot {
        heads: transaction
            .heads
            .iter()
            .map(|configuration| {
                let entry = output_surfaces
                    .iter()
                    .find(|entry| entry.output.name() == configuration.id.0);
                let modes = entry
                    .map(|entry| {
                        entry
                            .modes
                            .iter()
                            .map(|(mode, _)| TransactionOutputMode {
                                width: mode.size.w,
                                height: mode.size.h,
                                refresh_millihertz: mode.refresh,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let adaptive_sync_policy = configuration.adaptive_sync.unwrap_or_else(|| {
                    entry.map_or(AdaptiveSyncPolicy::Disabled, |entry| {
                        match entry.configured_vrr_mode {
                            VrrMode::Off => AdaptiveSyncPolicy::Disabled,
                            VrrMode::On => AdaptiveSyncPolicy::Enabled,
                            VrrMode::Auto => AdaptiveSyncPolicy::Automatic,
                        }
                    })
                });
                OutputHeadSnapshot {
                    configuration: configuration.clone(),
                    modes,
                    adaptive_sync_policy,
                    adaptive_sync_enabled: entry.is_some_and(|entry| entry.vrr_enabled),
                }
            })
            .collect(),
    }
}

fn output_capabilities(output_surfaces: &[OutputSurfaceEntry]) -> Vec<OutputHeadCapabilities> {
    output_surfaces
        .iter()
        .map(|entry| OutputHeadCapabilities {
            id: entry.output.name().as_str().into(),
            modes: entry
                .modes
                .iter()
                .map(|(mode, _)| TransactionOutputMode {
                    width: mode.size.w,
                    height: mode.size.h,
                    refresh_millihertz: mode.refresh,
                })
                .collect(),
            adaptive_sync: !matches!(entry.vrr_support, BackendVrrSupport::Unsupported),
        })
        .collect()
}

fn requested_vrr(
    entry: &OutputSurfaceEntry,
    configuration: &OutputHeadConfiguration,
) -> (VrrMode, bool) {
    let policy = configuration
        .adaptive_sync
        .unwrap_or(match entry.configured_vrr_mode {
            VrrMode::Off => AdaptiveSyncPolicy::Disabled,
            VrrMode::On => AdaptiveSyncPolicy::Enabled,
            VrrMode::Auto => AdaptiveSyncPolicy::Automatic,
        });
    match policy {
        AdaptiveSyncPolicy::Disabled => (VrrMode::Off, false),
        AdaptiveSyncPolicy::Enabled => (VrrMode::On, true),
        AdaptiveSyncPolicy::Automatic => (VrrMode::Auto, entry.vrr_enabled),
    }
}

pub(super) fn process_output_configurations(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    output_manager: &Arc<Mutex<ManagedDrmOutputManager>>,
    renderer: &mut GlesRenderer,
    loop_state: &mut DrmLoopState,
) {
    if !state.runtime.output_transactions.has_pending() {
        return;
    }

    let render_elements = smithay::backend::drm::output::DrmOutputRenderElements::<
        GlesRenderer,
        crate::backend::wayland::render::drm::DrmExtras,
    >::default();
    let capabilities = output_capabilities(output_surfaces);

    while let Some(pending) = state.runtime.output_transactions.take_next_pending() {
        if let Err(error) = pending.transaction.validate(&capabilities) {
            state
                .runtime
                .output_transactions
                .complete(pending, Err(error));
            continue;
        }
        if pending.kind == OutputTransactionKind::Test {
            let snapshot = transaction_snapshot(&pending.transaction, output_surfaces);
            state
                .runtime
                .output_transactions
                .complete(pending, Ok(snapshot));
            continue;
        }

        let requested: Vec<_> = pending
            .transaction
            .heads
            .iter()
            .map(|config| {
                let index = output_surfaces
                    .iter()
                    .position(|entry| entry.output.name() == config.id.0)
                    .expect("validated output disappeared before application");
                (
                    index,
                    config,
                    requested_mode(&output_surfaces[index], config),
                )
            })
            .collect();
        if requested.iter().any(|(index, _, _)| {
            loop_state
                .pending_crtcs
                .contains(&output_surfaces[*index].crtc)
        }) {
            state.runtime.output_transactions.requeue(pending);
            break;
        }

        let mut newly_enabled = Vec::new();
        let mut changed_modes = Vec::new();
        let mut changed_vrr = Vec::new();
        let mut applied = true;

        for (index, config, mode) in &requested {
            if !config.enabled {
                continue;
            }
            let entry = &mut output_surfaces[*index];
            let mode = mode.expect("enabled configurations were prevalidated");
            let was_enabled = entry.surface.is_some();
            if let Some(surface) = entry.surface.as_mut() {
                let current_mode = surface.with_compositor(|compositor| compositor.current_mode());
                if current_mode != mode {
                    if surface.use_mode(mode, renderer, &render_elements).is_err() {
                        applied = false;
                        break;
                    }
                    changed_modes.push((*index, current_mode));
                }
            } else {
                let mut manager = output_manager.lock().unwrap();
                match manager.lock().initialize_output(
                    entry.crtc,
                    mode,
                    &[entry.connector],
                    &entry.output,
                    None,
                    renderer,
                    &render_elements,
                ) {
                    Ok(surface) => {
                        entry.surface = Some(surface);
                        newly_enabled.push(*index);
                    }
                    Err(error) => {
                        log::warn!("failed to enable output {}: {error:?}", entry.output.name());
                        applied = false;
                        break;
                    }
                }
            }
            let (_, adaptive_sync) = requested_vrr(entry, config);
            let current_vrr = entry
                .surface
                .as_ref()
                .expect("output was enabled above")
                .with_compositor(|compositor| compositor.vrr_enabled());
            if current_vrr != adaptive_sync {
                if entry
                    .surface
                    .as_ref()
                    .expect("output was enabled above")
                    .with_compositor(|compositor| compositor.use_vrr(adaptive_sync))
                    .is_err()
                {
                    applied = false;
                    break;
                }
                if was_enabled {
                    changed_vrr.push((*index, current_vrr));
                }
            }
        }

        if !applied {
            for index in newly_enabled {
                output_surfaces[index].surface.take();
            }
            for (index, old_mode) in changed_modes {
                if let Some(surface) = output_surfaces[index].surface.as_mut() {
                    let _ = surface.use_mode(old_mode, renderer, &render_elements);
                }
            }
            for (index, old_vrr) in changed_vrr {
                if let Some(surface) = output_surfaces[index].surface.as_ref() {
                    let _ = surface.with_compositor(|compositor| compositor.use_vrr(old_vrr));
                }
            }
            state.runtime.output_transactions.complete(
                pending,
                Err(OutputTransactionError::Backend(
                    "DRM could not commit the requested output state".to_string(),
                )),
            );
            continue;
        }

        for (index, config, _) in &requested {
            let entry = &mut output_surfaces[*index];
            if !config.enabled {
                if let Some(id) = entry.pending_power_on.take() {
                    state.runtime.output_power.complete_by_id(
                        id,
                        OutputId(entry.output.name()),
                        Err(OutputPowerError::Unavailable(entry.output.name())),
                    );
                }
                entry.surface.take();
                entry.enabled = false;
                entry.powered = false;
                entry.vrr_enabled = false;
                state
                    .runtime
                    .output_power_modes
                    .remove(&entry.output.name());
            } else {
                let newly_enabled = !entry.enabled;
                entry.enabled = true;
                if newly_enabled {
                    entry.powered = true;
                    state
                        .runtime
                        .output_power_modes
                        .insert(entry.output.name(), OutputPowerMode::On);
                }
                let (mode, enabled) = requested_vrr(entry, config);
                entry.vrr_enabled = enabled;
                entry.configured_vrr_mode = mode;
            }
        }
        let snapshot = transaction_snapshot(&pending.transaction, output_surfaces);
        state
            .runtime
            .output_transactions
            .complete(pending, Ok(snapshot));
        loop_state.mark_all_dirty();
        // Project the authoritative state before attempting another apply.
        break;
    }
}

pub(super) fn process_output_power_requests(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    loop_state: &mut DrmLoopState,
) {
    if !state.runtime.output_power.has_pending() {
        return;
    }

    while let Some(request) = state.runtime.output_power.take_next_pending() {
        let Some(entry) = output_surfaces
            .iter_mut()
            .find(|entry| entry.output.name() == request.output.0)
        else {
            let name = request.output.0.clone();
            state
                .runtime
                .output_power
                .complete(request, Err(OutputPowerError::Unavailable(name)));
            continue;
        };
        if !entry.enabled || entry.surface.is_none() {
            let name = entry.output.name();
            state
                .runtime
                .output_power
                .complete(request, Err(OutputPowerError::Unavailable(name)));
            continue;
        }
        if entry.pending_power_on.is_some() || loop_state.pending_crtcs.contains(&entry.crtc) {
            state.runtime.output_power.requeue(request);
            break;
        }

        let current = if entry.powered {
            OutputPowerMode::On
        } else {
            OutputPowerMode::Off
        };
        if current == request.mode {
            state.runtime.output_power.complete(request, Ok(current));
            continue;
        }

        match request.mode {
            OutputPowerMode::Off => {
                let result = entry
                    .surface
                    .as_ref()
                    .expect("enabled DRM output has a surface")
                    .with_compositor(|compositor| compositor.clear());
                match result {
                    Ok(()) => {
                        entry.powered = false;
                        state
                            .runtime
                            .output_power_modes
                            .insert(entry.output.name(), OutputPowerMode::Off);
                        state
                            .runtime
                            .output_power
                            .complete(request, Ok(OutputPowerMode::Off));
                    }
                    Err(error) => {
                        state.runtime.output_power.complete(
                            request,
                            Err(OutputPowerError::Backend(format!("{error:?}"))),
                        );
                        break;
                    }
                }
            }
            OutputPowerMode::On => {
                entry.powered = true;
                entry.pending_power_on = Some(request.id);
                loop_state.mark_dirty(entry.crtc);
                // Completion is delayed until render_outputs queues the first
                // frame, which is the commit that re-enables a cleared CRTC.
            }
        }
    }
}
