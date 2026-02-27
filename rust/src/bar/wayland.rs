//! Wayland bar rendering using Smithay's GlesRenderer.
//!
//! This module provides GPU-accelerated bar rendering for the Wayland backend.
//! It uses Smithay's SolidColorRenderElement for rectangles and integrates
//! with the compositor's render loop.

use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::utils::{Logical, Point, Scale, Size};

use crate::globals::get_globals;
use crate::types::Monitor;

/// Default font size in pixels.
const DEFAULT_FONT_SIZE: f32 = 12.0;
/// Default bar height if not configured.
const DEFAULT_BAR_HEIGHT: i32 = 24;
/// Padding around text elements.
const TEXT_PADDING: i32 = 6;

/// A bar element ready for rendering.
#[derive(Debug, Clone)]
pub enum BarRenderElement {
    /// A solid colored rectangle.
    Rect {
        /// Position in logical coordinates.
        loc: Point<i32, Logical>,
        /// Size in logical coordinates.
        size: Size<i32, Logical>,
        /// RGBA color (0.0 - 1.0).
        color: [f32; 4],
    },
    /// Text element (rendered as colored rectangle for now).
    Text {
        /// Position in logical coordinates.
        loc: Point<i32, Logical>,
        /// Size in logical coordinates.
        size: Size<i32, Logical>,
        /// RGBA color.
        color: [f32; 4],
    },
}

/// Collection of render elements for the bar.
#[derive(Debug, Default, Clone)]
pub struct BarRenderElements {
    elements: Vec<BarRenderElement>,
}

impl BarRenderElements {
    /// Create a new empty collection.
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }

    /// Add a rectangle element.
    pub fn add_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: [f32; 4]) {
        if width <= 0 || height <= 0 {
            return;
        }
        self.elements.push(BarRenderElement::Rect {
            loc: Point::from((x, y)),
            size: Size::from((width, height)),
            color,
        });
    }

    /// Add a text placeholder element.
    pub fn add_text(&mut self, x: i32, y: i32, width: i32, height: i32, color: [f32; 4]) {
        if width <= 0 || height <= 0 {
            return;
        }
        self.elements.push(BarRenderElement::Text {
            loc: Point::from((x, y)),
            size: Size::from((width, height)),
            color,
        });
    }

    /// Get all elements.
    pub fn elements(&self) -> &[BarRenderElement] {
        &self.elements
    }

    /// Check if there are no elements.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Convert to Smithay SolidColorRenderElements.
    pub fn to_solid_elements(&self, scale: Scale<f64>) -> Vec<SolidColorRenderElement> {
        use smithay::utils::Physical;

        self.elements
            .iter()
            .map(|elem| {
                let (loc, size, color) = match elem {
                    BarRenderElement::Rect { loc, size, color } => (*loc, *size, *color),
                    BarRenderElement::Text { loc, size, color } => (*loc, *size, *color),
                };

                // Scale the logical coordinates for rendering
                let scaled_loc: Point<i32, Physical> = Point::from((
                    (loc.x as f64 * scale.x).round() as i32,
                    (loc.y as f64 * scale.y).round() as i32,
                ));
                let scaled_size = size.to_f64().upscale(scale).to_i32_round();

                // Create buffer with scaled size
                let buffer = SolidColorBuffer::new(scaled_size, color);
                SolidColorRenderElement::from_buffer(
                    &buffer,
                    scaled_loc,
                    scale,
                    1.0,
                    Kind::Unspecified,
                )
            })
            .collect()
    }
}

/// Bar renderer for Wayland backend.
pub struct BarRenderer {
    scale_factor: f32,
    bar_height: i32,
}

impl BarRenderer {
    /// Create a new bar renderer.
    pub fn new() -> Self {
        Self {
            scale_factor: 1.0,
            bar_height: DEFAULT_BAR_HEIGHT,
        }
    }

    /// Set the bar height.
    pub fn set_bar_height(&mut self, height: i32) {
        self.bar_height = height;
    }

