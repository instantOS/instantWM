use crate::bar::SystrayHitSlot;
use crate::bar::paint::{BarPainter, BarScheme, TextOverflow, draw_hover_accent};
#[allow(unused_imports)]
use crate::systray::{MenuAction, MenuToggle, MenuView};
use crate::types::Rect;
use crate::types::color::Rgba;

/// Hover presentation for one bar-hosted menu entry, mirroring the status
/// block hover accent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrayMenuHover {
    /// 0-based index into the menu view's entries.
    pub entry_index: usize,
    pub color: Rgba,
}

/// Render a bar-hosted tray menu through the backend-independent bar painter.
pub(crate) fn draw_menu(
    painter: &mut dyn BarPainter,
    menu: &MenuView,
    cells: &[SystrayHitSlot],
    base_scheme: &BarScheme,
    bar_height: i32,
    hover: Option<TrayMenuHover>,
    ui_scale: f64,
) {
    painter.set_scheme(base_scheme.clone());
    draw_entry_separators(painter, menu, cells, base_scheme, bar_height, ui_scale);
    for cell in cells {
        let Some(entry) = menu.entries.get(cell.idx) else {
            continue;
        };
        let width = cell.end - cell.start;
        if entry.separator {
            draw_separator_entry(
                painter,
                cell.start,
                width,
                base_scheme,
                bar_height,
                ui_scale,
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
        painter.text(
            Rect::new(cell.start, 0, width, bar_height),
            scaled_px(6, ui_scale),
            &entry.display_label(),
            false,
            0,
            TextOverflow::Ellipsis,
        );
        if !entry.enabled {
            painter.set_scheme(base_scheme.clone());
        }
        if let Some(hover) = hover.filter(|hover| hover.entry_index == cell.idx) {
            draw_hover_accent(
                painter,
                Rect::new(cell.start, 0, width, bar_height),
                hover.color,
            );
        }
    }
}

use crate::types::geometry::scaled_px;

/// Thin vertical rules between adjacent ordinary entries, in the style of the
/// i3bar block separators. Boundaries next to a separator entry already read
/// as a break, so they are not doubled up.
fn draw_entry_separators(
    painter: &mut dyn BarPainter,
    menu: &MenuView,
    cells: &[SystrayHitSlot],
    base_scheme: &BarScheme,
    bar_height: i32,
    ui_scale: f64,
) {
    for pair in cells.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        if right.start != left.end {
            continue;
        }
        let bordering_separator = [left.idx, right.idx]
            .into_iter()
            .any(|idx| menu.entries.get(idx).is_some_and(|entry| entry.separator));
        if bordering_separator {
            continue;
        }
        painter.set_scheme(base_scheme.clone());
        let line_height = (bar_height - scaled_px(8, ui_scale)).max(1).min(bar_height);
        let line_y = (bar_height - line_height) / 2;
        painter.rect(
            Rect::new(left.end, line_y, scaled_px(1, ui_scale), line_height),
            false,
        );
    }
}

/// A DBus menu separator entry: a short horizontal rule, vertically centred.
fn draw_separator_entry(
    painter: &mut dyn BarPainter,
    start: i32,
    width: i32,
    base_scheme: &BarScheme,
    bar_height: i32,
    ui_scale: f64,
) {
    painter.set_scheme(base_scheme.clone());
    let y = (bar_height - scaled_px(1, ui_scale)) / 2;
    painter.rect(
        Rect::new(
            start + scaled_px(4, ui_scale),
            y,
            (width - scaled_px(8, ui_scale)).max(1),
            scaled_px(1, ui_scale),
        ),
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::paint::HOVER_INDICATOR_HEIGHT;
    use crate::systray::{MenuAction, MenuEntry};

    #[derive(Default)]
    struct RecordingPainter {
        scheme: Option<BarScheme>,
        rectangles: Vec<(Rect, Rgba)>,
        texts: Vec<(Rect, String, BarScheme)>,
    }

    impl BarPainter for RecordingPainter {
        fn text_width(&mut self, text: &str) -> i32 {
            text.chars().count() as i32 * 10
        }

        fn set_scheme(&mut self, scheme: BarScheme) {
            self.scheme = Some(scheme);
        }

        fn rect(&mut self, bounds: Rect, invert: bool) {
            let color = self
                .scheme
                .as_ref()
                .expect("drawing requires a color scheme")
                .rect_color(invert);
            self.rectangles.push((bounds, color));
        }

        fn text(
            &mut self,
            bounds: Rect,
            _lpad: i32,
            text: &str,
            _invert: bool,
            _detail_height: i32,
            _overflow: TextOverflow,
        ) -> i32 {
            let scheme = self
                .scheme
                .clone()
                .expect("drawing requires a color scheme");
            self.texts.push((bounds, text.to_string(), scheme));
            bounds.x + bounds.w
        }

        fn blit_rgba(
            &mut self,
            destination: Rect,
            source_size: crate::types::Size,
            src_rgba: &[u8],
        ) {
            assert!(
                src_rgba.len() >= (source_size.w as usize) * (source_size.h as usize) * 4,
                "blit_rgba requires enough source pixels"
            );
            self.rectangles
                .push((destination, Rgba::rgb(1.0, 1.0, 1.0)));
        }
    }

    fn scheme() -> BarScheme {
        BarScheme {
            foreground: Rgba::new(1.0, 1.0, 1.0, 1.0),
            background: Rgba::rgb(0.0, 0.0, 0.0),
            detail: Rgba::new(0.5, 0.5, 0.5, 0.5),
        }
    }

    fn entry(label: &str) -> MenuEntry {
        MenuEntry {
            label: label.to_string(),
            width: label.chars().count() as i32 * 8 + 20,
            enabled: true,
            separator: false,
            toggle: MenuToggle::None,
            action: MenuAction::Activate(0),
        }
    }

    fn separator_entry() -> MenuEntry {
        MenuEntry {
            label: String::new(),
            width: 24,
            enabled: true,
            separator: true,
            toggle: MenuToggle::None,
            action: MenuAction::Activate(0),
        }
    }

    fn cells_for(widths: &[i32]) -> Vec<SystrayHitSlot> {
        let mut x = 100;
        widths
            .iter()
            .enumerate()
            .map(|(idx, width)| {
                let cell = SystrayHitSlot {
                    idx,
                    start: x,
                    end: x + width,
                };
                x += width;
                cell
            })
            .collect()
    }

    const BAR_HEIGHT: i32 = 24;

    #[test]
    fn adjacent_entries_get_a_vertical_separator_rule() {
        let menu = MenuView {
            entries: vec![entry("Open"), entry("Quit")],
        };
        let cells = cells_for(&[68, 60]);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            None,
            1.0,
        );

        let line_height = (BAR_HEIGHT - 8).clamp(1, BAR_HEIGHT);
        let separator = (
            Rect::new(cells[0].end, (BAR_HEIGHT - line_height) / 2, 1, line_height),
            scheme().foreground,
        );
        assert!(painter.rectangles.contains(&separator));
        // Exactly one rule for two entries: no outer edges are ruled.
        assert_eq!(
            painter
                .rectangles
                .iter()
                .filter(|(rect, _)| rect.w == 1 && rect.h == line_height)
                .count(),
            1
        );
    }

    #[test]
    fn boundaries_next_to_separator_entries_are_not_doubled() {
        let menu = MenuView {
            entries: vec![entry("Open"), separator_entry(), entry("Quit")],
        };
        let cells = cells_for(&[68, 24, 60]);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            None,
            1.0,
        );

        // No vertical rules at all: both boundaries flank the separator entry.
        assert!(
            !painter
                .rectangles
                .iter()
                .any(|(rect, _)| rect.w == 1 && rect.h > 1)
        );
        // The separator entry itself renders a centred horizontal rule.
        let rule = (
            Rect::new(cells[1].start + 4, (BAR_HEIGHT - 1) / 2, 16, 1),
            scheme().foreground,
        );
        assert!(painter.rectangles.contains(&rule));
    }

    #[test]
    fn hovered_entry_gets_a_bottom_accent_without_recoloring_other_cells() {
        let menu = MenuView {
            entries: vec![entry("Open"), entry("Quit")],
        };
        let cells = cells_for(&[68, 60]);
        let hover_color = Rgba::rgb(0.2, 0.8, 1.0);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            Some(TrayMenuHover {
                entry_index: 1,
                color: hover_color,
            }),
            1.0,
        );

        let accent = (
            Rect::new(
                cells[1].start,
                BAR_HEIGHT - HOVER_INDICATOR_HEIGHT,
                cells[1].end - cells[1].start,
                HOVER_INDICATOR_HEIGHT,
            ),
            hover_color,
        );
        assert!(painter.rectangles.contains(&accent));
        assert_eq!(
            painter
                .rectangles
                .iter()
                .filter(|(_, color)| *color == hover_color)
                .count(),
            1
        );
    }

    #[test]
    fn separator_entries_never_get_a_hover_accent() {
        let menu = MenuView {
            entries: vec![separator_entry()],
        };
        let cells = cells_for(&[24]);
        let hover_color = Rgba::rgb(0.2, 0.8, 1.0);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            Some(TrayMenuHover {
                entry_index: 0,
                color: hover_color,
            }),
            1.0,
        );

        assert!(
            !painter
                .rectangles
                .iter()
                .any(|(_, color)| *color == hover_color)
        );
    }

    #[test]
    fn submenu_entries_keep_their_suffix_and_toggle_prefixes() {
        let mut checked = entry("Enabled");
        checked.toggle = MenuToggle::Check(true);
        let mut submenu = entry("Preferences");
        submenu.action = MenuAction::OpenSubmenu(7);
        let menu = MenuView {
            entries: vec![checked, submenu],
        };
        let cells = cells_for(&[76, 100]);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            None,
            1.0,
        );

        assert_eq!(painter.texts[0].1, "✓ Enabled");
        assert_eq!(painter.texts[1].1, "Preferences ›");
    }

    #[test]
    fn disabled_entry_text_draws_with_a_dimmed_scheme() {
        let mut disabled = entry("Unavailable");
        disabled.enabled = false;
        let menu = MenuView {
            entries: vec![disabled],
        };
        let cells = cells_for(&[100]);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            None,
            1.0,
        );

        let (_, _, text_scheme) = &painter.texts[0];
        assert!(text_scheme.foreground.a() < scheme().foreground.a());
        // The dimmed scheme does not leak into later drawing.
        let restored = painter.scheme.as_ref().expect("scheme restored after draw");
        assert_eq!(restored.foreground, scheme().foreground);
    }

    #[test]
    fn decorative_pixels_follow_monitor_scale() {
        let menu = MenuView {
            entries: vec![entry("Open"), entry("Quit")],
        };
        let cells = cells_for(&[68, 60]);
        let mut painter = RecordingPainter::default();

        draw_menu(
            &mut painter,
            &menu,
            &cells,
            &scheme(),
            BAR_HEIGHT,
            None,
            2.0,
        );

        // The vertical rule doubles in width and inset from the bar edges.
        let line_height = (BAR_HEIGHT - scaled_px(8, 2.0)).clamp(1, BAR_HEIGHT);
        let separator = (
            Rect::new(
                cells[0].end,
                (BAR_HEIGHT - line_height) / 2,
                scaled_px(1, 2.0),
                line_height,
            ),
            scheme().foreground,
        );
        assert!(painter.rectangles.contains(&separator));
    }
}
