//! Display-independent system-tray model and bar geometry.
//!
//! StatusNotifier visual bounds and input bounds intentionally differ. Pixmaps
//! are never enlarged beyond their source resolution, while each item owns a
//! full-height cell. Legacy XEmbed windows share the sizing primitives but keep
//! their native rendering and input semantics.

use std::sync::Arc;

use crate::types::{Point, Rect, Size};

pub(crate) mod render;
pub(crate) mod status_notifier;

const MIN_VISUAL_PADDING: i32 = 2;
const MIN_MENU_CELL_WIDTH: i32 = 24;

/// Place a native context-menu toplevel next to its root-coordinate anchor.
/// Prefer opening leftward (tray icons normally live at the right edge) and
/// below a top bar, while keeping the complete window in the work area.
pub(crate) fn native_menu_rect(work_rect: Rect, requested: Rect, anchor: Point) -> Rect {
    let max_x = (work_rect.right() - requested.w).max(work_rect.x);
    let max_y = (work_rect.bottom() - requested.h).max(work_rect.y);

    let x = (anchor.x - requested.w).clamp(work_rect.x, max_x);
    let y = if anchor.y < work_rect.y {
        work_rect.y
    } else if anchor.y >= work_rect.bottom() {
        max_y
    } else if anchor.y + requested.h <= work_rect.bottom() {
        anchor.y
    } else {
        (anchor.y - requested.h).clamp(work_rect.y, max_y)
    };

    Rect::new(x, y, requested.w, requested.h)
}

/// An icon exported through the StatusNotifier protocol.
#[derive(Debug, Clone, Default)]
pub(crate) struct StatusNotifierItem {
    pub service: String,
    pub path: String,
    pub icon_rgba: Arc<[u8]>,
    pub icon_size: Size,
}

