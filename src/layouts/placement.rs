//! Shared geometry application for tiling layouts.
//!
//! Layout algorithms compute gapless *slots*.  This module is the only place
//! that turns those slots into final client rectangles by applying outer gaps,
//! inner gaps, and border subtraction.

use crate::config::config_toml::LayoutConfig;
use crate::constants::animation::BORDER_MULTIPLIER;
use crate::layouts::PresentationMode;
use crate::types::{Monitor, Rect};

/// Thickness of the hollow manual-placement preview frame in logical pixels.
/// It is intentionally independent of ordinary client borders: users may
/// configure those to zero while the placement affordance must remain clear.
pub(crate) const LAYOUT_PREVIEW_BORDER_WIDTH: i32 = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayoutPlacement {
    work_rect: Rect,
    inner_gap: i32,
}

impl LayoutPlacement {
    pub(crate) fn new(
        cfg: &LayoutConfig,
        monitor: &Monitor,
        layout: PresentationMode,
        tiled_client_count: u32,
    ) -> Self {
        let smart_gaps_disabled = cfg.smart_gaps && tiled_client_count <= 1;
        let maximized_disabled = layout.is_maximized() && !cfg.maximized_gaps;
        let gaps_enabled = layout.is_tiling() && !smart_gaps_disabled && !maximized_disabled;

        let outer_gap = if gaps_enabled {
            cfg.outer_gap.max(0)
        } else {
            0
        };
        let inner_gap = if gaps_enabled {
            cfg.inner_gap.max(0)
        } else {
            0
        };

        Self {
            work_rect: inset_rect_saturating(
                monitor.work_rect(),
                outer_gap,
                outer_gap,
                outer_gap,
                outer_gap,
            ),
            inner_gap,
        }
    }

    #[inline]
    pub(crate) fn work_rect(self) -> Rect {
        self.work_rect
    }

    pub(crate) fn client_rect(self, slot: Rect, border_width: i32) -> Rect {
        let gapped = self.apply_inner_gap(slot);
        Rect {
            x: gapped.x,
            y: gapped.y,
            w: (gapped.w - BORDER_MULTIPLIER * border_width).max(1),
            h: (gapped.h - BORDER_MULTIPLIER * border_width).max(1),
        }
    }

    fn apply_inner_gap(self, slot: Rect) -> Rect {
        if self.inner_gap <= 0 {
            return slot;
        }

        let half = self.inner_gap / 2;
        let other_half = self.inner_gap - half;
        let work_right = self.work_rect().x + self.work_rect().w;
        let work_bottom = self.work_rect().y + self.work_rect().h;
        let slot_right = slot.x + slot.w;
        let slot_bottom = slot.y + slot.h;

        let left = if slot.x <= self.work_rect().x {
            0
        } else {
            half
        };
        let top = if slot.y <= self.work_rect().y {
            0
        } else {
            half
        };
        let right = if slot_right >= work_right {
            0
        } else {
            other_half
        };
        let bottom = if slot_bottom >= work_bottom {
            0
        } else {
            other_half
        };

        inset_rect_saturating(slot, left, top, right, bottom)
    }
}

fn inset_rect_saturating(rect: Rect, left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    let left = left.max(0);
    let top = top.max(0);
    let right = right.max(0);
    let bottom = bottom.max(0);

    let max_horizontal = (rect.w - 1).max(0);
    let used_left = left.min(max_horizontal);
    let used_right = right.min(max_horizontal - used_left);

    let max_vertical = (rect.h - 1).max(0);
    let used_top = top.min(max_vertical);
    let used_bottom = bottom.min(max_vertical - used_top);

    Rect {
        x: rect.x + used_left,
        y: rect.y + used_top,
        w: (rect.w - used_left - used_right).max(1),
        h: (rect.h - used_top - used_bottom).max(1),
    }
}

