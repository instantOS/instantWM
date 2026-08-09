//! Output/display management for WaylandState.
//!
//! This module contains output-related methods on WaylandState,
//! including creating outputs, listing displays, and configuring display modes.

use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::utils::Transform;
use std::sync::Mutex;

use crate::backend::BackendVrrSupport;
use crate::config::config_toml::VrrMode;
use crate::types::{MonitorPosition, Point, Rect, Size};

use super::protocols::output_management::{
    ModeConfiguration, OutputConfiguration, OutputConfigurationTransaction,
    OutputManagementOutputState, OutputModeData,
};
use super::state::WaylandState;

struct OutputGlobal(Mutex<Option<GlobalId>>);

fn parse_transform(transform_str: &str) -> Option<Transform> {
    match transform_str.to_lowercase().as_str() {
        "normal" => Some(Transform::Normal),
        "90" => Some(Transform::_90),
        "180" => Some(Transform::_180),
        "270" => Some(Transform::_270),
        "flipped" => Some(Transform::Flipped),
        "flipped-90" | "flipped90" => Some(Transform::Flipped90),
        "flipped-180" | "flipped180" => Some(Transform::Flipped180),
        "flipped-270" | "flipped270" => Some(Transform::Flipped270),
        _ => None,
    }
}

impl WaylandState {
    fn set_output_global_enabled(&self, output: &Output, enabled: bool) {
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

    pub fn finish_output_configuration(
        &mut self,
        transaction: OutputConfigurationTransaction,
        succeeded: bool,
    ) {
        if !succeeded {
            transaction.configuration.failed();
            return;
        }

        let changed_outputs: Vec<_> = transaction
            .heads
            .iter()
            .map(|(output, _)| output.clone())
            .collect();

        for (output, config) in &transaction.heads {
            let output_state = output
                .user_data()
                .get::<OutputManagementOutputState>()
                .expect("output-management state is initialized when a head is added");
            match config {
                OutputConfiguration::Disabled => {
                    output_state.set(false, false);
                    self.runtime.output_enabled.insert(output.name(), false);
                    self.space.unmap_output(output);
                    self.set_output_global_enabled(output, false);
                    self.fail_pending_captures_for_output(output);
                }
                OutputConfiguration::Enabled {
                    mode,
                    position,
                    transform,
                    scale,
                    adaptive_sync,
                } => {
                    let mode = match mode {
                        Some(ModeConfiguration::Mode(resource)) => {
                            resource.data::<OutputModeData>().map(|data| data.mode)
                        }
                        Some(ModeConfiguration::Custom { size, refresh }) => {
                            output.modes().into_iter().find(|candidate| {
                                candidate.size == *size
                                    && refresh.is_none_or(|value| candidate.refresh == value)
                            })
                        }
                        None => output.current_mode(),
                    };
                    let location = position.unwrap_or_else(|| output.current_location());
                    let adaptive_sync =
                        adaptive_sync.unwrap_or_else(|| output_state.adaptive_sync());
                    output.change_current_state(
                        mode,
                        transform.or(Some(output.current_transform())),
                        scale
                            .map(Scale::Fractional)
                            .or(Some(output.current_scale())),
                        Some(location),
                    );
                    self.space.map_output(output, location);
                    self.set_output_global_enabled(output, true);
                    self.runtime.output_enabled.insert(output.name(), true);
                    output_state.set(true, adaptive_sync);
                    self.set_output_vrr_mode(
                        &output.name(),
                        if adaptive_sync {
                            VrrMode::On
                        } else {
                            VrrMode::Off
                        },
                    );
                }
            }
        }

        self.output_management_state
            .update_heads::<Self>(changed_outputs.iter());
        transaction.configuration.succeeded();
        self.request_render();
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
        if let Some(output) = self.space.outputs().find(|o| o.name() == display).cloned()
            && let Some(mode) = output
                .modes()
                .into_iter()
                .find(|mode| mode.size.w == size.w && mode.size.h == size.h)
        {
            output.change_current_state(Some(mode), None, None, None);
            self.output_management_state
                .update_heads::<Self>(std::iter::once(&output));
        }
    }

    /// Configure an output based on MonitorConfig.
    pub fn set_output_config(
        &mut self,
        display: &str,
        config: &crate::config::config_toml::MonitorConfig,
    ) {
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        let known_outputs: Vec<_> = outputs
            .iter()
            .map(|output| {
                let geom = self.space.output_geometry(output).unwrap_or_default();
                (
                    output.name(),
                    Rect::new(geom.loc.x, geom.loc.y, geom.size.w, geom.size.h),
                )
            })
            .collect();

        let mut changed_outputs = Vec::new();
        for output in outputs {
            if display != "*" && output.name() != display {
                continue;
            }

            let mut current_mode = output.current_mode();
            let mut current_scale = output.current_scale();
            let current_transform = output.current_transform();
            let current_geometry = self.space.output_geometry(&output).unwrap_or_default();
            let mut current_location = current_geometry.loc;

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
                current_mode = Some(mode);
            }

            if let Some(scale) = config.scale {
                current_scale = Scale::Fractional(scale as f64);
            }

            if let Some(vrr) = config.vrr {
                self.set_output_vrr_mode(&output.name(), vrr);
            }

            if let Some(enable) = config.enable {
                self.runtime.output_enabled.insert(output.name(), enable);
                if let Some(output_state) = output.user_data().get::<OutputManagementOutputState>()
                {
                    output_state.set(enable, output_state.adaptive_sync());
                }
            }

            let new_transform = config.transform.as_ref().and_then(|t| parse_transform(t));

            if let Some(ref pos) = config.position
                && let Some(position) = MonitorPosition::parse(pos).and_then(|p| {
                    let size = current_mode
                        .as_ref()
                        .map(|mode| Size::new(mode.size.w, mode.size.h))
                        .unwrap_or(Size::new(current_geometry.size.w, current_geometry.size.h));
                    p.resolve(
                        size,
                        known_outputs
                            .iter()
                            .map(|(name, rect)| (name.as_str(), *rect)),
                    )
                })
            {
                current_location = (position.x, position.y).into();
            }

            output.change_current_state(
                current_mode,
                new_transform.or(Some(current_transform)),
                Some(current_scale),
                Some(current_location),
            );
            if self
                .runtime
                .output_enabled
                .get(&output.name())
                .copied()
                .unwrap_or(true)
            {
                self.space.map_output(&output, current_location);
                self.set_output_global_enabled(&output, true);
            } else {
                self.space.unmap_output(&output);
                self.set_output_global_enabled(&output, false);
            }
            changed_outputs.push(output);
        }
        if !changed_outputs.is_empty() {
            self.output_management_state
                .update_heads::<Self>(changed_outputs.iter());
        }
    }
}
