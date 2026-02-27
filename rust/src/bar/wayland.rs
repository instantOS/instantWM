//! Wayland bar rendering using Smithay's GlesRenderer and cosmic-text.
//!
//! This module provides GPU-accelerated bar rendering for the Wayland backend.
//! It uses cosmic-text for text layout and rendering with swash integration.

use cosmic_text::{Buffer, BufferRef, FontSystem, Metrics, SwashCache};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Frame, Renderer, Texture};
use smithay::utils::{Physical, Point, Rectangle, Size, Transform};

use crate::contexts::WmCtx;
use crate::globals::get_globals;
use crate::types::{ColorScheme, Monitor, WindowId};

/// Default font size in pixels.
const DEFAULT_FONT_SIZE: f32 = 12.0;
/// Default bar height if not configured.
const DEFAULT_BAR_HEIGHT: i32 = 24;
/// Padding around text elements.
const TEXT_PADDING: i32 = 6;

/// GPU-accelerated bar renderer for Wayland backend.
///
/// Holds the font system, glyph cache, and font metrics for rendering
/// the status bar using Smithay's GlesRenderer.
pub struct BarRenderer {
    /// Font system for loading and managing fonts.
    font_system: FontSystem,
    /// Glyph cache for efficient text rendering.
    swash_cache: SwashCache,
    /// Font metrics for text measurement.
    metrics: Metrics,
    /// Scale factor for DPI-aware rendering.
    scale_factor: f32,
    /// Bar height in pixels.
    bar_height: i32,
}

impl BarRenderer {
    /// Create a new bar renderer with default system fonts.
    ///
    /// Initializes the font system with default fonts and sets up
    /// the swash cache for efficient glyph rendering.
    pub fn new() -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let metrics = Metrics::new(DEFAULT_FONT_SIZE, DEFAULT_FONT_SIZE * 1.2);

