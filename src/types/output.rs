//! Backend-neutral display modes and configuration requests.

use super::Size;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct OutputMode {
    pub width: i32,
    pub height: i32,
    pub refresh_millihertz: i32,
}

/// Dimensions accepted by both configuration and IPC, bounded by compositor geometry.
pub(crate) fn parse_mode_dimensions(resolution: &str) -> Option<Size> {
    let (width, height) = resolution.split_once('x')?;
    let width: i32 = width.parse().ok()?;
    let height: i32 = height.parse().ok()?;
    Size::new(width, height)
        .is_positive()
        .then_some(Size::new(width, height))
}

/// Validate and round Hz to the millihertz used by output protocols.
pub(crate) fn refresh_hz_to_millihertz(rate: f64) -> Option<i32> {
    let millihertz = (rate * 1000.0).round();
    (rate.is_finite() && millihertz >= 1.0 && millihertz <= f64::from(i32::MAX))
        .then_some(millihertz as i32)
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

impl std::str::FromStr for OutputMode {
    type Err = String;

    /// Parse a mode string like "1920x1080@60.000" into an OutputMode.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("invalid monitor mode: {s}");
        let (res, rate_str) = s.split_once('@').ok_or_else(invalid)?;
        let size = parse_mode_dimensions(res).ok_or_else(invalid)?;
        let rate_hz: f64 = rate_str.parse().map_err(|_| invalid())?;
        let refresh_millihertz = refresh_hz_to_millihertz(rate_hz).ok_or_else(invalid)?;
        Ok(Self {
            width: size.w,
            height: size.h,
            refresh_millihertz,
        })
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
