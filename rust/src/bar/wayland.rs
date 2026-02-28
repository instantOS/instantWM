//! Wayland bar rendering using Smithay's GlesRenderer.
//!
//! This module provides GPU-accelerated bar rendering for the Wayland backend.
//! It uses Smithay's SolidColorRenderElement for rectangles and integrates
//! with the compositor's render loop.

use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::utils::{Logical, Point, Scale, Size};

use crate::bar::paint::{BarPainter, BarScheme};
use crate::bar::renderer::draw_bar_common;

// Kept for compatibility with other modules that expect a default value.
#[allow(dead_code)]
const DEFAULT_BAR_HEIGHT: i32 = 24;

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

pub struct WaylandBarPainter {
    elements: BarRenderElements,
    scheme: Option<BarScheme>,
    scale: Scale<f64>,
}

impl Default for WaylandBarPainter {
    fn default() -> Self {
        Self {
            elements: BarRenderElements::new(),
            scheme: None,
            scale: Scale::from(1.0),
        }
    }
}

impl WaylandBarPainter {
    pub fn begin(&mut self, scale: Scale<f64>) {
        self.elements = BarRenderElements::new();
        self.scheme = None;
        self.scale = scale;
    }

    pub fn finish(&mut self) -> Vec<SolidColorRenderElement> {
        self.elements.to_solid_elements(self.scale)
    }
}

impl BarPainter for WaylandBarPainter {
    fn text_width(&self, text: &str) -> i32 {
        text.len() as i32 * 8
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        self.scheme = Some(scheme);
    }

    fn scheme(&self) -> Option<&BarScheme> {
        self.scheme.as_ref()
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, filled: bool, invert: bool) {
        if !filled || w <= 0 || h <= 0 {
            return;
        }
        let Some(scheme) = self.scheme.clone() else {
            return;
        };
        let color = if invert { scheme.fg } else { scheme.bg };
        self.elements.add_rect(x, y, w, h, color);
    }

    fn text(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        lpad: i32,
        text: &str,
        invert: bool,
        detail_height: i32,
    ) -> i32 {
        let Some(scheme) = self.scheme.clone() else {
            return x;
        };
        let bg = if invert { scheme.fg } else { scheme.bg };
        let fg = if invert { scheme.bg } else { scheme.fg };
        self.elements.add_rect(x, y, w, h, bg);
        if detail_height > 0 {
            self.elements
                .add_rect(x, y + h - detail_height, w, detail_height, scheme.detail);
        }
        let text_width = self.text_width(text);
        let draw_w = (w - lpad).max(0).min(text_width);
        if draw_w > 0 {
            self.elements
                .add_text(x + lpad, y + (h - 12) / 2, draw_w, 12, fg);
        }
        x + w
    }
}

pub fn draw_bar_wayland(ctx: &mut crate::contexts::WmCtx, mon_idx: usize) {
    draw_bar_common_with_painter(ctx, mon_idx);
}
pub fn draw_bars_wayland(ctx: &mut crate::contexts::WmCtx) {
    // Ensure status_text_width is computed for bar hit-testing.
    ctx.g.status_text_width =
        crate::bar::renderer::compute_status_hit_width(ctx.bar_painter, &ctx.g.status_text);
}
pub fn reset_bar_wayland(ctx: &mut crate::contexts::WmCtx) {
    crate::bar::renderer::reset_bar_common(ctx);
}
pub fn should_draw_bar_wayland(ctx: &crate::contexts::WmCtx) -> bool {
    ctx.g.cfg.showbar
}

/// Render the bar for all monitors to Smithay render elements.
pub fn render_bar_elements(
    ctx: &mut crate::contexts::WmCtx,
    scale: Scale<f64>,
) -> Vec<SolidColorRenderElement> {
    let mut all_elements = Vec::new();
    let mon_indices: Vec<usize> = ctx.g.monitors_iter().map(|(i, _)| i).collect();
    for mon_idx in mon_indices {
        if let Some(monitor) = ctx.g.monitor(mon_idx) {
            if !monitor.shows_bar() {
                continue;
            }
        } else {
            continue;
        }
        ctx.bar_painter.begin(scale);
        draw_bar_common_with_painter(ctx, mon_idx);
        let mut elements = ctx.bar_painter.finish();
        all_elements.append(&mut elements);
    }

    all_elements
}

fn draw_bar_common_with_painter(ctx: &mut crate::contexts::WmCtx, mon_idx: usize) {
    let painter_ptr = ctx.bar_painter as *mut WaylandBarPainter;
    let ctx_ptr = ctx as *mut crate::contexts::WmCtx;
    unsafe {
        draw_bar_common(&mut *ctx_ptr, mon_idx, &mut *painter_ptr);
    }
}
