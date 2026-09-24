//! Output/display management for WaylandState.
//!
//! This module contains output-related methods on WaylandState,
//! including creating outputs, listing displays, and configuring display modes.

use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::utils::Transform;
use std::sync::Mutex;

use crate::backend::BackendVrrSupport;
use crate::backend::output::{
    AdaptiveSyncPolicy, MonitorModeRequest, OutputHeadConfiguration, OutputId,
    OutputMode as TransactionOutputMode, OutputPlacement, OutputPositionSource, OutputSnapshot,
    OutputTransaction, OutputTransactionError, OutputTransactionKind, OutputTransform, RequestId,
    plan_automatic_output_positions, position_after,
};
use crate::backend::wayland::output::{from_smithay_transform, to_smithay_transform};
use crate::config::config_toml::VrrMode;
use crate::types::{MonitorPosition, Point, Rect, Size};

use super::protocols::output_management::OutputManagementOutputState;
use super::state::WaylandState;

struct OutputGlobal(Mutex<Option<GlobalId>>);

fn smithay_mode(mode: TransactionOutputMode) -> OutputMode {
    OutputMode {
        size: (mode.width, mode.height).into(),
        refresh: mode.refresh_millihertz,
    }
}

fn logical_output_size(configuration: &OutputHeadConfiguration) -> Size {
    let mode = configuration.mode.unwrap_or(TransactionOutputMode {
        width: WaylandState::MIN_WL_DIM,
        height: WaylandState::MIN_WL_DIM,
        refresh_millihertz: 60_000,
    });
    let (width, height) = if matches!(
        configuration.transform,
        OutputTransform::Rotate90
            | OutputTransform::Rotate270
            | OutputTransform::Flipped90
            | OutputTransform::Flipped270
    ) {
        (mode.height, mode.width)
    } else {
        (mode.width, mode.height)
    };
    let scale = if configuration.scale.is_finite() && configuration.scale > 0.0 {
        configuration.scale
    } else {
        1.0
    };
    Size::new(
        (f64::from(width) / scale).round() as i32,
        (f64::from(height) / scale).round() as i32,
    )
}

fn head_rect(head: &OutputHeadConfiguration) -> Rect {
    let size = logical_output_size(head);
    Rect::new(head.position.x, head.position.y, size.w, size.h)
}

/// Pairs an applied snapshot realizes: both heads enabled, and the mirror
/// already pinned to its source's scale. A snapshot from a transaction
/// submitted before `mirrors` changed may not carry the pin yet; its heads
/// stay ordinary outputs until the pinning transaction applies, since the
/// mirror is rendered from elements built at the source's scale.
fn realized_mirrors(
    mirrors: &crate::output_mirror::MirrorMap,
    snapshot: &OutputSnapshot,
) -> std::collections::HashMap<String, String> {
    let scale = |name: &str| {
        snapshot
            .heads
            .iter()
            .find(|head| head.configuration.enabled && head.configuration.id.0 == name)
            .map(|head| head.configuration.scale)
    };
    mirrors
        .active_pairs(|name| scale(name).is_some())
        .filter(|(mirror, target)| scale(mirror) == scale(&target.source))
        .map(|(mirror, target)| (mirror.to_string(), target.source.clone()))
        .collect()
}

/// Close every layer surface placed on `output`. Returns whether any existed.
fn close_layer_surfaces(output: &Output) -> bool {
    let mut map = smithay::desktop::layer_map_for_output(output);
    let layers: Vec<_> = map.layers().cloned().collect();
    for layer in &layers {
        layer.layer_surface().send_close();
        map.unmap_layer(layer);
    }
    !layers.is_empty()
}

impl WaylandState {
    pub(crate) fn set_output_global_enabled(&self, output: &Output, enabled: bool) {
        let Some(global) = output.user_data().get::<OutputGlobal>() else {
            return;
        };
        let mut global_id = global.0.lock().unwrap();
        match (enabled, global_id.take()) {
            (true, None) => {
                *global_id = Some(output.create_global::<Self>(&self.display_handle));
            }
            (true, Some(existing)) => {
                *global_id = Some(existing);
            }
            (false, Some(existing)) => {
                self.display_handle.remove_global::<Self>(existing);
            }
            (false, None) => {}
        }
    }