        Self {
            font_system,
            swash_cache,
            metrics,
            scale_factor: 1.0,
            bar_height: DEFAULT_BAR_HEIGHT,
        }
    }

    /// Create a new bar renderer with a specific scale factor.
    ///
    /// The scale factor is used for DPI-aware rendering on HiDPI displays.
    pub fn with_scale_factor(scale_factor: f32) -> Self {
        let mut renderer = Self::new();
        renderer.scale_factor = scale_factor;
        renderer.metrics = Metrics::new(
            DEFAULT_FONT_SIZE * scale_factor,
            DEFAULT_FONT_SIZE * 1.2 * scale_factor,
        );
        renderer
    }

    /// Set the bar height.
    pub fn set_bar_height(&mut self, height: i32) {
        self.bar_height = height;
    }

    /// Update the scale factor (e.g., when moving between monitors with different DPI).
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
        self.metrics = Metrics::new(
            DEFAULT_FONT_SIZE * scale_factor,
            DEFAULT_FONT_SIZE * 1.2 * scale_factor,
        );
    }

    /// Get the current scale factor.
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Get the current bar height.
    pub fn bar_height(&self) -> i32 {
        self.bar_height
    }

    /// Render the entire bar for a monitor.
    ///
    /// This is the main entry point for bar rendering. It draws all bar elements:
    /// - Background
    /// - Start menu icon
    /// - Tag indicators
    /// - Layout indicator
    /// - Shutdown button (when no client selected)
    /// - Window titles
    /// - Status text
    ///
    /// # Arguments
    ///
    /// * `renderer` - The GlesRenderer to render to
    /// * `frame` - The current frame being rendered
    /// * `monitor` - The monitor to render the bar for
    /// * `x` - X position of the bar
    /// * `y` - Y position of the bar
    /// * `width` - Width of the bar
    pub fn render_bar(
        &mut self,
        renderer: &mut GlesRenderer,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        monitor: &Monitor,
        x: i32,
        y: i32,
        width: i32,
    ) {
        let g = get_globals();
        let bh = g.cfg.bar_height.max(DEFAULT_BAR_HEIGHT);
        self.bar_height = bh;

        // Get the color scheme for the bar background
        let bg_color = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| color_to_rgba(&s.bg))
            .unwrap_or([0.07, 0.07, 0.07, 1.0]); // Default dark background

        // Draw bar background
        self.draw_rect(frame, x, y, width, bh, bg_color);

        // Calculate positions for bar elements
        let startmenu_size = g.cfg.startmenusize;
        let mut current_x = x + startmenu_size;

        // Draw start menu icon
        self.draw_startmenu_icon(frame, x, y, startmenu_size, bh);

        // Draw tag indicators
        current_x = self.draw_tag_indicators(frame, monitor, current_x, y, bh);

        // Draw layout indicator
        current_x = self.draw_layout_indicator(frame, monitor, current_x, y, bh);

        // Draw shutdown button if no client selected
        if monitor.sel.is_none() {
            current_x = self.draw_shutdown_button(frame, current_x, y, bh);
        }

        // Draw window titles in the remaining space
        let status_width = if g.selmon_id() == monitor.id() {
            self.measure_status_text()
        } else {
            0
        };

        let title_width = (x + width - current_x - status_width).max(0);
        if title_width > 0 {
            self.draw_window_titles(frame, monitor, current_x, y, title_width, bh);
        }

        // Draw status text on the selected monitor
        if g.selmon_id() == monitor.id() {
            self.draw_status_text(frame, x + width - status_width, y, status_width, bh);
        }
    }

    /// Draw a filled rectangle.
    ///
    /// # Arguments
    ///
    /// * `frame` - The frame to draw to
    /// * `x` - X position
    /// * `y` - Y position
    /// * `width` - Rectangle width
    /// * `height` - Rectangle height
    /// * `color` - RGBA color as [r, g, b, a] with values 0.0-1.0
    pub fn draw_rect(
        &self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        color: [f32; 4],
    ) {
        if width <= 0 || height <= 0 {
            return;
        }

        let rect = Rectangle::new(Point::from((x, y)), Size::from((width, height)));

        // Use the frame's render solid method
        frame.render_solid(rect, color, Transform::Normal);
    }

    /// Draw text at a position.
    ///
    /// # Arguments
    ///
    /// * `frame` - The frame to draw to
    /// * `x` - X position
    /// * `y` - Y position
    /// * `text` - The text to draw
    /// * `color` - RGBA color as [r, g, b, a]
    ///
    /// Returns the width of the rendered text.
    pub fn draw_text(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        x: i32,
        y: i32,
        text: &str,
        color: [f32; 4],
    ) -> i32 {
        let width = self.text_width(text);
        if width <= 0 || text.is_empty() {
            return 0;
        }

        // For now, draw a placeholder rectangle to represent text
        // Full cosmic-text integration would involve:
        // 1. Creating a Buffer with the text
        // 2. Shaping the text
        // 3. Rendering glyphs to a texture
        // 4. Drawing the texture
        let text_height = (self.metrics.font_size as i32).min(self.bar_height - 4);
        let text_y = y + (self.bar_height - text_height) / 2;

        // Draw text background (placeholder for actual text rendering)
        self.draw_rect(frame, x, text_y, width, text_height, color);

        width
    }

    /// Measure the width of text without rendering it.
    ///
    /// # Arguments
    ///
    /// * `text` - The text to measure
    ///
    /// Returns the width in pixels.
    pub fn text_width(&mut self, text: &str) -> i32 {
        if text.is_empty() {
            return 0;
        }

        // Create a temporary buffer for text measurement
        let mut buffer = Buffer::new(&mut self.font_system, self.metrics);
        buffer.set_text(
            &mut self.font_system,
            text,
            cosmic_text::Attrs::new(),
            cosmic_text::Shaping::Advanced,
        );

        // Get the layout width
        let layout = buffer.line_layout(&mut self.font_system, 0);
        match layout {
            Some(layout) if !layout.is_empty() => {
                let width: f32 = layout.iter().map(|span| span.w).sum();
                (width / self.scale_factor).ceil() as i32 + TEXT_PADDING * 2
            }
            _ => (text.len() as f32 * self.metrics.font_size * 0.6) as i32 + TEXT_PADDING * 2,
        }
    }

    /// Draw the start menu icon.
    fn draw_startmenu_icon(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        x: i32,
        y: i32,
        size: i32,
        bh: i32,
    ) -> i32 {
        let g = get_globals();
        let is_inverted = g
            .selmon()
            .is_some_and(|mon| mon.gesture == crate::types::Gesture::StartMenu);

        let fg_color = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| color_to_rgba(&s.fg))
            .unwrap_or([0.9, 0.9, 0.9, 1.0]);

        let bg_color = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| color_to_rgba(&s.bg))
            .unwrap_or([0.07, 0.07, 0.07, 1.0]);

        // Draw background
        self.draw_rect(
            frame,
            x,
            y,
            size,
            bh,
            if is_inverted { fg_color } else { bg_color },
        );

        // Draw simple icon pattern (two nested rectangles)
        let icon_size = 14i32;
        let icon_offset = (bh - icon_size) / 2;
        let inner_size = 6i32;

        // Outer rectangle
        let outer_color = if is_inverted { bg_color } else { fg_color };
        self.draw_rect(
            frame,
            x + 5,
            y + icon_offset,
            icon_size,
            icon_size,
            outer_color,
        );

        // Inner rectangle
        let inner_color = if is_inverted { fg_color } else { bg_color };
        self.draw_rect(
            frame,
            x + 9,
            y + icon_offset + 4,
            inner_size,
            inner_size,
            inner_color,
        );

        // Small detail rectangle
        self.draw_rect(
            frame,
            x + 19,
            y + icon_offset + icon_size,
            inner_size,
            inner_size,
            outer_color,
        );

        size
    }

    /// Draw tag indicators.
    fn draw_tag_indicators(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        monitor: &Monitor,
        x: i32,
        y: i32,
        bh: i32,
    ) -> i32 {
        let g = get_globals();
        let mut current_x = x;

        // Calculate occupied tags
        let occupied_tags: u32 = g
            .clients
            .values()
            .filter(|c| c.mon == monitor.num)
            .map(|c| c.tags)
            .fold(0, |acc, tags| acc | tags);

        let selmon_gesture = g.selmon().map(|s| s.gesture).unwrap_or_default();

        for (i, tag) in monitor.tags.iter().enumerate() {
            let tag_index = i as u32;
            let is_occupied = occupied_tags & (1 << tag_index) != 0;
            let is_selected = monitor.tagset[monitor.seltags as usize] & (1 << tag_index) != 0;
            let is_hover = selmon_gesture == crate::types::Gesture::Tag(i);

            // Get color scheme for this tag
            let (bg_color, fg_color) =
                self.get_tag_colors(monitor, tag_index, is_occupied, is_selected, is_hover);

            // Calculate tag width based on text
            let tag_width = self.text_width(&tag.name).max(30);

            // Draw tag background
            self.draw_rect(frame, current_x, y, tag_width, bh, bg_color);

            // Draw tag text
            self.draw_text(frame, current_x, y, &tag.name, fg_color);

            // Draw detail bar at bottom for hover
            if is_hover {
                let detail_height = 8;
                let detail_color = g
                    .cfg
                    .statusscheme
                    .as_ref()
                    .map(|s| color_to_rgba(&s.detail))
                    .unwrap_or([0.0, 0.33, 0.47, 1.0]);
                self.draw_rect(
                    frame,
                    current_x,
                    y + bh - detail_height,
                    tag_width,
                    detail_height,
                    detail_color,
                );
            }

            current_x += tag_width;
        }

        current_x
    }

    /// Get colors for a tag based on its state.
    fn get_tag_colors(
        &self,
        monitor: &Monitor,
        tag_index: u32,
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

        // Default colors
        let default_bg = [0.07, 0.07, 0.07, 1.0];
        let default_fg = [0.9, 0.9, 0.9, 1.0];

        if schemes.is_empty() {
            return (default_bg, default_fg);
        }

        let scheme_idx = if is_occupied {
            if is_selected {
                crate::config::SchemeTag::Focus as usize
            } else {
                crate::config::SchemeTag::NoFocus as usize
            }
        } else if is_selected {
            crate::config::SchemeTag::Empty as usize
        } else {
            crate::config::SchemeTag::Inactive as usize
        };

        let scheme = schemes.get(scheme_idx);
        match scheme {
            Some(s) => (color_to_rgba(&s.bg), color_to_rgba(&s.fg)),
            None => (default_bg, default_fg),
        }
    }

    /// Draw the layout indicator.
    fn draw_layout_indicator(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        monitor: &Monitor,
        x: i32,
        y: i32,
        bh: i32,
    ) -> i32 {
        let g = get_globals();
        let layout_symbol = monitor.layout_symbol();
        let text_width = self.text_width(&layout_symbol);
        let width = text_width + g.cfg.horizontal_padding * 2;

        let (bg_color, fg_color) = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
            .unwrap_or(([0.07, 0.07, 0.07, 1.0], [0.9, 0.9, 0.9, 1.0]));

        // Draw background
        self.draw_rect(frame, x, y, width, bh, bg_color);

        // Draw layout symbol
        let text_x = x + (width - text_width) / 2;
        self.draw_text(frame, text_x, y, &layout_symbol, fg_color);

        x + width
    }

    /// Draw the shutdown button.
    fn draw_shutdown_button(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        x: i32,
        y: i32,
        bh: i32,
    ) -> i32 {
        let g = get_globals();

        let (bg_color, fg_color) = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
            .unwrap_or(([0.07, 0.07, 0.07, 1.0], [0.9, 0.9, 0.9, 1.0]));

        // Draw button background
        self.draw_rect(frame, x, y, bh, bh, bg_color);

        // Draw power icon using simple rectangles
        let icon_size = bh * 5 / 8;
        let icon_x = x + (bh - icon_size) / 2;
        let icon_y = y + (bh - icon_size) / 2;
        let stroke = (icon_size / 6).max(2);
        let gap = stroke;

        // Stem
        let stem_w = stroke;
        let stem_h = icon_size / 2;
        let stem_x = icon_x + (icon_size - stem_w) / 2;
        let stem_y = icon_y;
        self.draw_rect(frame, stem_x, stem_y, stem_w, stem_h, fg_color);

        // Arc approximation - left side
        let arc_x = icon_x;
        let arc_y = icon_y + gap + stroke;
        let arc_h = icon_size - gap - stroke;
        self.draw_rect(frame, arc_x, arc_y, stroke, arc_h, fg_color);

        // Arc approximation - right side
        self.draw_rect(
            frame,
            icon_x + icon_size - stroke,
            arc_y,
            stroke,
            arc_h,
            fg_color,
        );

        // Arc approximation - bottom
        let bot_x = icon_x + stroke;
        let bot_y = icon_y + icon_size - stroke;
        let bot_w = (icon_size - stroke * 2).max(0);
        self.draw_rect(frame, bot_x, bot_y, bot_w, stroke, fg_color);

        x + bh
    }

    /// Draw window titles.
    fn draw_window_titles(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        monitor: &Monitor,
        x: i32,
        y: i32,
        width: i32,
        bh: i32,
    ) {
        let g = get_globals();
        let selected = monitor.selected_tags();

        // Count visible clients
        let visible_clients: Vec<_> = monitor
            .iter_clients(&g.clients)
            .filter(|(_, c)| c.is_visible_on_tags(selected))
            .map(|(win, _)| win)
            .collect();

        let n = visible_clients.len() as i32;

        if n > 0 {
            // Divide width among clients
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
                let (bg_color, fg_color) = self.get_window_colors(client, is_selected);

                // Draw title background
                self.draw_rect(frame, current_x, y, this_width, bh, bg_color);

                // Draw client name
                let name = &client.name;
                let text_width = self.text_width(name);
                let avail_width = this_width - TEXT_PADDING * 2;

                if text_width < avail_width {
                    // Center text
                    let text_x = current_x + (this_width - text_width) / 2;
                    self.draw_text(frame, text_x, y, name, fg_color);
                } else {
                    // Left-align with padding
                    let text_x = current_x + TEXT_PADDING;
                    self.draw_text(frame, text_x, y, name, fg_color);
                }

                // Draw close button for selected window
                if is_selected {
                    self.draw_close_button(frame, current_x, y, bh);
                }

                current_x += this_width;
            }
        } else {
            // No clients - draw empty area with help text
            let (bg_color, fg_color) = g
                .cfg
                .statusscheme
                .as_ref()
                .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
                .unwrap_or(([0.07, 0.07, 0.07, 1.0], [0.9, 0.9, 0.9, 1.0]));

            self.draw_rect(frame, x, y, width, bh, bg_color);

            // Show help text if no clients
            let help_text = "Press space to launch an application";
            let text_width = self.text_width(help_text);
            let avail = width - bh;
            let title_width = text_width.min(avail);

            if title_width > 0 {
                let text_x = x + bh + (avail - title_width + 1) / 2;
                self.draw_text(frame, text_x, y, help_text, fg_color);
            }
        }
    }

    /// Get colors for a window based on its state.
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

        let scheme_idx = if is_selected {
            if client.issticky {
                crate::config::SchemeWin::StickyFocus as usize
            } else {
                crate::config::SchemeWin::Focus as usize
            }
        } else if client.issticky {
            crate::config::SchemeWin::Sticky as usize
        } else if client.is_hidden {
            crate::config::SchemeWin::Minimized as usize
        } else {
            crate::config::SchemeWin::Normal as usize
        };

        let scheme = schemes.get(scheme_idx);
        match scheme {
            Some(s) => (color_to_rgba(&s.bg), color_to_rgba(&s.fg)),
            None => (default_bg, default_fg),
        }
    }

    /// Draw close button.
    fn draw_close_button(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        x: i32,
        y: i32,
        bh: i32,
    ) {
        let g = get_globals();
        let button_width = 16;
        let button_height = 16;
        let button_x = x + bh / 6;
        let button_y = (bh - button_height) / 2;

        let (bg_color, detail_color) = g
            .cfg
            .closebuttonschemes
            .no_hover
            .first()
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.detail)))
            .unwrap_or(([0.8, 0.2, 0.2, 1.0], [0.6, 0.1, 0.1, 1.0]));

        // Draw button background
        self.draw_rect(
            frame,
            button_x,
            y + button_y,
            button_width,
            button_height,
            bg_color,
        );

        // Draw detail bar at bottom
        let detail_height = 4;
        self.draw_rect(
            frame,
            button_x,
            y + button_y + button_height - detail_height,
            button_width,
            detail_height,
            detail_color,
        );
    }

    /// Measure the width of the status text.
    fn measure_status_text(&mut self) -> i32 {
        let g = get_globals();
        if g.status_text.is_empty() {
            return 0;
        }
        self.text_width(&g.status_text)
    }

    /// Draw status text.
    fn draw_status_text(
        &mut self,
        frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
        x: i32,
        y: i32,
        width: i32,
        bh: i32,
    ) {
        let g = get_globals();
        if g.status_text.is_empty() {
            return;
        }

        let (bg_color, fg_color) = g
            .cfg
            .statusscheme
            .as_ref()
            .map(|s| (color_to_rgba(&s.bg), color_to_rgba(&s.fg)))
            .unwrap_or(([0.07, 0.07, 0.07, 1.0], [0.9, 0.9, 0.9, 1.0]));

        // Draw status background
        self.draw_rect(frame, x, y, width, bh, bg_color);

        // Draw status text
        self.draw_text(frame, x + TEXT_PADDING, y, &g.status_text, fg_color);
    }
}

