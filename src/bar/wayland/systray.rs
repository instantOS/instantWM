use crate::bar::paint::BarPainter;
use crate::bar::scene;
use crate::types::geometry::Rect;

use super::WaylandBarPainter;

pub(super) fn draw_snapshot(
    painter: &mut WaylandBarPainter,
    snapshot: &scene::SystraySnapshot,
    layout: &scene::WorkerTrayLayout,
    bar_height: i32,
) {
    painter.set_scheme(snapshot.base_scheme.clone());
    if layout.tray_total_w > 0 {
        painter.rect(
            Rect::new(layout.tray_start_x, 0, layout.tray_total_w, bar_height),
            true,
            true,
        );
    }
    if layout.menu_total_w > 0 {
        painter.rect(
            Rect::new(layout.menu_start_x, 0, layout.menu_total_w, bar_height),
            true,
            true,
        );
    }

    let icon_h = bar_height.max(1);
    for slot in &layout.tray_slots {
        let Some(item) = snapshot.items.items.get(slot.idx) else {
            continue;
        };
        painter.blit_rgba_bgra(
            slot.start,
            0,
            slot.end - slot.start,
            icon_h,
            item.icon_w,
            item.icon_h,
            &item.icon_rgba,
        );
    }

    if let Some(menu) = &snapshot.menu {
        let mut scheme = snapshot.base_scheme.clone();
        painter.set_scheme(scheme.clone());
        for (row, item) in menu.items.iter().enumerate() {
            let Some(slot) = layout.menu_slots.get(row) else {
                continue;
            };
            let x = slot.start;
            let w = slot.end - slot.start;
            if item.separator {
                painter.rect(Rect::new(x + 3, bar_height / 2, w - 6, 1), true, false);
                continue;
            }
            if !item.enabled {
                scheme.fg[3] = 0.6;
                painter.set_scheme(scheme.clone());
            }
            painter.text(Rect::new(x, 0, w, bar_height), 8, &item.label, false, 0);
            if !item.enabled {
                scheme.fg[3] = 1.0;
                painter.set_scheme(scheme.clone());
            }
        }
    }
}