    fn apply_output_snapshot(&mut self, snapshot: &OutputSnapshot) {
        let mut changed_outputs = Vec::new();
        for head in &snapshot.heads {
            let Some(output) = self
                .output_management_state
                .outputs()
                .iter()
                .find(|output| output.name() == head.configuration.id.0)
                .cloned()
            else {
                continue;
            };
            let config = &head.configuration;
            let available_modes: Vec<_> = head.modes.iter().copied().map(smithay_mode).collect();
            for mode in output.modes() {
                if !available_modes.contains(&mode) {
                    output.delete_mode(mode);
                }
            }
            for mode in available_modes {
                output.add_mode(mode);
            }
            let output_state = output
                .user_data()
                .get::<OutputManagementOutputState>()
                .expect("output-management state is initialized when a head is added");
            self.project_output_vrr_state(
                &output.name(),
                match head.adaptive_sync_policy {
                    AdaptiveSyncPolicy::Disabled => VrrMode::Off,
                    AdaptiveSyncPolicy::Enabled => VrrMode::On,
                    AdaptiveSyncPolicy::Automatic => VrrMode::Auto,
                },
                head.adaptive_sync_enabled,
            );
            let location = (config.position.x, config.position.y).into();
            output.change_current_state(
                config.mode.map(smithay_mode),
                Some(to_smithay_transform(config.transform)),
                Some(Scale::Fractional(config.scale)),
                Some(location),
            );
            if !config.enabled {
                self.runtime.output_power_modes.remove(&output.name());
                let cancelled = self.output_power_state.fail_output(&output.name());
                self.runtime.output_power.cancel(&cancelled);
                output_state.set(false, false);
                self.space.unmap_output(&output);
                self.set_output_global_enabled(&output, false);
                self.fail_pending_captures_for_output(&output);
            } else {
                output_state.set(true, head.adaptive_sync_enabled);
            }
            changed_outputs.push(output);
        }
        self.output_management_state
            .update_heads::<Self>(changed_outputs.iter());
        self.set_mirror_roles(realized_mirrors(&self.runtime.mirror_of, snapshot));
        self.request_render();
    }

    /// Give every enabled output its role: realized mirrors leave the space
    /// and stop advertising a `wl_output`, every other enabled output owns
    /// its region of the space.
    ///
    /// A mirror presents its source's region, so it must not own input,
    /// hit-testing, layer-shell placement or damage of its own. Keeping it out
    /// of the space makes every space consumer correct without special cases;
    /// the renderer projects the source's scene onto it.
    pub(crate) fn set_mirror_roles(&mut self, realized: std::collections::HashMap<String, String>) {
        let mut layers_closed = false;
        for output in self.output_management_state.outputs().to_vec() {
            let enabled = output
                .user_data()
                .get::<OutputManagementOutputState>()
                .is_none_or(OutputManagementOutputState::enabled);
            if !enabled {
                continue;
            }
            let name = output.name();
            if realized.contains_key(&name) {
                if !self.runtime.realized_mirrors.contains_key(&name) {
                    // Everything clients attached to its `wl_output` goes.
                    layers_closed |= close_layer_surfaces(&output);
                    self.fail_pending_captures_for_output(&output);
                    let cancelled = self.output_power_state.fail_output(&name);
                    self.runtime.output_power.cancel(&cancelled);
                }
                self.space.unmap_output(&output);
                self.set_output_global_enabled(&output, false);
            } else {
                self.space.map_output(&output, output.current_location());
                self.set_output_global_enabled(&output, true);
            }
        }
        self.runtime.realized_mirrors = realized;
        if layers_closed {
            self.push_command(
                crate::backend::wayland::commands::WmCommand::SyncLayerExclusiveZones,
            );
        }
        self.request_render();
    }

    /// Break realized pairs whose mirror or source head disappeared, turning
    /// the remaining head into an ordinary output immediately.
    pub(crate) fn drop_unavailable_mirrors(&mut self) {
        let present: std::collections::HashSet<String> = self
            .output_management_state
            .outputs()
            .iter()
            .map(Output::name)
            .collect();
        let realized: std::collections::HashMap<_, _> = self
            .runtime
            .realized_mirrors
            .iter()
            .filter(|(mirror, source)| present.contains(*mirror) && present.contains(*source))
            .map(|(mirror, source)| (mirror.clone(), source.clone()))
            .collect();
        if realized.len() != self.runtime.realized_mirrors.len() {
            self.set_mirror_roles(realized);
        }
    }

