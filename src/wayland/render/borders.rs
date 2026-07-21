//! Wayland border rendering.
//!
//! Generates solid color render elements for window borders, handling
//! z-order occlusion (borders behind windows are clipped).

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::desktop::PopupManager;

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::model::WmModel;
use crate::types::{BorderColorConfig, Rect, Size, WindowId};

/// Information about a window needed for border rendering.
#[derive(Debug, Clone, Copy)]
struct WindowBorderInfo {
    id: WindowId,
    content_rect: Rect,
    border_width: i32,
    is_visible: bool,
    is_hidden: bool,
    is_floating: bool,
    is_tiling_layout: bool,
}

impl WindowBorderInfo {
    /// Total outer size including borders.
    fn outer_size(&self) -> Size {
        let border_width = self.border_width;
        Size::new(
            self.content_rect.w + 2 * border_width,
            self.content_rect.h + 2 * border_width,
        )
    }

    /// Bounding rectangle including borders.
    fn bounding_rect(&self) -> Rect {
        self.content_rect.with_size(self.outer_size())
    }

    /// Checks if this window should render borders.
    fn has_borders(&self) -> bool {
        self.is_visible && !self.is_hidden && self.border_width > 0
    }

    /// Returns the border color based on focus state.
    fn border_color(
        &self,
        is_focused: bool,
        colors: &BorderColorConfig,
    ) -> crate::bar::color::Rgba {
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
fn collect_window_info(model: &WmModel, state: &WaylandState) -> Vec<WindowBorderInfo> {
    let mut windows = Vec::new();

    for window in state.space.elements() {
        let Some(marker) = window.user_data().get::<WindowIdMarker>() else {
            continue;
        };
        let Some(view) = model.client_view(marker.id) else {
            continue;
        };
        let c = view.client;

        let size = window.geometry().size;
        let content_rect = Rect::new(c.geo.x, c.geo.y, size.w.max(1), size.h.max(1));

        let is_visible = c.is_visible(view.monitor.selected_tags());
        let is_tiling_layout = view.monitor.is_tiling_layout();

        windows.push(WindowBorderInfo {
            id: marker.id,
            content_rect,
            border_width: c.border_width.max(0),
            is_visible,
            is_hidden: c.is_hidden,
            is_floating: c.mode.is_floating(),
            is_tiling_layout,
        });
    }

    windows
}

/// Generates the four border rectangles for a window.
fn generate_border_rectangles(outer_rect: Rect, border_width: i32) -> Vec<Rect> {
    if border_width <= 0 || outer_rect.w <= 2 * border_width || outer_rect.h <= 2 * border_width {
        return Vec::new();
    }

    let inner_height = (outer_rect.h - 2 * border_width).max(0);

    vec![
        // Top border
        Rect::new(outer_rect.x, outer_rect.y, outer_rect.w, border_width),
        // Bottom border
        Rect::new(
            outer_rect.x,
            outer_rect.y + outer_rect.h - border_width,
            outer_rect.w,
            border_width,
        ),
        // Left border (between top and bottom)
        Rect::new(
            outer_rect.x,
            outer_rect.y + border_width,
            border_width,
            inner_height,
        ),
        // Right border (between top and bottom)
        Rect::new(
            outer_rect.x + outer_rect.w - border_width,
            outer_rect.y + border_width,
            border_width,
            inner_height,
        ),
    ]
}

/// Subtracts occluders from border parts, returning the remaining visible parts.
/// Reuses the scratch vector's capacity to avoid heap allocations.
fn apply_occluders(
    border_parts: Vec<Rect>,
    occluders: &[Rect],
    scratch: &mut Vec<Rect>,
) -> Vec<Rect> {
    let mut remaining = border_parts;
    scratch.clear();

    for occluder in occluders {
        if remaining.is_empty() {
            break;
        }
        for part in remaining.drain(..) {
            scratch.extend(part.subtract(occluder));
        }
        std::mem::swap(&mut remaining, scratch);
        scratch.clear();
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

/// Collects bounding rectangles of all currently-mapped xdg popups in
/// compositor coordinates.
///
/// Popups (e.g. right-click menus) are emitted by smithay alongside their
/// parent toplevel in the same render bucket as window surfaces, which sits
/// below the WM's border bucket. Without explicit occlusion, borders would
/// paint over popups that extend past their parent window. We treat every
/// popup as an occluder for every border so popups appear on top.
fn build_popup_occluders(state: &WaylandState) -> Vec<Rect> {
    let mut occluders = Vec::new();
    for window in state.space.elements() {
        let Some(toplevel) = window.toplevel() else {
            continue;
        };
        let Some(space_loc) = state.space.element_location(window) else {
            continue;
        };
        let window_geometry = window.geometry();
        for (popup, popup_offset) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
            let popup_geometry = popup.geometry();
            if popup_geometry.size.w <= 0 || popup_geometry.size.h <= 0 {
                continue;
            }
            occluders.push(Rect::new(
                space_loc.x + window_geometry.loc.x + popup_offset.x,
                space_loc.y + window_geometry.loc.y + popup_offset.y,
                popup_geometry.size.w,
                popup_geometry.size.h,
            ));
        }
    }
    occluders
}

/// Compute a zero-allocation u64 hash representing the current compositor state
/// that affects borders (geometries, focus, tag masks, and popups).
pub fn get_borders_hash(
    model: &WmModel,
    state: &WaylandState,
    layout_preview: Option<Rect>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    // 1. Selected window
    model.selected_win().hash(&mut hasher);
    if let Some(preview) = layout_preview {
        preview.x.hash(&mut hasher);
        preview.y.hash(&mut hasher);
        preview.w.hash(&mut hasher);
        preview.h.hash(&mut hasher);
    }

    // 2. Monitor layout / selected tags
    for mon in model.monitors.iter_all() {
        mon.id().hash(&mut hasher);
        mon.monitor_rect.x.hash(&mut hasher);
        mon.monitor_rect.y.hash(&mut hasher);
        mon.monitor_rect.w.hash(&mut hasher);
        mon.monitor_rect.h.hash(&mut hasher);
        mon.selected_tags().hash(&mut hasher);
        mon.is_tiling_layout().hash(&mut hasher);
    }

    // 3. Window and Popup properties
    for window in state.space.elements() {
        if let Some(marker) = window.user_data().get::<WindowIdMarker>() {
            marker.id.hash(&mut hasher);
            if let Some(c) = model.client(marker.id) {
                c.geo.x.hash(&mut hasher);
                c.geo.y.hash(&mut hasher);
                c.border_width.hash(&mut hasher);
                c.is_hidden.hash(&mut hasher);
                c.mode.is_floating().hash(&mut hasher);
            }
            let size = window.geometry().size;
            size.w.hash(&mut hasher);
            size.h.hash(&mut hasher);
        }

        if let Some(toplevel) = window.toplevel()
            && let Some(space_loc) = state.space.element_location(window)
        {
            let window_geometry = window.geometry();
            for (popup, popup_offset) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                let popup_geometry = popup.geometry();
                space_loc.x.hash(&mut hasher);
                space_loc.y.hash(&mut hasher);
                window_geometry.loc.x.hash(&mut hasher);
                window_geometry.loc.y.hash(&mut hasher);
                popup_offset.x.hash(&mut hasher);
                popup_offset.y.hash(&mut hasher);
                popup_geometry.size.w.hash(&mut hasher);
                popup_geometry.size.h.hash(&mut hasher);
            }
        }
    }

    hasher.finish()
}

/// Renders border elements for all visible windows.
pub fn render_border_elements(
    model: &WmModel,
    colors: &BorderColorConfig,
    state: &WaylandState,
    layout_preview: Option<Rect>,
) -> Vec<SolidColorRenderElement> {
    let windows = collect_window_info(model, state);
    let selected_win = model.selected_win();
    let mut elements = Vec::new();

    // Build occluders list (each window can occlude borders behind it)
    let occluders: Vec<Rect> = build_occluders(&windows);
    // Popups always render above borders, so they occlude every border.
    let popup_occluders: Vec<Rect> = build_popup_occluders(state);

    let mut scratch = Vec::with_capacity(32);

    for (idx, window) in windows.iter().enumerate() {
        if !window.has_borders() {
            continue;
        }

        let border_width = window.border_width;

        // Generate the four border sides
        let border_parts = generate_border_rectangles(window.bounding_rect(), border_width);
        if border_parts.is_empty() {
            continue;
        }

        // Subtract occluders from higher windows (windows in front)
        let higher_occluders = &occluders[idx + 1..];
        let visible_parts = apply_occluders(border_parts, higher_occluders, &mut scratch);
        // Subtract popup areas so right-click menus and similar overlays
        // are not covered by borders.
        let visible_parts = apply_occluders(visible_parts, &popup_occluders, &mut scratch);

        // Get color based on focus state
        let is_focused = Some(window.id) == selected_win;
        let color = window.border_color(is_focused, colors);

        // Create render elements for visible border parts
        for part in visible_parts {
            push_solid(&mut elements, part, color);
        }
    }

    if let Some(preview) = layout_preview {
        for side in crate::layouts::placement::outline_rectangles(
            preview,
            crate::layouts::placement::LAYOUT_PREVIEW_BORDER_WIDTH,
        ) {
            push_solid(&mut elements, side, colors.snap);
        }
    }

    elements
}

fn push_solid(out: &mut Vec<SolidColorRenderElement>, rect: Rect, color: crate::bar::color::Rgba) {
    if !rect.size().is_positive() {
        return;
    }
    let buffer = SolidColorBuffer::new((rect.w, rect.h), color.into_array());
    out.push(SolidColorRenderElement::from_buffer(
        &buffer,
        (rect.x, rect.y),
        smithay::utils::Scale::from(1.0),
        1.0,
        Kind::Unspecified,
    ));
}
