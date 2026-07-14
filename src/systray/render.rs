use crate::bar::SystrayHitSlot;
use crate::bar::paint::{BarPainter, BarScheme};
use crate::systray::{MenuAction, MenuToggle, MenuView};
use crate::types::Rect;

/// Render a bar-hosted tray menu through the backend-independent bar painter.
pub(crate) fn draw_menu(
    painter: &mut dyn BarPainter,
    menu: &MenuView,
    cells: &[SystrayHitSlot],
    base_scheme: &BarScheme,
    bar_height: i32,
) {
    for cell in cells {
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
            let mut disabled_scheme = base_scheme.clone();
            disabled_scheme.fg[3] *= 0.55;
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
            6,
            &format!("{prefix}{}{suffix}", entry.label),
            false,
            0,
        );
        if !entry.enabled {
            painter.set_scheme(base_scheme.clone());
        }
    }
}