    /// The output whose region `output` presents: its source for a realized
    /// mirror, itself otherwise.
    pub(crate) fn presented_output(&self, output: &Output) -> Output {
        self.runtime
            .realized_mirrors
            .get(&output.name())
            .and_then(|source| {
                self.space
                    .outputs()
                    .find(|candidate| candidate.name() == *source)
            })
            .unwrap_or(output)
            .clone()
    }

    /// Current ownership of an output's position for automatic placement.
    pub(crate) fn output_position_source(&self, name: &str) -> OutputPositionSource {
        if self.runtime.configured_output_positions.contains(name) {
            return OutputPositionSource::Configured;
        }
        match self.runtime.output_position_sources.get(name) {
            Some(OutputPositionSource::ClientManaged) => OutputPositionSource::ClientManaged,
            _ => OutputPositionSource::Automatic,
        }
    }

    fn finish_output_transaction(
        &mut self,
        id: RequestId,
        kind: OutputTransactionKind,
        result: Result<OutputSnapshot, OutputTransactionError>,
    ) -> bool {
        let succeeded = result.is_ok();
        let changed = kind == OutputTransactionKind::Apply && succeeded;
        if let Err(error) = &result {
            log::warn!("output transaction {id:?} failed: {error}");
        }
        if let Ok(snapshot) = &result
            && kind == OutputTransactionKind::Apply
        {
            self.apply_output_snapshot(snapshot);
        }
        self.output_management_state
            .finish_transaction(id, succeeded);
        changed
    }

    pub fn project_completed_output_transactions(&mut self) -> bool {
        let completed = self.runtime.output_transactions.take_completed();
        let mut changed = false;
        for (id, (kind, result)) in completed {
            changed |= self.finish_output_transaction(id, kind, result);
        }
        changed
    }

    pub fn project_completed_output_power_requests(&mut self) {
        for (id, (output, result)) in self.runtime.output_power.take_completed() {
            let cancelled = self.output_power_state.complete(id, &output, result);
            self.runtime.output_power.cancel(&cancelled);
        }
    }

    fn current_output_transaction(&self) -> OutputTransaction {
        if let Some(pending) = self.runtime.output_transactions.latest_pending_apply() {
            return pending.clone();
        }
        let heads = self
            .output_management_state
            .outputs()
            .iter()
            .map(|output| {
                let output_state = output.user_data().get::<OutputManagementOutputState>();
                let vrr_mode = self
                    .output_vrr_metadata(&output.name())
                    .map(|metadata| metadata.vrr_mode)
                    .unwrap_or(VrrMode::Off);
                OutputHeadConfiguration {
                    id: OutputId(output.name()),
                    enabled: output_state.is_none_or(OutputManagementOutputState::enabled),
                    mode: output.current_mode().map(|mode| TransactionOutputMode {
                        width: mode.size.w,
                        height: mode.size.h,
                        refresh_millihertz: mode.refresh,
                    }),
                    position: Point::new(output.current_location().x, output.current_location().y),
                    transform: from_smithay_transform(output.current_transform()),
                    scale: output.current_scale().fractional_scale(),
                    adaptive_sync: Some(match vrr_mode {
                        VrrMode::Off => AdaptiveSyncPolicy::Disabled,
                        VrrMode::On => AdaptiveSyncPolicy::Enabled,
                        VrrMode::Auto => AdaptiveSyncPolicy::Automatic,
                    }),
                }
            })
            .collect();
        OutputTransaction { heads }
    }

    fn queue_output_transaction(&mut self, transaction: OutputTransaction) {
        self.runtime
            .output_transactions
            .submit_coalescing_apply(transaction);
        self.request_render();
    }

