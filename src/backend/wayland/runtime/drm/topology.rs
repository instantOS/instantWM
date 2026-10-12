//! Reconcile hotplugged DRM devices and their logical outputs.

use smithay::reexports::calloop::LoopHandle;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc;

use super::layout::{
    DrmLayoutState, compact_drm_automatic_layout, recover_pointer_after_output_change,
    refresh_drm_layout_state,
};
use super::{DrmRuntimeEvent, devices::DrmDevices, scheduling::DrmLoopState};
use crate::backend::output::{OutputId, OutputPowerError};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::render::drm::{
    OutputSurfaceEntry, OutputTopology, ScanoutId, probe_usable_connectors,
    reconcile_output_surfaces, refresh_output_modes,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn reconcile_drm_outputs(
    state: &mut WaylandState,
    output_surfaces: &mut Vec<OutputSurfaceEntry>,
    devices: &Rc<RefCell<DrmDevices>>,
    loop_handle: &LoopHandle<'_, WaylandState>,
    runtime_event_tx: &mpsc::Sender<DrmRuntimeEvent>,
    loop_state: &mut DrmLoopState,
    layout_state: &mut DrmLayoutState,
    input_dimensions: &Rc<Cell<crate::types::Rect>>,
) -> bool {
    let mut devices = devices.borrow_mut();
    let changes = match devices.discover(&loop_state.removed_devices) {
        Ok(changes) => changes,
        Err(error) => {
            log::warn!("Cannot enumerate DRM devices after hotplug: {error}");
            return false;
        }
    };
    let removed_devices = changes.removed;
    loop_state.removed_devices.clear();
    let mut snapshots: Vec<_> = devices
        .devices
        .iter()
        .filter(|(id, _)| !removed_devices.contains(id))
        .filter_map(
            |(&id, device)| match probe_usable_connectors(&device.manager) {
                Ok(connectors) => Some((id, connectors)),
                Err(error) => {
                    log::warn!(
                        "failed to probe {} after hotplug: {error}",
                        device.path.display()
                    );
                    None
                }
            },
        )
        .collect();
    let old_connectors: HashSet<_> = output_surfaces
        .iter()
        .map(|entry| (entry.device, entry.connector))
        .collect();
    let mut retained = Vec::with_capacity(output_surfaces.len());
    let mut removed = Vec::new();
    for entry in output_surfaces.drain(..) {
        // A failed device probe preserves its outputs. Device removal is
        // authoritative even when its fd can no longer answer connector ioctls.
        let disconnected = removed_devices.contains(&entry.device)
            || snapshots
                .iter()
                .find(|(id, _)| *id == entry.device)
                .is_some_and(|(_, usable)| {
                    !usable.iter().any(|info| info.handle() == entry.connector)
                });
        if disconnected {
            removed.push(entry);
        } else {
            retained.push(entry);
        }
    }
    *output_surfaces = retained;
    let retained_connectors: HashSet<_> = output_surfaces
        .iter()
        .map(|entry| (entry.device, entry.connector))
        .collect();
    retire_disconnected_outputs(state, loop_state, removed);

    // All surfaces and client references are gone before their device closes.
    for id in removed_devices {
        devices.retire(id, loop_handle);
    }
    for path in changes.added {
        let initialized = state
            .graphics
            .as_mut()
            .expect("DRM graphics initialized")
            .with_renderer(|renderer| {
                devices.add(path.clone(), renderer, loop_handle, runtime_event_tx)
            });
        match initialized {
            Ok(id) => match probe_usable_connectors(devices.manager(id)) {
                Ok(connectors) => snapshots.push((id, connectors)),
                Err(error) => log::warn!(
                    "Cannot probe new scanout device {}: {error}",
                    path.display()
                ),
            },
            Err(error) => log::warn!(
                "Cannot initialize scanout device {}: {error:#}",
                path.display()
            ),
        }
    }
    let mut moved_any = false;
    let mut modes_changed = false;
    for (id, usable) in snapshots {
        let (mut local, other): (Vec<_>, Vec<_>) = output_surfaces
            .drain(..)
            .partition(|entry| entry.device == id);
        *output_surfaces = other;
        let in_flight: Vec<_> = loop_state
            .pending_crtcs
            .iter()
            .filter(|scanout| scanout.device == id)
            .map(|scanout| scanout.crtc)
            .collect();
        let occupied_names: Vec<_> = output_surfaces
            .iter()
            .map(|entry| entry.output.name())
            .collect();
        let reconciliation = state
            .graphics
            .as_mut()
            .expect("DRM graphics initialized")
            .with_renderer(|renderer| {
                reconcile_output_surfaces(
                    devices.manager(id),
                    renderer,
                    &mut state.native,
                    &mut local,
                    OutputTopology {
                        device: id,
                        connectors: &usable,
                        in_flight: &in_flight,
                        occupied_names: &occupied_names,
                    },
                )
            });
        loop_state.topology_dirty |= reconciliation.deferred;
        moved_any |= !reconciliation.moved.is_empty();
        for (old, _) in &reconciliation.moved {
            loop_state.remove_output(ScanoutId {
                device: id,
                crtc: *old,
            });
        }
        for (_, new) in &reconciliation.moved {
            loop_state.add_output(ScanoutId {
                device: id,
                crtc: *new,
            });
        }
        for entry in &mut local {
            if let Some(info) = usable.iter().find(|info| info.handle() == entry.connector) {
                modes_changed |= refresh_output_modes(entry, info);
            }
        }
        output_surfaces.extend(local);
    }
    let added: Vec<_> = output_surfaces
        .iter()
        .filter(|entry| !retained_connectors.contains(&(entry.device, entry.connector)))
        .collect();
    for entry in &added {
        state
            .native
            .space
            .map_output(&entry.output, (entry.rect.x, entry.rect.y));
        loop_state.add_output(entry.scanout());
    }
    state
        .native
        .output_management_state
        .add_heads::<WaylandState>(added.iter().map(|entry| &entry.output));
    let connector_set_unchanged = added.is_empty()
        && old_connectors.len() == output_surfaces.len()
        && output_surfaces
            .iter()
            .all(|entry| old_connectors.contains(&(entry.device, entry.connector)));
    if connector_set_unchanged && !moved_any && !modes_changed {
        return false;
    }
    // Keep automatic placement stable in device lifetime / connector order.
    output_surfaces.sort_by_key(|entry| (entry.device, u32::from(entry.connector)));
    compact_drm_automatic_layout(state, output_surfaces);
    refresh_drm_layout_state(state, output_surfaces, layout_state);
    input_dimensions.set(layout_state.layout);
    crate::monitor::refresh_monitor_layout(&mut state.ctx());
    state
        .native
        .push_command(crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones);
    crate::monitor::apply_monitor_config(&mut state.ctx());
    // Re-evaluate suppression even without a new lid event: the availability
    // of an external display may have changed while the laptop stayed closed.
    state.native.runtime.lid_policy_dirty = true;
    recover_pointer_after_output_change(state);
    true
}

/// Remove protocol and compositor references before closing a scanout device.
fn retire_disconnected_outputs(
    state: &mut WaylandState,
    loop_state: &mut DrmLoopState,
    removed: Vec<OutputSurfaceEntry>,
) {
    if removed.is_empty() {
        return;
    }
    state
        .native
        .output_management_state
        .remove_heads::<WaylandState>(removed.iter().map(|entry| &entry.output));
    for mut entry in removed {
        let name = entry.output.name();
        log::info!("Output {name}: disconnected");
        state
            .native
            .runtime
            .lid_output_policy
            .forget(&OutputId(name.clone()));
        if let Some(id) = entry.pending_power_on.take() {
            state.native.runtime.output_power.complete(
                id,
                (
                    OutputId(name.clone()),
                    Err(OutputPowerError::Unavailable(name.clone())),
                ),
            );
        }
        entry.surface.take();
        loop_state.remove_output(entry.scanout());
        state.native.space.unmap_output(&entry.output);
        state.native.set_output_global_enabled(&entry.output, false);
        state.native.fail_pending_captures_for_output(&entry.output);
        state.native.runtime.output_power_modes.remove(&name);
        state.native.runtime.output_metadata.remove(&name);
        state.native.runtime.output_position_sources.remove(&name);
        let cancelled = state.native.output_power_state.fail_output(&name);
        state.native.runtime.output_power.cancel(&cancelled);
    }
    state.native.drop_unavailable_mirrors();
}