    /// Render the bar for a monitor and return the render elements.
    pub fn render_bar(
        &mut self,
        monitor: &Monitor,
        x: i32,
        y: i32,
        width: i32,
    ) -> BarRenderElements {
        let g = get_globals();
        let bh = g.cfg.bar_height.max(DEFAULT_BAR_HEIGHT);
        self.bar_height = bh;

        let mut elements = BarRenderElements::new();

        let (bg_color, fg_color) = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
            .unwrap_or(([0.07, 0.07, 0.07, 1.0], [0.9, 0.9, 0.9, 1.0]));

        // Draw bar background
        elements.add_rect(x, y, width, bh, bg_color);

        let startmenu_size = g.cfg.startmenusize;
        let mut current_x = x + startmenu_size;

        // Draw start menu icon
        self.draw_startmenu_icon(&mut elements, x, y, startmenu_size, bh, fg_color, bg_color);

        // Draw tags
        current_x = self.draw_tags(&mut elements, monitor, current_x, y, bh);

        // Draw layout indicator
        current_x = self.draw_layout(&mut elements, monitor, current_x, y, bh, fg_color, bg_color);

        // Draw shutdown button if no client selected
        if monitor.sel.is_none() {
            current_x =
                self.draw_shutdown_button(&mut elements, current_x, y, bh, fg_color, bg_color);
        }

        // Draw window titles
        let status_width = if g.selmon_id() == monitor.id() {
            self.measure_status_text()
        } else {
            0
        };

        let title_width = (x + width - current_x - status_width).max(0);
        if title_width > 0 {
            self.draw_window_titles(&mut elements, monitor, current_x, y, title_width, bh);
        }

        // Draw status text
        if g.selmon_id() == monitor.id() && status_width > 0 {
            self.draw_status_text(
                &mut elements,
                x + width - status_width,
                y,
                status_width,
                bh,
                fg_color,
                bg_color,
            );
        }

        elements
    }

    fn draw_startmenu_icon(
        &self,
        elements: &mut BarRenderElements,
        x: i32,
        y: i32,
        size: i32,
        bh: i32,
        fg: [f32; 4],
        bg: [f32; 4],
    ) {
        let g = get_globals();
        let is_inverted = g
            .selmon()
            .is_some_and(|mon| mon.gesture == crate::types::Gesture::StartMenu);

        let (icon_bg, icon_fg) = if is_inverted { (fg, bg) } else { (bg, fg) };

        elements.add_rect(x, y, size, bh, icon_bg);

        let icon_size = 14i32;
        let icon_offset = (bh - icon_size) / 2;
        let inner_size = 6i32;

        elements.add_rect(x + 5, y + icon_offset, icon_size, icon_size, icon_fg);
        elements.add_rect(x + 9, y + icon_offset + 4, inner_size, inner_size, icon_bg);
        elements.add_rect(
            x + 19,
            y + icon_offset + icon_size,
            inner_size,
            inner_size,
            icon_fg,
        );
    }

    fn draw_tags(
        &self,
        elements: &mut BarRenderElements,
        monitor: &Monitor,
        x: i32,
        y: i32,
        bh: i32,
    ) -> i32 {
        let g = get_globals();
        let mut current_x = x;

        let occupied_tags: u32 = g
            .clients
            .values()
            .filter(|c| c.mon_id == Some(monitor.id()))
            .map(|c| c.tags)
            .fold(0, |acc, tags| acc | tags);

        let selmon_gesture = g.selmon().map(|s| s.gesture).unwrap_or_default();

        for (i, tag) in monitor.tags.iter().enumerate() {
            let is_occupied = occupied_tags & (1 << i) != 0;
            let is_selected = monitor.tagset[monitor.seltags as usize] & (1 << i) != 0;
            let is_hover = selmon_gesture == crate::types::Gesture::Tag(i);

            let (bg_color, fg_color) = self.get_tag_colors(i, is_occupied, is_selected, is_hover);
            let tag_width = 40;

            elements.add_rect(current_x, y, tag_width, bh, bg_color);

            let text_width = tag.name.len() as i32 * 8;
            if text_width < tag_width - TEXT_PADDING * 2 {
                let text_x = current_x + (tag_width - text_width) / 2;
                let text_y = y + (bh - 12) / 2;
                elements.add_text(text_x, text_y, text_width, 12, fg_color);
            }

            if is_hover {
                let detail_color = g
                    .cfg
                    .statusscheme
                    .as_ref()
                    .map(|s| color_to_rgba(&s.detail))
                    .unwrap_or([0.0, 0.33, 0.47, 1.0]);
                elements.add_rect(current_x, y + bh - 4, tag_width, 4, detail_color);
            }

            current_x += tag_width;
        }

        current_x
    }

    fn get_tag_colors(
        &self,
        _idx: usize,
        is_occupied: bool,
        is_selected: bool,
        is_hover: bool,
    ) -> ([f32; 4], [f32; 4]) {
        let g = get_globals();
        let schemes = if is_hover {
            &g.tags.schemes.hover
        } else {
            &g.tags.schemes.no_hover
        };

        let default_bg = [0.07, 0.07, 0.07, 1.0];
        let default_fg = [0.9, 0.9, 0.9, 1.0];

        if schemes.is_empty() {
            return (default_bg, default_fg);
        }

        use crate::config::SchemeTag;
        let scheme_idx = if is_occupied {
            if is_selected {
                SchemeTag::Focus as usize
            } else {
                SchemeTag::NoFocus as usize
            }
        } else if is_selected {
            SchemeTag::Empty as usize
        } else {
            SchemeTag::Inactive as usize
        };

        schemes
            .get(scheme_idx)
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
            .unwrap_or((default_bg, default_fg))
    }