/// Four non-overlapping rectangles forming a hollow frame inside `rect`.
/// Used by both backends for the manual-tree placement preview.
pub(crate) fn outline_rectangles(rect: Rect, requested_width: i32) -> [Rect; 4] {
    let width = requested_width
        .max(1)
        .min((rect.w.max(1) + 1) / 2)
        .min((rect.h.max(1) + 1) / 2);
    let inner_height = (rect.h - 2 * width).max(0);
    [
        Rect::new(rect.x, rect.y, rect.w.max(1), width),
        Rect::new(rect.x, rect.y + rect.h - width, rect.w.max(1), width),
        Rect::new(rect.x, rect.y + width, width, inner_height),
        Rect::new(rect.x + rect.w - width, rect.y + width, width, inner_height),
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        LAYOUT_PREVIEW_BORDER_WIDTH, LayoutPlacement, inset_rect_saturating, outline_rectangles,
    };
    use crate::config::config_toml::LayoutConfig;
    use crate::layouts::PresentationMode;
    use crate::types::{Monitor, Rect};

    fn config_with_gaps(inner_gap: i32, outer_gap: i32, smart_gaps: bool) -> LayoutConfig {
        LayoutConfig {
            inner_gap,
            outer_gap,
            smart_gaps,
            maximized_gaps: false,
            ..LayoutConfig::default()
        }
    }

    fn monitor_with_work_rect(work_rect: Rect) -> Monitor {
        Monitor {
            available_rect: work_rect,
            ..Monitor::default()
        }
    }

    #[test]
    fn no_gaps_preserves_slot_except_border() {
        let cfg = config_with_gaps(0, 0, false);
        let monitor = monitor_with_work_rect(Rect::new(0, 0, 100, 80));
        let placement = LayoutPlacement::new(&cfg, &monitor, PresentationMode::Tiled, 2);

        assert_eq!(placement.work_rect(), Rect::new(0, 0, 100, 80));
        assert_eq!(
            placement.client_rect(Rect::new(0, 0, 50, 80), 1),
            Rect::new(0, 0, 48, 78)
        );
    }

    #[test]
    fn outer_gap_shrinks_layout_work_rect() {
        let cfg = config_with_gaps(0, 8, false);
        let monitor = monitor_with_work_rect(Rect::new(10, 20, 100, 80));
        let placement = LayoutPlacement::new(&cfg, &monitor, PresentationMode::Tiled, 2);

        assert_eq!(placement.work_rect(), Rect::new(18, 28, 84, 64));
    }

    #[test]
    fn inner_gap_is_split_between_adjacent_slots() {
        let cfg = config_with_gaps(9, 0, false);
        let monitor = monitor_with_work_rect(Rect::new(0, 0, 100, 80));
        let placement = LayoutPlacement::new(&cfg, &monitor, PresentationMode::Tiled, 2);

        let left = placement.client_rect(Rect::new(0, 0, 50, 80), 0);
        let right = placement.client_rect(Rect::new(50, 0, 50, 80), 0);

        assert_eq!(left, Rect::new(0, 0, 45, 80));
        assert_eq!(right, Rect::new(54, 0, 46, 80));
        assert_eq!(right.x - (left.x + left.w), 9);
    }

    #[test]
    fn smart_gaps_disable_all_gaps_for_single_tiled_client() {
        let cfg = config_with_gaps(8, 8, true);
        let monitor = monitor_with_work_rect(Rect::new(0, 0, 100, 80));
        let placement = LayoutPlacement::new(&cfg, &monitor, PresentationMode::Tiled, 1);

        assert_eq!(placement.work_rect(), Rect::new(0, 0, 100, 80));
        assert_eq!(
            placement.client_rect(Rect::new(0, 0, 100, 80), 0),
            Rect::new(0, 0, 100, 80)
        );
    }

    #[test]
    fn maximized_ignores_gaps_by_default() {
        let cfg = config_with_gaps(8, 8, false);
        let monitor = monitor_with_work_rect(Rect::new(0, 0, 100, 80));
        let placement = LayoutPlacement::new(&cfg, &monitor, PresentationMode::Maximized, 3);

        assert_eq!(placement.work_rect(), Rect::new(0, 0, 100, 80));
    }

    #[test]
    fn inset_never_collapses_rect() {
        assert_eq!(
            inset_rect_saturating(Rect::new(0, 0, 4, 3), 8, 8, 8, 8),
            Rect::new(3, 2, 1, 1)
        );
    }

    #[test]
    fn outline_is_hollow_and_keeps_all_sides_inside_the_rect() {
        assert_eq!(
            outline_rectangles(Rect::new(10, 20, 100, 80), LAYOUT_PREVIEW_BORDER_WIDTH,),
            [
                Rect::new(10, 20, 100, 6),
                Rect::new(10, 94, 100, 6),
                Rect::new(10, 26, 6, 68),
                Rect::new(104, 26, 6, 68),
            ]
        );
    }
}
