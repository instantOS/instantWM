//! Smithay adapters for backend-neutral output types.

use crate::backend::output::{OutputMode, OutputTransform};
use crate::types::Size;
use smithay::output::Mode;
use smithay::utils::Transform;

/// Smallest output mode we ever publish, in pixels.
///
/// A nested host window can report a 0x0 size (minimized, unmapped, or
/// mid-startup), and a zero-sized `wl_output` mode is invalid. 64 is an
/// arbitrary safe floor, not a physical constraint — the point is only that
/// no downstream consumer ever sees a degenerate mode.
pub(crate) const MIN_OUTPUT_DIM: i32 = 64;

/// Clamp an output size so Smithay never sees a degenerate (zero or negative)
/// mode, per [`MIN_OUTPUT_DIM`].
///
/// This is a Wayland *output* policy, deliberately kept out of `Size`: sizes
/// for windows, icons, and bar surfaces have their own, much smaller floors.
pub(crate) fn clamp_output_size(size: Size) -> Size {
    Size::new(size.w.max(MIN_OUTPUT_DIM), size.h.max(MIN_OUTPUT_DIM))
}

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

    #[test]
    fn clamp_output_size_floors_each_axis_independently() {
        // A minimized/unmapped host window reports 0x0.
        assert_eq!(clamp_output_size(Size::new(0, 0)), Size::new(64, 64));
        assert_eq!(clamp_output_size(Size::new(1920, 0)), Size::new(1920, 64));
        // Negative dimensions must not survive either.
        assert_eq!(clamp_output_size(Size::new(-5, -5)), Size::new(64, 64));
        // Anything already at or above the floor is untouched.
        assert_eq!(
            clamp_output_size(Size::new(1920, 1080)),
            Size::new(1920, 1080)
        );
        assert_eq!(
            clamp_output_size(Size::new(MIN_OUTPUT_DIM, MIN_OUTPUT_DIM)),
            Size::new(MIN_OUTPUT_DIM, MIN_OUTPUT_DIM)
        );
    }
}
