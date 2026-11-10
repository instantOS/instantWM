//! Backend-neutral display modes and configuration requests.

use super::Size;

pub use instantos_ipc::OutputMode;
use instantos_ipc::refresh_hz_to_millihertz;

fn parse_mode_dimensions(resolution: &str) -> Option<Size> {
    instantos_ipc::parse_mode_dimensions(resolution).map(|(w, h)| Size::new(w, h))
}

/// A mode requested by monitor configuration, independent of the output API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorModeRequest {
    size: Size,
    refresh_hz: Option<f32>,
}

impl MonitorModeRequest {
    /// Any refresh rate at exactly `width`x`height`.
    pub fn size(size: Size) -> Self {
        Self {
            size,
            refresh_hz: None,
        }
    }

    pub fn parse(resolution: &str, refresh_hz: Option<f32>) -> Option<Self> {
        let size = parse_mode_dimensions(resolution)?;
        if let Some(rate) = refresh_hz {
            refresh_hz_to_millihertz(f64::from(rate))?;
        }
        Some(Self { size, refresh_hz })
    }

    /// Unknown refresh is acceptable only when no rate was requested.
    pub fn matches(self, width: i32, height: i32, refresh_millihertz: Option<u32>) -> bool {
        if self.size != Size::new(width, height) {
            return false;
        }
        match self.refresh_hz {
            None => true,
            Some(requested) => refresh_millihertz.is_some_and(|actual| {
                (f64::from(actual) / 1000.0 - f64::from(requested)).abs() < 0.1
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MonitorModeRequest, OutputMode};

    #[test]
    fn configuration_and_ipc_reject_invalid_dimensions_and_refresh() {
        for resolution in [
            "0x1080",
            "1920x0",
            "-1x1080",
            "2147483648x1080",
            "1920x1080x60",
        ] {
            assert!(MonitorModeRequest::parse(resolution, None).is_none());
            assert!(format!("{resolution}@60").parse::<OutputMode>().is_err());
        }
        for rate in [0.0, -1.0, f32::NAN, f32::INFINITY, 3_000_000.0] {
            assert!(MonitorModeRequest::parse("1920x1080", Some(rate)).is_none());
            assert!(format!("1920x1080@{rate}").parse::<OutputMode>().is_err());
        }
    }

    #[test]
    fn ipc_rounds_fractional_refresh_to_nearest_millihertz() {
        let mode: OutputMode = "1920x1080@59.9406".parse().unwrap();
        assert_eq!(
            (mode.width, mode.height, mode.refresh_millihertz),
            (1920, 1080, 59941)
        );
        assert!(
            MonitorModeRequest::parse("1920x1080", Some(59.94))
                .unwrap()
                .matches(1920, 1080, Some(mode.refresh_millihertz as u32))
        );
    }
}
