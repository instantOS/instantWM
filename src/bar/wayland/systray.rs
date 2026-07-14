use crate::bar::paint::BarPainter;
use crate::bar::scene;
use crate::types::geometry::Rect;

use super::WaylandBarPainter;

pub(super) fn draw_snapshot(
    painter: &mut WaylandBarPainter,
    snapshot: &scene::SystraySnapshot,
    layout: &crate::bar::systray::TrayLayout,
    bar_height: i32,
) {
    painter.set_scheme(snapshot.base_scheme.clone());
    if layout.total_width > 0 {
        painter.rect(
            Rect::new(layout.start_x, 0, layout.total_width, bar_height),
            true,
            true,
        );
    }
    if layout.menu_width > 0 {
        painter.rect(
            Rect::new(layout.menu_start_x, 0, layout.menu_width, bar_height),
            true,
            true,
        );
    }

    for cell in &layout.cells {
        let Some(item) = snapshot.items.items.get(cell.idx) else {
            continue;
        };
        painter.blit_rgba_bgra(
            cell.icon.x,
            cell.icon.y,
            cell.icon.w,
            cell.icon.h,
            item.icon_w,
            item.icon_h,
            &item.icon_rgba,
        );
    }

    let Some(menu) = snapshot.menu.as_ref() else {
        return;
    };
    let mut scheme = snapshot.base_scheme.clone();
    for cell in &layout.menu_cells {
        let Some(entry) = menu.entries.get(cell.idx) else {
            continue;
        };
        let width = cell.end - cell.start;
        if entry.separator {
            painter.rect(
                Rect::new(cell.start + 4, bar_height / 2, (width - 8).max(1), 1),
                true,
                false,
            );
            continue;
        }
        if !entry.enabled {
            scheme.fg[3] = 0.55;
            painter.set_scheme(scheme.clone());
        }
        let prefix = match entry.toggle {
            crate::bar::systray::MenuToggle::Check(true) => "✓ ",
            crate::bar::systray::MenuToggle::Check(false) => "□ ",
            crate::bar::systray::MenuToggle::Radio(true) => "● ",
            crate::bar::systray::MenuToggle::Radio(false) => "○ ",
            crate::bar::systray::MenuToggle::None => "",
        };
        let suffix = if matches!(
            entry.action,
            crate::bar::systray::MenuAction::OpenSubmenu(_)
        ) {
            " ›"
        } else {
            ""
        };
        painter.text(
            Rect::new(cell.start, 0, width, bar_height),
            6,
            &format!("{prefix}{}{suffix}", entry.label),
            false,
            0,
        );
        if !entry.enabled {
            scheme.fg[3] = 1.0;
            painter.set_scheme(scheme.clone());
        }
    }
}
