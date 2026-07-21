//! Public command and target vocabulary for the manual layout tree.
//!
//! These types deliberately contain no tree representation. Callers can ask
//! for semantic operations, but cannot construct malformed nodes or splits.

use crate::types::{Point, WindowId};

pub(super) const DEFAULT_RESIZE_STEP: f64 = 0.05;
pub(super) const DEFAULT_MINIMUM_WEIGHT: f64 = 0.15;

#[derive(Debug, Clone, Copy)]
pub struct CommandConfig {
    pub resize_step: f64,
    pub minimum_weight: f64,
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            resize_step: DEFAULT_RESIZE_STEP,
            minimum_weight: DEFAULT_MINIMUM_WEIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Divide a rectangle into left-to-right children.
    Vertical,
    /// Divide a rectangle into top-to-bottom children.
    Horizontal,
}

impl Axis {
    pub const fn other(self) -> Self {
        match self {
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Vertical,
            Self::Top | Self::Bottom => Axis::Horizontal,
        }
    }

    pub const fn is_leading(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
}

/// One-shot transformations replacing the old continuously active algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    MasterStack,
    Grid,
    HorizontalGrid,
    BottomStack,
    BottomStackHorizontal,
    /// Preserve every leaf while giving the selected one a dominant slot.
    Focus,
}

/// Opaque semantic target shared by pointer and keyboard placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementTarget {
    pub target: WindowId,
    pub side: Option<Side>,
    pub candidate_index: usize,
    pub position: Point,
}
