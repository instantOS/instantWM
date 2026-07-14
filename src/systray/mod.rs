//! Display-independent system-tray model and bar geometry.
//!
//! StatusNotifier visual bounds and input bounds intentionally differ. Pixmaps
//! are never enlarged beyond their source resolution, while each item owns a
//! full-height cell. Legacy XEmbed windows share the sizing primitives but keep
//! their native rendering and input semantics.

use std::sync::Arc;

use crate::types::Rect;

pub(crate) mod render;
pub(crate) mod status_notifier;

const MIN_VISUAL_PADDING: i32 = 2;
const MIN_MENU_CELL_WIDTH: i32 = 24;

/// An icon exported through the StatusNotifier protocol.
#[derive(Debug, Clone, Default)]
pub(crate) struct StatusNotifierItem {
    pub service: String,
    pub path: String,
    pub icon_rgba: Arc<[u8]>,
    pub icon_w: i32,
    pub icon_h: i32,
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

    let dimensions: Vec<(i32, i32, i32)> = tray
        .items
        .iter()
        .map(|item| {
            let (icon_width, icon_height) = fit_icon_size(
                item.icon_w,
                item.icon_h,
                max_icon_height,
                IconScale::DownOnly,
            );
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
        .map(|(idx, width)| {
            let cell = crate::bar::SystrayHitSlot {
                idx,
                start: x,
                end: x + width,
            };
            x += width;
            cell
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

    // Preserve every item even on narrow outputs.  When there is enough room,
    // retain a useful minimum target; otherwise distribute every pixel so no
    // entry becomes unreachable beyond the left edge.
    let count = widths.len() as i32;
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
pub(crate) fn fit_icon_size(
    source_width: i32,
    source_height: i32,
    target_height: i32,
    scale: IconScale,
) -> (i32, i32) {
    if source_width <= 0 || source_height <= 0 || target_height <= 0 {
        return (0, 0);
    }
    let height = match scale {
        IconScale::DownOnly => source_height.min(target_height),
        IconScale::FitHeight => target_height,
    };
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
    use crate::systray::StatusNotifierItem;

    fn item(width: i32, height: i32) -> StatusNotifierItem {
        StatusNotifierItem {
            icon_w: width,
            icon_h: height,
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
        assert_eq!(fit_icon_size(16, 16, 30, IconScale::FitHeight), (30, 30));
        assert_eq!(fit_icon_size(32, 16, 30, IconScale::FitHeight), (60, 30));
    }

    #[test]
    fn invalid_icon_dimensions_are_rejected_for_every_backend_policy() {
        for scale in [IconScale::DownOnly, IconScale::FitHeight] {
            assert_eq!(fit_icon_size(16, 0, 30, scale), (0, 0));
            assert_eq!(fit_icon_size(-1, 16, 30, scale), (0, 0));
        }
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
}