    /// Queue the desired output state as a policy transaction, even when no
    /// output property changed: the DRM runtime uses this boundary to update
    /// position ownership after configuration entries are removed.
    ///
    /// Heads that stopped being mirrors since the last projection re-enter
    /// the desktop. A pinned mirror carried its source's position and scale,
    /// so unless configured otherwise it returns to scale 1.0 and starts right
    /// of the layout. Automatic heads are then compacted, which also closes
    /// the hole a head leaves when it becomes a mirror.
    pub(crate) fn queue_output_policy_projection(
        &mut self,
        configs: &std::collections::HashMap<String, crate::config::config_toml::MonitorConfig>,
    ) {
        let mut transaction = self.current_output_transaction();
        let enabled: std::collections::HashSet<String> = transaction
            .heads
            .iter()
            .filter(|head| head.enabled)
            .map(|head| head.id.0.clone())
            .collect();
        let mirrors: std::collections::HashSet<String> = self
            .runtime
            .mirror_of
            .active_pairs(|name| enabled.contains(name))
            .map(|(mirror, _)| mirror.to_string())
            .collect();
        let mut released: Vec<_> = self
            .runtime
            .projected_mirrors
            .difference(&mirrors)
            .cloned()
            .collect();
        released.sort();
        for name in released {
            let occupied: Vec<Rect> = transaction
                .heads
                .iter()
                .filter(|head| head.enabled && head.id.0 != name && !mirrors.contains(&head.id.0))
                .map(head_rect)
                .collect();
            let Some(head) = transaction.heads.iter_mut().find(|head| head.id.0 == name) else {
                continue;
            };
            let config = configs.get(&name).or_else(|| configs.get("*"));
            if config.is_none_or(|config| config.scale.is_none()) {
                head.scale = 1.0;
            }
            if self.output_position_source(&name) == OutputPositionSource::Automatic {
                head.position = position_after(occupied);
            }
        }

        let mut placements: Vec<_> = transaction
            .heads
            .iter()
            .filter(|head| head.enabled && !mirrors.contains(&head.id.0))
            .map(|head| OutputPlacement {
                id: head.id.0.clone(),
                rect: head_rect(head),
                source: self.output_position_source(&head.id.0),
            })
            .collect();
        for (name, position) in plan_automatic_output_positions(&mut placements) {
            if let Some(head) = transaction.heads.iter_mut().find(|head| head.id.0 == name) {
                head.position = position;
            }
        }

        self.runtime.projected_mirrors = mirrors;
        self.queue_output_transaction(transaction);
    }

    /// Create and register a default output.
    pub fn create_output(
        &mut self,
        name: &str,
        size: Size,
        refresh_millihertz: Option<u32>,
    ) -> Output {
        let safe_size = Size::new(size.w.max(Self::MIN_WL_DIM), size.h.max(Self::MIN_WL_DIM));
        let mode = OutputMode {
            size: (safe_size.w, safe_size.h).into(),
            refresh: refresh_millihertz
                .and_then(|refresh| i32::try_from(refresh).ok())
                .filter(|refresh| *refresh > 0)
                .unwrap_or(60_000),
        };
        let output = self.create_output_global(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
                serial_number: "Unknown".into(),
            },
            mode,
            Point::default(),
        );
        self.space.map_output(&output, (0, 0));
        self.set_output_vrr_support(name, BackendVrrSupport::Unsupported);
        self.set_output_vrr_mode(name, VrrMode::Off);
        self.set_output_vrr_enabled(name, false);

        // Register the new output with wlr-output-management.
        self.output_management_state
            .add_heads::<Self>(std::iter::once(&output));

