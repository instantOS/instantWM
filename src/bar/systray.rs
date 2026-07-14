//! Geometry for StatusNotifier items rendered inside the bar.
//!
//! A tray item's visual bounds and input bounds intentionally differ.  Icons
//! are never enlarged beyond their source resolution, while each item owns a
//! full-height cell.  Adjacent cells touch, so padding improves legibility
//! without creating dead input zones between icons.

use crate::types::{Rect, WaylandSystray};

const MIN_VISUAL_PADDING: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrayCell {
    pub idx: usize,
    pub hit_start: i32,
    pub hit_end: i32,
    pub icon: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TrayLayout {
    pub start_x: i32,
    pub total_width: i32,
    pub cells: Vec<TrayCell>,
}

pub(crate) fn layout(
    tray: &WaylandSystray,
    monitor_width: i32,
    bar_height: i32,
    configured_padding: i32,
) -> TrayLayout {
    let bar_height = bar_height.max(1);
    let padding = configured_padding
        .max(MIN_VISUAL_PADDING)
        .min((bar_height - 1) / 2);
    let max_icon_height = (bar_height - 2 * padding).max(1);

    let dimensions: Vec<(i32, i32, i32)> = tray
        .items
        .iter()
        .map(|item| {
            let (icon_width, icon_height) =
                fitted_icon_size(item.icon_w, item.icon_h, max_icon_height);
            let cell_width = bar_height.max(icon_width + 2 * padding);
            (cell_width, icon_width, icon_height)
        })
        .collect();
    let total_width = dimensions.iter().map(|(cell_width, _, _)| cell_width).sum();
    let start_x = monitor_width - total_width;

    let mut x = start_x;
    let cells = dimensions
        .into_iter()
        .enumerate()
        .map(|(idx, (cell_width, icon_width, icon_height))| {
            let icon_x = x + (cell_width - icon_width) / 2;
            let icon_y = (bar_height - icon_height) / 2;
            let cell = TrayCell {
                idx,
                hit_start: x,
                hit_end: x + cell_width,
                icon: Rect::new(icon_x, icon_y, icon_width, icon_height),
            };
            x += cell_width;
            cell
        })
        .collect();

    TrayLayout {
        start_x,
        total_width,
        cells,
    }
}

/// Fit an icon inside the available height while preserving aspect ratio.
/// Source images are never enlarged: interpolation can make an enlarged icon
/// softer, but it cannot add the detail omitted by the StatusNotifier item.
fn fitted_icon_size(source_width: i32, source_height: i32, max_height: i32) -> (i32, i32) {
    if source_width <= 0 || source_height <= 0 || max_height <= 0 {
        return (0, 0);
    }
    let height = source_height.min(max_height);
    let width = if height == source_height {
        source_width
    } else {
        ((source_width as i64 * height as i64 + source_height as i64 / 2) / source_height as i64)
            as i32
    };
    (width.max(1), height.max(1))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::types::WaylandSystrayItem;

    fn item(width: i32, height: i32) -> WaylandSystrayItem {
        WaylandSystrayItem {
            icon_w: width,
            icon_h: height,
            icon_rgba: Arc::from(vec![0; (width * height * 4) as usize]),
            ..WaylandSystrayItem::default()
        }
    }

    #[test]
    fn native_size_icon_is_centered_in_a_full_height_cell() {
        let tray = WaylandSystray {
            items: vec![item(16, 16)],
        };
        let layout = layout(&tray, 1920, 30, 0);

        assert_eq!(layout.total_width, 30);
        assert_eq!(layout.start_x, 1890);
        assert_eq!(layout.cells[0].icon, Rect::new(1897, 7, 16, 16));
        assert_eq!(
            (layout.cells[0].hit_start, layout.cells[0].hit_end),
            (1890, 1920)
        );
    }

    #[test]
    fn adjacent_cells_have_no_dead_input_space() {
        let tray = WaylandSystray {
            items: vec![item(16, 16), item(24, 12)],
        };
        let layout = layout(&tray, 100, 30, 4);

        assert_eq!(layout.cells[0].hit_end, layout.cells[1].hit_start);
        assert!(layout.cells.iter().all(|cell| cell.icon.y > 0));
        assert!(
            layout
                .cells
                .iter()
                .all(|cell| cell.icon.y + cell.icon.h < 30)
        );
    }

    #[test]
    fn large_icons_shrink_but_small_icons_do_not_grow() {
        let tray = WaylandSystray {
            items: vec![item(64, 64), item(8, 8)],
        };
        let layout = layout(&tray, 100, 30, 2);

        assert_eq!((layout.cells[0].icon.w, layout.cells[0].icon.h), (26, 26));
        assert_eq!((layout.cells[1].icon.w, layout.cells[1].icon.h), (8, 8));
    }
}