impl Default for BarRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Render bar elements to a Smithay GlesFrame.
///
/// This is a helper function that can be called from the Wayland backend
/// when rendering a monitor's output.
///
/// # Arguments
///
/// * `renderer` - The GlesRenderer
/// * `frame` - The current frame being rendered
/// * `monitor` - The monitor to render the bar for
/// * `bar_area` - The rectangle where the bar should be rendered
pub fn render_bar_to_frame(
    renderer: &mut GlesRenderer,
    frame: &mut <GlesRenderer as Renderer>::Frame<'_>,
    bar_renderer: &mut BarRenderer,
    monitor: &Monitor,
    bar_area: Rectangle<i32, Physical>,
) {
    bar_renderer.render_bar(
        renderer,
        frame,
        monitor,
        bar_area.loc.x,
        bar_area.loc.y,
        bar_area.size.w,
    );
}

/// Convert a Color to RGBA float array.
///
/// The Color struct stores a pixel value and XRenderColor components.
/// This function extracts the RGB components for use with GlesRenderer.
fn color_to_rgba(color: &crate::drw::Color) -> [f32; 4] {
    let r = color.color.color.red as f32 / 65535.0;
    let g = color.color.color.green as f32 / 65535.0;
    let b = color.color.color.blue as f32 / 65535.0;
    let a = color.color.color.alpha as f32 / 65535.0;
    [r, g, b, a]
}

/// Convert a hex color string to RGBA float array.
///
/// Supports both 6-character (#RRGGBB) and 8-character (#RRGGBBAA) formats.
pub fn hex_to_rgba(hex: &str) -> [f32; 4] {
    let hex = hex.trim_start_matches('#');

    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
            [r, g, b, 1.0]
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
            let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255) as f32 / 255.0;
            [r, g, b, a]
        }
        _ => [0.0, 0.0, 0.0, 1.0],
    }
}

/// Convert RGBA float values to a Color.
///
/// This creates a Color struct suitable for use with the existing
/// color scheme system.
pub fn rgba_to_color(r: f32, g: f32, b: f32, a: f32) -> crate::drw::Color {
    use crate::drw::ffi::XRenderColor;
    use crate::drw::ffi::XftColor;

    crate::drw::Color {
        color: XftColor {
            pixel: ((r * 255.0) as u64) << 16
                | ((g * 255.0) as u64) << 8
                | ((b * 255.0) as u64)
                | ((a * 255.0) as u64) << 24,
            color: XRenderColor {
                red: (r * 65535.0) as u16,
                green: (g * 65535.0) as u16,
                blue: (b * 65535.0) as u16,
                alpha: (a * 65535.0) as u16,
            },
        },
    }
}