    fn draw_layout(
        &self,
        elements: &mut BarRenderElements,
        monitor: &Monitor,
        x: i32,
        y: i32,
        bh: i32,
        fg: [f32; 4],
        bg: [f32; 4],
    ) -> i32 {
        let layout_symbol = monitor.layout_symbol();
        let width = 60i32;

        elements.add_rect(x, y, width, bh, bg);

        let text_width = layout_symbol.len() as i32 * 6;
        if text_width < width - TEXT_PADDING * 2 {
            let text_x = x + (width - text_width) / 2;
            let text_y = y + (bh - 12) / 2;
            elements.add_text(text_x, text_y, text_width, 12, fg);
        }

        x + width
    }

    fn draw_shutdown_button(
        &self,
        elements: &mut BarRenderElements,
        x: i32,
        y: i32,
        bh: i32,
        fg: [f32; 4],
        bg: [f32; 4],
    ) -> i32 {
        elements.add_rect(x, y, bh, bh, bg);

        let icon_size = bh * 5 / 8;
        let icon_x = x + (bh - icon_size) / 2;
        let icon_y = y + (bh - icon_size) / 2;
        let stroke = (icon_size / 6).max(2);
        let gap = stroke;

        let stem_w = stroke;
        let stem_h = icon_size / 2;
        let stem_x = icon_x + (icon_size - stem_w) / 2;
        elements.add_rect(stem_x, icon_y, stem_w, stem_h, fg);

        let arc_y = icon_y + gap + stroke;
        let arc_h = icon_size - gap - stroke;

        elements.add_rect(icon_x, arc_y, stroke, arc_h, fg);
        elements.add_rect(icon_x + icon_size - stroke, arc_y, stroke, arc_h, fg);

        let bot_x = icon_x + stroke;
        let bot_y = icon_y + icon_size - stroke;
        let bot_w = (icon_size - stroke * 2).max(0);
        elements.add_rect(bot_x, bot_y, bot_w, stroke, fg);

        x + bh
    }

    fn draw_window_titles(
        &self,
        elements: &mut BarRenderElements,
        monitor: &Monitor,
        x: i32,
        y: i32,
        width: i32,
        bh: i32,
    ) {
        let g = get_globals();
        let selected = monitor.selected_tags();

        let visible_clients: Vec<_> = monitor
            .iter_clients(&g.clients)
            .filter(|(_, c)| c.is_visible_on_tags(selected))
            .map(|(win, _)| win)
            .collect();

        let n = visible_clients.len() as i32;

        if n > 0 {
            let each_width = width / n;
            let remainder = width % n;
            let mut current_x = x;

            for (i, &win) in visible_clients.iter().enumerate() {
                let Some(client) = g.clients.get(&win) else {
                    continue;
                };

                let this_width = if i < remainder as usize {
                    each_width + 1
                } else {
                    each_width
                };
                let is_selected = monitor.sel == Some(win);
                let (bg, fg) = self.get_window_colors(client, is_selected);

                elements.add_rect(current_x, y, this_width, bh, bg);

                let name_width = client.name.len().min(20) as i32 * 6;
                if name_width < this_width - TEXT_PADDING * 2 {
                    let text_x = current_x + TEXT_PADDING;
                    let text_y = y + (bh - 12) / 2;
                    elements.add_text(text_x, text_y, name_width, 12, fg);
                }

                if is_selected {
                    self.draw_close_button(elements, current_x, y, bh);
                }

                current_x += this_width;
            }
        } else {
            let (bg, fg) = g
                .cfg
                .statusscheme
                .as_ref()
                .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
                .unwrap_or(([0.07, 0.07, 0.07, 1.0], [0.9, 0.9, 0.9, 1.0]));

            elements.add_rect(x, y, width, bh, bg);

            let help_text = "Press space to launch";
            let text_width = help_text.len() as i32 * 6;
            if text_width < width - TEXT_PADDING * 2 {
                let text_x = x + (width - text_width) / 2;
                let text_y = y + (bh - 12) / 2;
                elements.add_text(text_x, text_y, text_width, 12, fg);
            }
        }
    }

