use crate::bar::SystrayHitSlot;
use crate::bar::paint::{BarPainter, BarScheme, TextOverflow};
use crate::systray::{MenuAction, MenuToggle, MenuView};
use crate::types::Rect;

/// Render a bar-hosted tray menu through the backend-independent bar painter.
pub(crate) fn draw_menu(
    painter: &mut dyn BarPainter,
    menu: &MenuView,
    cells: &[SystrayHitSlot],
    base_scheme: &BarScheme,
    bar_height: i32,
    ui_scale: f64,
) {
    let scaled = |value: i32| ((value as f64 * ui_scale).round() as i32).max(1);
    for cell in cells {
        let Some(entry) = menu.entries.get(cell.idx) else {
            continue;
        };
        let width = cell.end - cell.start;
        if entry.separator {
            painter.rect(
                Rect::new(
                    cell.start + scaled(4),
                    bar_height / 2,
                    (width - scaled(8)).max(1),
                    scaled(1),
                ),
                false,
            );
            continue;
        }
        if !entry.enabled {
            let mut disabled_scheme = base_scheme.clone();
            disabled_scheme.foreground = disabled_scheme
                .foreground
                .with_alpha(disabled_scheme.foreground.a() * 0.55);
            painter.set_scheme(disabled_scheme);
        }
        let prefix = match entry.toggle {
            MenuToggle::Check(true) => "✓ ",
            MenuToggle::Check(false) => "□ ",
            MenuToggle::Radio(true) => "● ",
            MenuToggle::Radio(false) => "○ ",
            MenuToggle::None => "",
        };
        let suffix = if matches!(entry.action, MenuAction::OpenSubmenu(_)) {
            " ›"
        } else {
            ""
        };
        painter.text(
            Rect::new(cell.start, 0, width, bar_height),
            scaled(6),
            &format!("{prefix}{}{suffix}", entry.label),
            false,
            0,
            TextOverflow::Ellipsis,
        );
        if !entry.enabled {
            painter.set_scheme(base_scheme.clone());
        }
    }
}