/// Current items exported through the StatusNotifier protocol.
#[derive(Debug, Clone, Default)]
pub(crate) struct StatusNotifierTray {
    pub items: Vec<StatusNotifierItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconScale {
    /// Preserve the source resolution and only shrink oversized pixmaps.
    DownOnly,
    /// Ask an embedded client window to occupy the available height.
    FitHeight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MenuAction {
    Activate(i32),
    OpenSubmenu(i32),
    Back,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum MenuToggle {
    #[default]
    None,
    Check(bool),
    Radio(bool),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MenuEntry {
    pub label: String,
    pub width: i32,
    pub enabled: bool,
    pub separator: bool,
    pub toggle: MenuToggle,
    pub action: MenuAction,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct MenuView {
    pub entries: Vec<MenuEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TrayMenuPresentation {
    pub session_id: u64,
    pub view: MenuView,
}

/// Main-thread view of the active tray menu session.
///
/// `accepting_updates` is deliberately separate from `view`: opening a menu is
/// asynchronous, and closing it must reject late updates for the same session.
#[derive(Debug, Default)]
pub(crate) struct TrayMenuState {
    session_id: u64,
    accepting_updates: bool,
    view: Option<MenuView>,
}

impl TrayMenuState {
    pub fn begin(&mut self, session_id: u64) -> bool {
        let changed = self.view.take().is_some();
        self.session_id = session_id;
        self.accepting_updates = true;
        changed
    }

    pub fn apply(&mut self, session_id: u64, view: Option<MenuView>) -> bool {
        if !self.accepting_updates || self.session_id != session_id {
            return false;
        }
        let changed = self.view != view;
        self.view = view;
        if self.view.is_none() {
            self.accepting_updates = false;
        }
        changed
    }

    pub fn close(&mut self) -> Option<u64> {
        if !self.accepting_updates && self.view.is_none() {
            return None;
        }
        self.accepting_updates = false;
        self.view = None;
        Some(self.session_id)
    }

    pub fn presentation(&self) -> Option<TrayMenuPresentation> {
        self.view.clone().map(|view| TrayMenuPresentation {
            session_id: self.session_id,
            view,
        })
    }

    pub fn current(&self) -> Option<TrayMenuPresentation> {
        self.presentation()
    }
}

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
    pub menu_start_x: i32,
    pub menu_width: i32,
    pub menu_cells: Vec<crate::bar::SystrayHitSlot>,
}

pub(crate) fn layout(
    tray: &StatusNotifierTray,
    menu: Option<&MenuView>,
    monitor_width: i32,
    bar_height: i32,
    configured_padding: i32,
) -> TrayLayout {
    let bar_height = bar_height.max(1);
    let padding = configured_padding
        .max(MIN_VISUAL_PADDING)
        .min((bar_height - 1) / 2);
    let max_icon_height = (bar_height - 2 * padding).max(1);

    let dimensions: Vec<(i32, Size)> = tray
        .items
        .iter()
        .map(|item| {
            let icon_size = fit_icon_size(item.icon_size, max_icon_height, IconScale::DownOnly);
            let cell_width = bar_height.max(icon_size.w + 2 * padding);
            (cell_width, icon_size)
        })
        .collect();
    let total_width = dimensions.iter().map(|(cell_width, _)| cell_width).sum();
    let start_x = monitor_width - total_width;

    let mut x = start_x;
    let cells = dimensions
        .into_iter()
        .enumerate()
        .map(|(idx, (cell_width, icon_size))| {
            let icon_x = x + (cell_width - icon_size.w) / 2;
            let icon_y = (bar_height - icon_size.h) / 2;
            let cell = TrayCell {
                idx,
                hit_start: x,
                hit_end: x + cell_width,
                icon: Rect::from_position_and_size(Point::new(icon_x, icon_y), icon_size),
            };
            x += cell_width;
            cell
        })
        .collect();

    let (menu_start_x, menu_width, menu_cells) = menu_layout(menu, start_x);

    TrayLayout {
        start_x,
        total_width,
        cells,
        menu_start_x,
        menu_width,
        menu_cells,
    }
}

fn menu_layout(
    menu: Option<&MenuView>,
    available_width: i32,
) -> (i32, i32, Vec<crate::bar::SystrayHitSlot>) {
    let Some(menu) = menu.filter(|menu| !menu.entries.is_empty()) else {
        return (available_width, 0, Vec::new());
    };
    let available_width = available_width.max(0);
    let widths = fit_menu_widths(
        menu.entries
            .iter()
            .map(|entry| entry.width.max(MIN_MENU_CELL_WIDTH))
            .collect(),
        available_width,
    );
    let menu_width = widths.iter().sum();
    let menu_start_x = available_width - menu_width;
    let mut x = menu_start_x;
    let cells = widths
        .into_iter()
        .enumerate()
        .filter_map(|(idx, width)| {
            if width <= 0 {
                return None;
            }
            let cell = crate::bar::SystrayHitSlot {
                idx,
                start: x,
                end: x + width,
            };
            x += width;
            Some(cell)
        })
        .collect();
    (menu_start_x, menu_width, cells)
}

fn fit_menu_widths(mut widths: Vec<i32>, available: i32) -> Vec<i32> {
    let natural: i32 = widths.iter().sum();
    if natural <= available || natural <= 0 {
        return widths;
    }
    if available <= 0 {
        widths.fill(0);
        return widths;
    }

    let count = widths.len() as i32;
    if available < count {
        // A one-dimensional menu cannot give more entries distinct hit targets
        // than there are pixels. Keep the geometry on-screen and omit the
        // unrepresentable tail rather than drawing it beyond the output.
        for (index, width) in widths.iter_mut().enumerate() {
            *width = i32::from(index < available as usize);
        }
        return widths;
    }

    // Preserve every item on narrow outputs. When there is enough room, retain
    // a useful minimum target; otherwise distribute every available pixel.
    let floor = if available >= count * MIN_MENU_CELL_WIDTH {
        MIN_MENU_CELL_WIDTH
    } else {
        1
    };
    let distributable = (available - floor * count).max(0);
    let natural_extra: i32 = widths.iter().map(|width| (width - floor).max(0)).sum();
    let mut used = 0;
    for width in &mut widths {
        let extra = if natural_extra > 0 {
            distributable * (*width - floor).max(0) / natural_extra
        } else {
            distributable / count.max(1)
        };
        *width = floor + extra;
        used += *width;
    }
    let mut remainder = available - used;
    for width in &mut widths {
        if remainder <= 0 {
            break;
        }
        *width += 1;
        remainder -= 1;
    }
    widths
}

/// Fit an icon inside the available height while preserving aspect ratio.
/// Source images are never enlarged: interpolation can make an enlarged icon
/// softer, but it cannot add the detail omitted by the StatusNotifier item.
pub(crate) fn fit_icon_size(source_size: Size, target_height: i32, scale: IconScale) -> Size {
    if !source_size.is_positive() || target_height <= 0 {
        return Size::default();
    }
    let height = match scale {
        IconScale::DownOnly => source_size.h.min(target_height),
        IconScale::FitHeight => target_height,
    };
    let width = if height == source_size.h {
        source_size.w
    } else {
        ((source_size.w as i64 * height as i64 + source_size.h as i64 / 2) / source_size.h as i64)
            as i32
    };
    Size::new(width.max(1), height.max(1))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::systray::StatusNotifierItem;

    fn item(width: i32, height: i32) -> StatusNotifierItem {
        StatusNotifierItem {
            icon_size: Size::new(width, height),
            icon_rgba: Arc::from(vec![0; (width * height * 4) as usize]),
            ..StatusNotifierItem::default()
        }
    }

    #[test]
    fn native_size_icon_is_centered_in_a_full_height_cell() {
        let tray = StatusNotifierTray {
            items: vec![item(16, 16)],
        };
        let layout = layout(&tray, None, 1920, 30, 0);

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
        let tray = StatusNotifierTray {
            items: vec![item(16, 16), item(24, 12)],
        };
        let layout = layout(&tray, None, 100, 30, 4);

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
        let tray = StatusNotifierTray {
            items: vec![item(64, 64), item(8, 8)],
        };
        let layout = layout(&tray, None, 100, 30, 2);

        assert_eq!((layout.cells[0].icon.w, layout.cells[0].icon.h), (26, 26));
        assert_eq!((layout.cells[1].icon.w, layout.cells[1].icon.h), (8, 8));
    }

    #[test]
    fn embedded_icon_windows_fill_height_without_losing_aspect_ratio() {
        assert_eq!(
            fit_icon_size(Size::new(16, 16), 30, IconScale::FitHeight),
            Size::new(30, 30)
        );
        assert_eq!(
            fit_icon_size(Size::new(32, 16), 30, IconScale::FitHeight),
            Size::new(60, 30)
        );
    }

    #[test]
    fn invalid_icon_dimensions_are_rejected_for_every_backend_policy() {
        for scale in [IconScale::DownOnly, IconScale::FitHeight] {
            assert_eq!(fit_icon_size(Size::new(16, 0), 30, scale), Size::default());
            assert_eq!(fit_icon_size(Size::new(-1, 16), 30, scale), Size::default());
        }
    }

    #[test]
    fn native_menu_opens_left_and_below_a_top_bar() {
        let rect = native_menu_rect(
            Rect::new(1920, 32, 1920, 1048),
            Rect::new(0, 0, 320, 480),
            Point::new(3820, 16),
        );

        assert_eq!(rect, Rect::new(3500, 32, 320, 480));
    }

    #[test]
    fn native_menu_opens_above_a_bottom_bar() {
        let rect = native_menu_rect(
            Rect::new(0, 0, 1920, 1048),
            Rect::new(0, 0, 240, 300),
            Point::new(1900, 1064),
        );

        assert_eq!(rect, Rect::new(1660, 748, 240, 300));
    }

    #[test]
    fn closed_menu_rejects_late_updates_from_its_session() {
        let mut state = TrayMenuState::default();
        state.begin(7);
        assert!(state.apply(7, Some(MenuView::default())));
        assert_eq!(state.close(), Some(7));

        assert!(!state.apply(7, Some(MenuView::default())));
        assert!(state.presentation().is_none());
    }

    #[test]
    fn newer_menu_session_rejects_updates_from_an_older_session() {
        let mut state = TrayMenuState::default();
        state.begin(3);
        state.begin(4);

        assert!(!state.apply(3, Some(MenuView::default())));
        assert!(state.apply(4, Some(MenuView::default())));
        assert_eq!(state.presentation().unwrap().session_id, 4);
    }

    #[test]
    fn menu_is_fitted_left_of_tray_without_losing_entries() {
        let tray = StatusNotifierTray {
            items: vec![item(16, 16)],
        };
        let menu = MenuView {
            entries: (0..4)
                .map(|id| MenuEntry {
                    label: format!("item {id}"),
                    width: 80,
                    enabled: true,
                    separator: false,
                    toggle: MenuToggle::None,
                    action: MenuAction::Activate(id),
                })
                .collect(),
        };
        let layout = layout(&tray, Some(&menu), 230, 30, 2);

        assert_eq!(layout.menu_start_x, 0);
        assert_eq!(layout.menu_width, 200);
        assert_eq!(layout.menu_cells.len(), menu.entries.len());
        assert_eq!(layout.menu_cells.last().unwrap().end, layout.start_x);
    }

    #[test]
    fn menu_never_extends_left_when_entries_outnumber_available_pixels() {
        let menu = MenuView {
            entries: (0..5)
                .map(|id| MenuEntry {
                    label: format!("item {id}"),
                    width: 80,
                    enabled: true,
                    separator: false,
                    toggle: MenuToggle::None,
                    action: MenuAction::Activate(id),
                })
                .collect(),
        };

        let layout = layout(&StatusNotifierTray::default(), Some(&menu), 3, 30, 2);

        assert_eq!(layout.menu_start_x, 0);
        assert_eq!(layout.menu_width, 3);
        assert_eq!(layout.menu_cells.len(), 3);
        assert!(layout.menu_cells.iter().all(|cell| cell.start >= 0));
        assert_eq!(layout.menu_cells.last().unwrap().end, 3);
    }
}