    fn get_window_colors(
        &self,
        client: &crate::types::Client,
        is_selected: bool,
    ) -> ([f32; 4], [f32; 4]) {
        let g = get_globals();
        let schemes = &g.cfg.windowschemes.no_hover;

        let default_bg = [0.07, 0.07, 0.07, 1.0];
        let default_fg = [0.9, 0.9, 0.9, 1.0];

        if schemes.is_empty() {
            return (default_bg, default_fg);
        }

        use crate::config::SchemeWin;
        let scheme_idx = if is_selected {
            if client.issticky {
                SchemeWin::StickyFocus as usize
            } else {
                SchemeWin::Focus as usize
            }
        } else if client.issticky {
            SchemeWin::Sticky as usize
        } else if client.is_hidden {
            SchemeWin::Minimized as usize
        } else {
            SchemeWin::Normal as usize
        };

        schemes
            .get(scheme_idx)
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
            .unwrap_or((default_bg, default_fg))
    }

    fn draw_close_button(&self, elements: &mut BarRenderElements, x: i32, y: i32, bh: i32) {
        let g = get_globals();
        let button_size = 16i32;
        let button_x = x + bh / 6;
        let button_y = y + (bh - button_size) / 2;

        let (bg, detail) = g
            .cfg
            .closebuttonschemes
            .no_hover
            .first()
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.detail)))
            .unwrap_or(([0.8, 0.2, 0.2, 1.0], [0.6, 0.1, 0.1, 1.0]));

        elements.add_rect(button_x, button_y, button_size, button_size, bg);
        elements.add_rect(button_x, button_y + button_size - 4, button_size, 4, detail);
    }

    fn measure_status_text(&self) -> i32 {
        let g = get_globals();
        if g.status_text.is_empty() {
            return 0;
        }
        g.status_text.len() as i32 * 6 + TEXT_PADDING * 2
    }

    fn draw_status_text(
        &self,
        elements: &mut BarRenderElements,
        x: i32,
        y: i32,
        width: i32,
        bh: i32,
        fg: [f32; 4],
        bg: [f32; 4],
    ) {
        let g = get_globals();
        if g.status_text.is_empty() {
            return;
        }

        elements.add_rect(x, y, width, bh, bg);

        let text_width = g.status_text.len().min(50) as i32 * 6;
        let actual_width = text_width.min(width - TEXT_PADDING * 2);
        if actual_width > 0 {
            let text_x = x + TEXT_PADDING;
            let text_y = y + (bh - 12) / 2;
            elements.add_text(text_x, text_y, actual_width, 12, fg);
        }
    }
}

impl Default for BarRenderer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn draw_bar_wayland(_ctx: &mut crate::contexts::WmCtx, _mon_idx: usize) {}
pub fn draw_bars_wayland(_ctx: &mut crate::contexts::WmCtx) {}
pub fn reset_bar_wayland(ctx: &mut crate::contexts::WmCtx) {
    let should_reset = ctx
        .g
        .selmon()
        .is_some_and(|selmon| selmon.gesture != crate::types::Gesture::None);
    if should_reset {
        if let Some(selmon) = ctx.g.selmon_mut() {
            selmon.gesture = crate::types::Gesture::None;
        }
    }
}
pub fn should_draw_bar_wayland(ctx: &crate::contexts::WmCtx) -> bool {
    ctx.g.cfg.showbar
}

/// Render the bar for all monitors to Smithay render elements.
pub fn render_bar_elements(
    bar_renderer: &mut BarRenderer,
    ctx: &crate::contexts::WmCtx,
    scale: Scale<f64>,
) -> Vec<SolidColorRenderElement> {
    let mut all_elements = Vec::new();
    let bh = ctx.g.cfg.bar_height.max(DEFAULT_BAR_HEIGHT);
    bar_renderer.set_bar_height(bh);

    for (_mon_idx, monitor) in ctx.g.monitors_iter() {
        if !monitor.shows_bar() {
            continue;
        }

        let bar_x = monitor.work_rect.x;
        let bar_y = monitor.work_rect.y;
        let bar_width = monitor.work_rect.w;

        if bar_width <= 0 || bh <= 0 {
            continue;
        }

        let bar_elements = bar_renderer.render_bar(monitor, bar_x, bar_y, bar_width);
        let solid_elements = bar_elements.to_solid_elements(scale);
        all_elements.extend(solid_elements);
    }

    all_elements
}

fn color_to_rgba(color: &crate::drw::Color) -> [f32; 4] {
    let r = color.color.color.red as f32 / 65535.0;
    let g = color.color.color.green as f32 / 65535.0;
    let b = color.color.color.blue as f32 / 65535.0;
    let a = color.color.color.alpha as f32 / 65535.0;
    [r, g, b, a]
}
