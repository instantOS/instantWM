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
    AdaptiveSyncPolicy, CompletedOutputTransaction, OutputHeadConfiguration, OutputId,
    OutputMode as TransactionOutputMode, OutputSnapshot, OutputTransaction, OutputTransactionKind,
    OutputTransform,
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
            if !config.enabled {
                self.runtime.output_power_modes.remove(&output.name());
                let cancelled = self.output_power_state.fail_output(&output.name());
                self.runtime.output_power.cancel(&cancelled);
                output_state.set(false, false);
                self.space.unmap_output(&output);
                self.set_output_global_enabled(&output, false);
                self.fail_pending_captures_for_output(&output);
            } else {
                let location = (config.position.x, config.position.y).into();
                output.change_current_state(
                    config.mode.map(smithay_mode),
                    Some(to_smithay_transform(config.transform)),
                    Some(Scale::Fractional(config.scale)),
                    Some(location),
                );
                self.space.map_output(&output, location);
                self.set_output_global_enabled(&output, true);
                output_state.set(true, head.adaptive_sync_enabled);
            }
            changed_outputs.push(output);
        }
        self.output_management_state
            .update_heads::<Self>(changed_outputs.iter());
        self.request_render();
    }

    fn finish_output_transaction(&mut self, completed: CompletedOutputTransaction) -> bool {
        let succeeded = completed.result.is_ok();
        let changed = completed.kind == OutputTransactionKind::Apply && succeeded;
        if let Err(error) = &completed.result {
            log::warn!("output transaction {:?} failed: {error}", completed.id);
        }
        if let Ok(snapshot) = &completed.result
            && completed.kind == OutputTransactionKind::Apply
        {
            self.apply_output_snapshot(snapshot);
        }
        self.output_management_state
            .finish_transaction(completed.id, succeeded);
        changed
    }

    pub fn project_completed_output_transactions(&mut self) -> bool {
        let completed = self.runtime.output_transactions.take_completed();
        let mut changed = false;
        for transaction in completed {
            changed |= self.finish_output_transaction(transaction);
        }
        changed
    }

    pub fn project_completed_output_power_requests(&mut self) {
        for completed in self.runtime.output_power.take_completed() {
            let cancelled = self.output_power_state.complete(completed);
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

    /// Queue the current desired state as a policy transaction even when no
    /// output property changed. The DRM runtime uses this boundary to update
    /// position ownership after configuration entries are removed.
    pub(crate) fn queue_output_policy_projection(&mut self) {
        let transaction = self.current_output_transaction();
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

    /// List all connected displays.
    pub fn list_displays(&self) -> Vec<String> {
        self.space.outputs().map(|o| o.name()).collect()
    }

    /// List available display modes for a display.
    pub fn list_display_modes(&self, display: &str) -> Vec<String> {
        let mut result = Vec::new();
        if let Some(output) = self.space.outputs().find(|o| o.name() == display) {
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

            if let Some(ref res) = config.resolution
                && let Some((w_str, h_str)) = res.split_once('x')
                && let (Ok(w), Ok(h)) = (w_str.parse::<i32>(), h_str.parse::<i32>())
                && let Some(mode) = output.modes().into_iter().find(|m| {
                    m.size.w == w
                        && m.size.h == h
                        && config
                            .refresh_rate
                            .map(|r| (m.refresh as f32 / 1000.0 - r).abs() < 0.1)
                            .unwrap_or(true)
                })
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
