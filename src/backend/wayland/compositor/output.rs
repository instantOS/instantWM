//! Output/display management for WaylandState.
//!
//! This module contains output-related methods on WaylandState,
//! including creating outputs, listing displays, and configuring display modes.

use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::utils::Transform;

use crate::backend::BackendVrrSupport;
use crate::config::config_toml::VrrMode;
use crate::types::{MonitorPosition, Rect};

use super::state::WaylandState;

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
    /// Create and register a default output.
    pub fn create_output(&mut self, name: &str, width: i32, height: i32) -> Output {
        let safe_width = width.max(Self::MIN_WL_DIM);
        let safe_height = height.max(Self::MIN_WL_DIM);
        let mode = OutputMode {
            size: (safe_width, safe_height).into(),
            refresh: 60_000,
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
            (0, 0),
        );
        self.space.map_output(&output, (0, 0));
        self.set_output_vrr_support(name, BackendVrrSupport::Unsupported);
        self.set_output_vrr_mode(name, VrrMode::Off);
        self.set_output_vrr_enabled(name, false);

        output
    }

    pub(crate) fn create_output_global(
        &self,
        name: String,
        physical_properties: PhysicalProperties,
        mode: OutputMode,
        location: (i32, i32),
    ) -> Output {
        let output = Output::new(name, physical_properties);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some(location.into()),
        );
        output.set_preferred(mode);
        let _global = output.create_global::<WaylandState>(&self.display_handle);
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
    pub fn set_display_mode(&mut self, display: &str, width: i32, height: i32) {
        if let Some(output) = self.space.outputs().find(|o| o.name() == display).cloned()
            && let Some(mode) = output
                .modes()
                .into_iter()
                .find(|m| m.size.w == width && m.size.h == height)
        {
            output.change_current_state(Some(mode), None, None, None);
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

            let new_transform = config.transform.as_ref().and_then(|t| parse_transform(t));

            if let Some(ref pos) = config.position
                && let Some((x, y)) = MonitorPosition::parse(pos).and_then(|p| {
                    let size = current_mode
                        .as_ref()
                        .map(|mode| (mode.size.w, mode.size.h))
                        .unwrap_or((current_geometry.size.w, current_geometry.size.h));
                    p.resolve(
                        size,
                        known_outputs
                            .iter()
                            .map(|(name, rect)| (name.as_str(), *rect)),
                    )
                })
            {
                current_location = (x, y).into();
            }

            output.change_current_state(
                current_mode,
                new_transform.or(Some(current_transform)),
                Some(current_scale),
                Some(current_location),
            );
            self.space.map_output(&output, current_location);
        }
    }
}
