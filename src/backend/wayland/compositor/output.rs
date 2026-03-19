//! Output/display management for WaylandState.
//!
//! This module contains output-related methods on WaylandState,
//! including creating outputs, listing displays, and configuring display modes.

use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::utils::Transform;

use super::state::WaylandState;

impl WaylandState {
    /// Create and register a default output.
    pub fn create_output(&mut self, name: &str, width: i32, height: i32) -> Output {
        let safe_width = width.max(Self::MIN_WL_DIM);
        let safe_height = height.max(Self::MIN_WL_DIM);
        let output = Output::new(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
            },
        );

        let mode = OutputMode {
            size: (safe_width, safe_height).into(),
            refresh: 60_000,
        };

        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Integer(1)),
            Some((0, 0).into()),
        );
        output.set_preferred(mode);

        let _global = output.create_global::<WaylandState>(&self.display_handle);
        self.space.map_output(&output, (0, 0));

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
        for output in outputs {
            if display != "*" && output.name() != display {
                continue;
            }

            let mut current_mode = output.current_mode();
            let mut current_scale = output.current_scale();
            let current_transform = output.current_transform();
            let mut current_location = self
                .space
                .output_geometry(&output)
                .map(|g| g.loc)
                .unwrap_or_default();

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

            if let Some(ref pos) = config.position
                && let Some((x_str, y_str)) = pos.split_once(',')
                && let (Ok(x), Ok(y)) = (x_str.parse::<i32>(), y_str.parse::<i32>())
            {
                current_location = (x, y).into();
            }

            output.change_current_state(
                current_mode,
                Some(current_transform),
                Some(current_scale),
                Some(current_location),
            );
            self.space.map_output(&output, current_location);
        }
    }
}