        output
    }

    pub(crate) fn create_output_global(
        &self,
        name: String,
        physical_properties: PhysicalProperties,
        mode: OutputMode,
        location: Point,
    ) -> Output {
        let output = Output::new(name, physical_properties);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((location.x, location.y).into()),
        );
        output.set_preferred(mode);
        let global = output.create_global::<WaylandState>(&self.display_handle);
        output
            .user_data()
            .insert_if_missing_threadsafe(|| OutputGlobal(Mutex::new(Some(global))));
        output
    }

    /// List all connected displays, including mirror heads.
    pub fn list_displays(&self) -> Vec<String> {
        self.output_management_state
            .outputs()
            .iter()
            .map(Output::name)
            .collect()
    }

    /// List available display modes for a display.
    pub fn list_display_modes(&self, display: &str) -> Vec<String> {
        let mut result = Vec::new();
        if let Some(output) = self
            .output_management_state
            .outputs()
            .iter()
            .find(|o| o.name() == display)
        {
            for mode in output.modes() {
                result.push(format!(
                    "{}x{}@{}",
                    mode.size.w,
                    mode.size.h,
                    mode.refresh as f64 / 1000.0
                ));
            }
        }
        result
    }

    /// Set the display mode for a display.
    pub fn set_display_mode(&mut self, display: &str, size: Size) {
        let mut transaction = self.current_output_transaction();
        let Some(output) = self
            .output_management_state
            .outputs()
            .iter()
            .find(|output| output.name() == display)
        else {
            return;
        };
        let Some(mode) = output
            .modes()
            .into_iter()
            .find(|mode| mode.size.w == size.w && mode.size.h == size.h)
        else {
            return;
        };
        if let Some(head) = transaction
            .heads
            .iter_mut()
            .find(|head| head.id.0 == display)
        {
            head.mode = Some(TransactionOutputMode {
                width: mode.size.w,
                height: mode.size.h,
                refresh_millihertz: mode.refresh,
            });
            self.queue_output_transaction(transaction);
        }
    }

    /// Configure an output based on MonitorConfig.
    pub fn set_output_config(
        &mut self,
        display: &str,
        config: &crate::config::config_toml::MonitorConfig,
    ) {
        let mut transaction = self.current_output_transaction();
        let known_outputs: Vec<_> = transaction
            .heads
            .iter()
            .map(|head| {
                let size = logical_output_size(head);
                (
                    head.id.0.clone(),
                    Rect::new(head.position.x, head.position.y, size.w, size.h),
                )
            })
            .collect();

        let outputs = self.output_management_state.outputs().to_vec();
        let mut changed = false;
        for head in &mut transaction.heads {
            if display != "*" && head.id.0 != display {
                continue;
            }
            let Some(output) = outputs.iter().find(|output| output.name() == head.id.0) else {
                continue;
            };

            if let Some(request) = config
                .resolution
                .as_deref()
                .and_then(|resolution| MonitorModeRequest::parse(resolution, config.refresh_rate))
                && let Some(mode) = output
                    .modes()
                    .into_iter()
                    .find(|m| request.matches(m.size.w, m.size.h, u32::try_from(m.refresh).ok()))
            {
                head.mode = Some(TransactionOutputMode {
                    width: mode.size.w,
                    height: mode.size.h,
                    refresh_millihertz: mode.refresh,
                });
            }

            if let Some(scale) = config.scale {
                head.scale = f64::from(scale);
            }

            if let Some(vrr) = config.vrr {
                head.adaptive_sync = Some(match vrr {
                    VrrMode::Off => AdaptiveSyncPolicy::Disabled,
                    VrrMode::On => AdaptiveSyncPolicy::Enabled,
                    VrrMode::Auto => AdaptiveSyncPolicy::Automatic,
                });
            }

            if let Some(enable) = config.enable {
                head.enabled = enable;
            }

            if let Some(transform) = config
                .transform
                .as_ref()
                .and_then(|t| OutputTransform::parse(t))
            {
                head.transform = transform;
            }

            if let Some(ref pos) = config.position
                && let Some(position) = MonitorPosition::parse(pos).and_then(|p| {
                    let size = logical_output_size(head);
                    p.resolve(
                        size,
                        known_outputs
                            .iter()
                            .map(|(name, rect)| (name.as_str(), *rect)),
                    )
                })
            {
                head.position = position;
            }
            changed = true;
        }
        if changed {
            self.queue_output_transaction(transaction);
        }
    }

    pub fn set_output_vrr_support(&mut self, output_name: &str, support: BackendVrrSupport) {
        let entry = self
            .runtime
            .output_metadata
            .entry(output_name.to_string())
            .or_insert(WaylandOutputMetadata {
                vrr_support: support,
                vrr_mode: VrrMode::default(),
                vrr_enabled: false,
            });
        entry.vrr_support = support;
    }

    pub fn set_output_vrr_mode(&mut self, output_name: &str, mode: VrrMode) {
        let entry = self
            .runtime
            .output_metadata
            .entry(output_name.to_string())
            .or_insert(WaylandOutputMetadata {
                vrr_support: BackendVrrSupport::Unsupported,
                vrr_mode: mode,
                vrr_enabled: false,
            });
        entry.vrr_mode = mode;
    }

    pub(crate) fn project_output_vrr_state(
        &mut self,
        output_name: &str,
        mode: VrrMode,
        enabled: bool,
    ) {
        self.set_output_vrr_mode(output_name, mode);
        self.runtime
            .output_metadata
            .get_mut(output_name)
            .expect("setting the VRR mode initializes output metadata")
            .vrr_enabled = enabled;
    }

    pub fn set_output_vrr_enabled(&mut self, output_name: &str, enabled: bool) {
        let entry = self
            .runtime
            .output_metadata
            .entry(output_name.to_string())
            .or_insert(WaylandOutputMetadata {
                vrr_support: BackendVrrSupport::Unsupported,
                vrr_mode: VrrMode::default(),
                vrr_enabled: enabled,
            });
        let changed = entry.vrr_enabled != enabled;
        entry.vrr_enabled = enabled;
        if changed
            && let Some(output) = self
                .output_management_state
                .outputs()
                .iter()
                .find(|output| output.name() == output_name)
                .cloned()
        {
            if let Some(output_state) = output.user_data().get::<OutputManagementOutputState>() {
                output_state.set(output_state.enabled(), enabled);
            }
            self.output_management_state
                .update_heads::<Self>(std::iter::once(&output));
        }
    }

    pub fn output_vrr_metadata(&self, output_name: &str) -> Option<&WaylandOutputMetadata> {
        self.runtime.output_metadata.get(output_name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaylandOutputMetadata {
    pub vrr_support: BackendVrrSupport,
    pub vrr_mode: VrrMode,
    pub vrr_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(transform: OutputTransform, scale: f64) -> OutputHeadConfiguration {
        OutputHeadConfiguration {
            id: "test".into(),
            enabled: true,
            mode: Some(TransactionOutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 60_000,
            }),
            position: Point::default(),
            transform,
            scale,
            adaptive_sync: None,
        }
    }

    #[test]
    fn transaction_geometry_uses_logical_scaled_dimensions() {
        assert_eq!(
            logical_output_size(&configuration(OutputTransform::Normal, 1.5)),
            Size::new(1280, 720)
        );
    }

    fn mirror_map(pairs: &[(&str, &str)]) -> crate::output_mirror::MirrorMap {
        crate::output_mirror::MirrorMap::from_pairs(
            pairs
                .iter()
                .map(|(mirror, source)| (mirror.to_string(), source.to_string())),
        )
    }

    fn snapshot(heads: &[(&str, bool, f64)]) -> OutputSnapshot {
        OutputSnapshot {
            heads: heads
                .iter()
                .map(
                    |(name, enabled, scale)| crate::backend::output::OutputHeadSnapshot {
                        configuration: OutputHeadConfiguration {
                            id: (*name).into(),
                            enabled: *enabled,
                            ..configuration(OutputTransform::Normal, *scale)
                        },
                        modes: Vec::new(),
                        adaptive_sync_policy: AdaptiveSyncPolicy::Disabled,
                        adaptive_sync_enabled: false,
                    },
                )
                .collect(),
        }
    }

    /// Three 1920x1080 outputs side by side, all automatically placed.
    fn three_outputs() -> WaylandState {
        let (_event_loop, mut state) = super::super::new_event_loop_and_state();
        for (index, name) in ["eDP-1", "DP-1", "HDMI-1"].into_iter().enumerate() {
            let output = state.create_output(name, Size::new(1920, 1080), None);
            let location = (index as i32 * 1920, 0).into();
            output.change_current_state(None, None, None, Some(location));
            state.space.map_output(&output, location);
        }
        state
    }

    fn pending_head(state: &WaylandState, name: &str) -> OutputHeadConfiguration {
        state
            .runtime
            .output_transactions
            .latest_pending_apply()
            .expect("a projection was queued")
            .heads
            .iter()
            .find(|head| head.id.0 == name)
            .expect("head in transaction")
            .clone()
    }

    #[test]
    fn snapshots_realize_only_enabled_and_pinned_pairs() {
        let mirrors = mirror_map(&[("DP-1", "eDP-1")]);

        let pinned = snapshot(&[("eDP-1", true, 1.5), ("DP-1", true, 1.5)]);
        assert_eq!(
            realized_mirrors(&mirrors, &pinned),
            [("DP-1".to_string(), "eDP-1".to_string())].into()
        );

        // Submitted before the declaration: not pinned yet, so not realized.
        let unpinned = snapshot(&[("eDP-1", true, 1.5), ("DP-1", true, 1.0)]);
        assert!(realized_mirrors(&mirrors, &unpinned).is_empty());

        // A disabled source leaves the mirror an ordinary output.
        let source_off = snapshot(&[("eDP-1", false, 1.0), ("DP-1", true, 1.0)]);
        assert!(realized_mirrors(&mirrors, &source_off).is_empty());
    }

    #[test]
    fn mirror_roles_move_heads_out_of_and_back_into_the_space() {
        let mut state = three_outputs();
        let in_space = |state: &WaylandState| -> Vec<String> {
            let mut names: Vec<_> = state.space.outputs().map(Output::name).collect();
            names.sort();
            names
        };

        state.set_mirror_roles([("DP-1".to_string(), "eDP-1".to_string())].into());
        assert_eq!(in_space(&state), vec!["HDMI-1", "eDP-1"]);
        let mirror = state
            .output_management_state
            .outputs()
            .iter()
            .find(|output| output.name() == "DP-1")
            .unwrap()
            .clone();
        assert_eq!(state.presented_output(&mirror).name(), "eDP-1");

        state.set_mirror_roles(Default::default());
        assert_eq!(in_space(&state), vec!["DP-1", "HDMI-1", "eDP-1"]);
        assert_eq!(state.presented_output(&mirror).name(), "DP-1");
    }

    #[test]
    fn projection_closes_the_hole_a_new_mirror_leaves() {
        let mut state = three_outputs();
        state.runtime.mirror_of = mirror_map(&[("DP-1", "eDP-1")]);

        state.queue_output_policy_projection(&Default::default());

        assert!(state.runtime.projected_mirrors.contains("DP-1"));
        assert_eq!(pending_head(&state, "eDP-1").position, Point::new(0, 0));
        assert_eq!(pending_head(&state, "HDMI-1").position, Point::new(1920, 0));
    }

    #[test]
    fn a_released_mirror_reenters_right_of_the_layout_at_its_own_scale() {
        let mut state = three_outputs();
        // DP-1 was pinned onto eDP-1 (position and scale) and its
        // declaration has now been removed.
        state.runtime.projected_mirrors.insert("DP-1".to_string());
        let mirror = state
            .output_management_state
            .outputs()
            .iter()
            .find(|output| output.name() == "DP-1")
            .unwrap()
            .clone();
        mirror.change_current_state(
            None,
            None,
            Some(Scale::Fractional(2.0)),
            Some((0, 0).into()),
        );

        state.queue_output_policy_projection(&Default::default());

        let released = pending_head(&state, "DP-1");
        assert_eq!(released.scale, 1.0);
        // Compaction then packs it after the other automatic outputs.
        assert_eq!(pending_head(&state, "eDP-1").position, Point::new(0, 0));
        assert_eq!(released.position, Point::new(3840, 0));
        assert!(state.runtime.projected_mirrors.is_empty());
    }

    #[test]
    fn a_released_mirror_keeps_its_configured_scale() {
        let mut state = three_outputs();
        state.runtime.projected_mirrors.insert("DP-1".to_string());
        let configs: std::collections::HashMap<_, _> = [(
            "DP-1".to_string(),
            crate::config::config_toml::MonitorConfig {
                scale: Some(1.25),
                ..Default::default()
            },
        )]
        .into();
        state.set_output_config("DP-1", &configs["DP-1"]);

        state.queue_output_policy_projection(&configs);

        assert_eq!(pending_head(&state, "DP-1").scale, 1.25);
    }

    #[test]
    fn transaction_geometry_swaps_rotated_dimensions() {
        assert_eq!(
            logical_output_size(&configuration(OutputTransform::Rotate90, 2.0)),
            Size::new(540, 960)
        );
        assert_eq!(
            logical_output_size(&configuration(OutputTransform::Flipped270, 1.0)),
            Size::new(1080, 1920)
        );
    }
}
