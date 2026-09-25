//! Smithay adapters for backend-neutral output types.

use crate::backend::output::{OutputMode, OutputTransform};
use smithay::output::Mode;
use smithay::utils::Transform;

pub(crate) fn to_smithay_mode(mode: OutputMode) -> Mode {
    Mode {
        size: (mode.width, mode.height).into(),
        refresh: mode.refresh_millihertz,
    }
}

pub(crate) fn from_smithay_mode(mode: Mode) -> OutputMode {
    OutputMode {
        width: mode.size.w,
        height: mode.size.h,
        refresh_millihertz: mode.refresh,
    }
}

impl From<Mode> for OutputMode {
    fn from(mode: Mode) -> Self {
        from_smithay_mode(mode)
    }
}

pub(crate) fn to_smithay_transform(transform: OutputTransform) -> Transform {
    match transform {
        OutputTransform::Normal => Transform::Normal,
        OutputTransform::Rotate90 => Transform::_90,
        OutputTransform::Rotate180 => Transform::_180,
        OutputTransform::Rotate270 => Transform::_270,
        OutputTransform::Flipped => Transform::Flipped,
        OutputTransform::Flipped90 => Transform::Flipped90,
        OutputTransform::Flipped180 => Transform::Flipped180,
        OutputTransform::Flipped270 => Transform::Flipped270,
    }
}

pub(crate) fn from_smithay_transform(transform: Transform) -> OutputTransform {
    match transform {
        Transform::Normal => OutputTransform::Normal,
        Transform::_90 => OutputTransform::Rotate90,
        Transform::_180 => OutputTransform::Rotate180,
        Transform::_270 => OutputTransform::Rotate270,
        Transform::Flipped => OutputTransform::Flipped,
        Transform::Flipped90 => OutputTransform::Flipped90,
        Transform::Flipped180 => OutputTransform::Flipped180,
        Transform::Flipped270 => OutputTransform::Flipped270,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_output_transform_round_trips_through_smithay() {
        for transform in [
            OutputTransform::Normal,
            OutputTransform::Rotate90,
            OutputTransform::Rotate180,
            OutputTransform::Rotate270,
            OutputTransform::Flipped,
            OutputTransform::Flipped90,
            OutputTransform::Flipped180,
            OutputTransform::Flipped270,
        ] {
            assert_eq!(
                from_smithay_transform(to_smithay_transform(transform)),
                transform
            );
        }
    }

    #[test]
    fn mode_round_trips_through_smithay() {
        let mode = OutputMode {
            width: 1920,
            height: 1080,
            refresh_millihertz: 144_000,
        };
        assert_eq!(from_smithay_mode(to_smithay_mode(mode)), mode);
        assert_eq!(OutputMode::from(to_smithay_mode(mode)), mode);
    }
}
