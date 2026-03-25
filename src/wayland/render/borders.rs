//! Wayland border rendering.
//!
//! Generates solid color render elements for window borders, handling
//! z-order occlusion (borders behind windows are clipped).

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::globals::Globals;
use crate::types::{BorderColorConfig, Rect, WindowId};

/// Information about a window needed for border rendering.
#[derive(Debug, Clone, Copy)]
struct WindowBorderInfo {
    id: WindowId,
    geo: Rect,
    border_width: i32,
    content_size: (i32, i32),
    is_visible: bool,
    is_hidden: bool,
    is_floating: bool,
    is_tiling_layout: bool,
}

impl WindowBorderInfo {
    /// Total outer size including borders.
    fn outer_size(&self) -> (i32, i32) {
        let bw = self.border_width;
        let (cw, ch) = self.content_size;
        (cw + 2 * bw, ch + 2 * bw)
    }

    /// Bounding rectangle including borders.
    fn bounding_rect(&self) -> Rect {
        let (ow, oh) = self.outer_size();
        Rect::new(self.geo.x, self.geo.y, ow, oh)
    }

    /// Checks if this window should render borders.
    fn has_borders(&self) -> bool {
        self.is_visible && !self.is_hidden && self.border_width > 0
    }

    /// Returns the border color based on focus state.
    fn border_color(&self, is_focused: bool, colors: &BorderColorConfig) -> [f32; 4] {
        if is_focused {
            if self.is_floating || !self.is_tiling_layout {
                colors.float_focus
            } else {
                colors.tile_focus
            }
        } else {
            colors.normal
        }
    }
}

/// Collects window information from the compositor state.
fn collect_window_info(g: &Globals, state: &WaylandState) -> Vec<WindowBorderInfo> {
    let mut windows = Vec::new();

    for window in state.space.elements() {
        let Some(marker) = window.user_data().get::<WindowIdMarker>() else {
            continue;
        };
        let Some(c) = g.clients.get(&marker.id) else {
            continue;
        };

        let size = window.geometry().size;
        let content_size = (size.w.max(1), size.h.max(1));

        let is_visible = g
            .monitor(c.monitor_id)
            .map(|m| c.is_visible(m.selected_tags()))
            .unwrap_or(false);

        let is_tiling_layout = g
            .monitor(c.monitor_id)
            .map(|m| m.is_tiling_layout())
            .unwrap_or(true);

        windows.push(WindowBorderInfo {
            id: marker.id,
            geo: c.geo,
            border_width: c.border_width.max(0),
            content_size,
            is_visible,
            is_hidden: c.is_hidden,
            is_floating: c.is_floating,
            is_tiling_layout,
        });
    }

    windows
}

/// Generates the four border rectangles for a window.
fn generate_border_rectangles(x: i32, y: i32, outer_w: i32, outer_h: i32, bw: i32) -> Vec<Rect> {
    if bw <= 0 || outer_w <= 2 * bw || outer_h <= 2 * bw {
        return Vec::new();
    }

    let inner_h = (outer_h - 2 * bw).max(0);

    vec![
        // Top border
        Rect::new(x, y, outer_w, bw),
        // Bottom border
        Rect::new(x, y + outer_h - bw, outer_w, bw),
        // Left border (between top and bottom)
        Rect::new(x, y + bw, bw, inner_h),
        // Right border (between top and bottom)
        Rect::new(x + outer_w - bw, y + bw, bw, inner_h),
    ]
}

/// Subtracts occluders from border parts, returning the remaining visible parts.
fn apply_occluders(border_parts: Vec<Rect>, occluders: &[Rect]) -> Vec<Rect> {
    let mut remaining = border_parts;

    for occluder in occluders {
        if remaining.is_empty() {
            break;
        }
        remaining = remaining
            .into_iter()
            .flat_map(|part| part.subtract(occluder))
            .collect();
    }

    remaining
}

/// Builds occluder rectangles from windows (windows block borders behind them).
fn build_occluders(windows: &[WindowBorderInfo]) -> Vec<Rect> {
    windows
        .iter()
        .filter(|w| w.is_visible)
        .map(|w| w.bounding_rect())
        .collect()
}

/// Renders border elements for all visible windows.
pub fn render_border_elements(g: &Globals, state: &WaylandState) -> Vec<SolidColorRenderElement> {
    let windows = collect_window_info(g, state);
    let selected_win = g.selected_win();
    let colors = &g.cfg.bordercolors;
    let mut elements = Vec::new();

    // Build occluders list (each window can occlude borders behind it)
    let occluders: Vec<Rect> = build_occluders(&windows);

    for (idx, window) in windows.iter().enumerate() {
        if !window.has_borders() {
            continue;
        }

        let (outer_w, outer_h) = window.outer_size();
        let bw = window.border_width;

        // Generate the four border sides
        let border_parts =
            generate_border_rectangles(window.geo.x, window.geo.y, outer_w, outer_h, bw);
        if border_parts.is_empty() {
            continue;
        }

        // Subtract occluders from higher windows (windows in front)
        let higher_occluders = &occluders[idx + 1..];
        let visible_parts = apply_occluders(border_parts, higher_occluders);

        // Get color based on focus state
        let is_focused = Some(window.id) == selected_win;
        let color = window.border_color(is_focused, colors);

        // Create render elements for visible border parts
        for part in visible_parts {
            push_solid(&mut elements, part.x, part.y, part.w, part.h, color);
        }
    }

    elements
}

fn push_solid(
    out: &mut Vec<SolidColorRenderElement>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: [f32; 4],
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let buffer = SolidColorBuffer::new((w, h), color);
    out.push(SolidColorRenderElement::from_buffer(
        &buffer,
        (x, y),
        smithay::utils::Scale::from(1.0),
        1.0,
        Kind::Unspecified,
    ));
}
