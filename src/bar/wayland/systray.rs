use crate::bar::paint::BarPainter;
use crate::bar::scene;
use crate::types::geometry::Rect;

use super::WaylandBarPainter;

pub(super) fn draw_snapshot(
    painter: &mut WaylandBarPainter,
    snapshot: &scene::SystraySnapshot,
    menu: Option<&crate::systray::TrayMenuPresentation>,
    layout: &crate::systray::TrayLayout,
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

    let Some(menu) = menu else {
        return;
    };
    crate::systray::render::draw_menu(
        painter,
        &menu.view,
        &layout.menu_cells,
        &snapshot.base_scheme,
        bar_height,
    );
}
