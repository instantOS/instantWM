//! Wayland border rendering.
//!
//! Generates solid color render elements for window borders, handling
//! z-order occlusion (borders behind windows are clipped).

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::desktop::PopupManager;

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::model::WmModel;
use crate::types::{BorderColorConfig, Rect, WindowId};

/// Information about a window needed for border rendering.
#[derive(Debug, Clone, Copy)]
struct WindowBorderInfo {
    id: WindowId,
    /// Currently presented rectangle in the core convention: outer origin,
    /// content size. This is intentionally not the logical target rectangle.
    displayed_rect: Rect,
    border_width: i32,
    is_visible: bool,
    is_hidden: bool,
    is_floating: bool,
    is_tiling_layout: bool,
}

impl WindowBorderInfo {
    /// Bounding rectangle including borders.
    fn bounding_rect(&self) -> Rect {
        self.displayed_rect.with_borders(self.border_width)
    }

    /// Checks if this window should render borders.
    fn has_borders(&self) -> bool {
        self.is_visible && !self.is_hidden && self.border_width > 0
    }

    fn occluder(&self) -> Option<Rect> {
        (self.is_visible && !self.is_hidden).then(|| self.bounding_rect())
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

/// A coherent snapshot of all compositor state needed to draw borders.
///
/// Capturing displayed rectangles and popup occluders together ensures cache
/// identity and rendering use the same animation frame. Logical model state is
/// retained only for non-geometric policy such as visibility and border color.
pub struct BorderScene {
    windows: Vec<WindowBorderInfo>,
    popup_occluders: Vec<Rect>,
    selected_win: Option<WindowId>,
}

impl BorderScene {
    pub fn capture(model: &WmModel, state: &WaylandState) -> Self {
        Self {
            windows: collect_window_info(model, state),
            popup_occluders: build_popup_occluders(state),
            selected_win: model.selected_win(),
        }
    }

    /// Hash exactly the captured inputs which affect generated border
    /// elements. A displayed animation step therefore invalidates borders,
    /// while unrelated logical model changes do not.
    pub fn cache_key(&self, colors: &BorderColorConfig) -> u64 {
        use std::hash::{Hash, Hasher};

        fn hash_rect(rect: Rect, hasher: &mut impl Hasher) {
            rect.x.hash(hasher);
            rect.y.hash(hasher);
            rect.w.hash(hasher);
            rect.h.hash(hasher);
        }

        fn hash_color(color: crate::bar::color::Rgba, hasher: &mut impl Hasher) {
            for component in color.into_array() {
                component.to_bits().hash(hasher);
            }
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.selected_win.hash(&mut hasher);
        for window in &self.windows {
            window.id.hash(&mut hasher);
            hash_rect(window.displayed_rect, &mut hasher);
            window.border_width.hash(&mut hasher);
            window.is_visible.hash(&mut hasher);
            window.is_hidden.hash(&mut hasher);
            window.is_floating.hash(&mut hasher);
            window.is_tiling_layout.hash(&mut hasher);
        }
        for popup in &self.popup_occluders {
            hash_rect(*popup, &mut hasher);
        }
        hash_color(colors.normal, &mut hasher);
        hash_color(colors.tile_focus, &mut hasher);
        hash_color(colors.float_focus, &mut hasher);
        hasher.finish()
    }

    pub fn render(&self, colors: &BorderColorConfig) -> Vec<SolidColorRenderElement> {
        render_border_scene(self, colors)
    }
}

/// Collect window policy from the model and displayed geometry from the
/// compositor. Never substitute `client.geo` here: it is the logical target.
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

        let Some(displayed_rect) = state.displayed_window_rect(window, c.border_width) else {
            continue;
        };

        let is_visible = c.is_visible(view.monitor.visible_tags());
        let is_tiling_layout = view.monitor.is_tiling_layout();

        windows.push(WindowBorderInfo {
            id: marker.id,
            displayed_rect,
            border_width: c.border_width.max(0),
            is_visible,
            is_hidden: c.is_hidden,
            is_floating: c.mode().is_normal_floating(),
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
            outer_rect.bottom() - border_width,
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
            outer_rect.right() - border_width,
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
    occluders: impl IntoIterator<Item = Rect>,
    scratch: &mut Vec<Rect>,
) -> Vec<Rect> {
    let mut remaining = border_parts;
    scratch.clear();

    for occluder in occluders {
        if remaining.is_empty() {
            break;
        }
        // Skip occluders that touch no remaining part: subtract is a no-op
        // there (returns the part unchanged) but would still allocate per part.
        if !remaining
            .iter()
            .any(|part| part.intersects_other(&occluder))
        {
            continue;
        }
        for part in remaining.drain(..) {
            scratch.extend(part.subtract(&occluder));
        }
        std::mem::swap(&mut remaining, scratch);
        scratch.clear();
    }

    remaining
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

/// Return the visible border pieces for one window after applying all
/// higher-z-order window and popup occluders.
fn visible_border_parts(
    scene: &BorderScene,
    window_index: usize,
    scratch: &mut Vec<Rect>,
) -> Vec<Rect> {
    let Some(window) = scene
        .windows
        .get(window_index)
        .filter(|window| window.has_borders())
    else {
        return Vec::new();
    };

    let border_parts = generate_border_rectangles(window.bounding_rect(), window.border_width);
    if border_parts.is_empty() {
        return border_parts;
    }

    let higher_occluders = scene.windows[window_index + 1..]
        .iter()
        .filter_map(WindowBorderInfo::occluder);
    let visible_parts = apply_occluders(border_parts, higher_occluders, scratch);
    apply_occluders(
        visible_parts,
        scene.popup_occluders.iter().copied(),
        scratch,
    )
}

/// Render border elements from a previously captured displayed scene.
fn render_border_scene(
    scene: &BorderScene,
    colors: &BorderColorConfig,
) -> Vec<SolidColorRenderElement> {
    let windows = &scene.windows;
    let mut elements = Vec::new();

    let mut scratch = Vec::with_capacity(32);

    for (idx, window) in windows.iter().enumerate() {
        let visible_parts = visible_border_parts(scene, idx, &mut scratch);

        // Get color based on focus state
        let is_focused = Some(window.id) == scene.selected_win;
        let color = window.border_color(is_focused, colors);

        // Create render elements for visible border parts
        for part in visible_parts {
            push_solid(&mut elements, part, color);
        }
    }

    elements
}

/// Append the four inexpensive solid elements forming the animated placement
/// preview. Keeping these out of the shared-scene cache avoids rebuilding
/// every client border on each animation frame.
pub fn append_layout_preview(
    out: &mut Vec<SolidColorRenderElement>,
    preview: Option<Rect>,
    color: crate::bar::color::Rgba,
) {
    let Some(preview) = preview else {
        return;
    };
    for side in crate::layouts::placement::outline_rectangles(
        preview,
        crate::layouts::placement::LAYOUT_PREVIEW_BORDER_WIDTH,
    ) {
        push_solid(out, side, color);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::color::Rgba;

    fn window_at(displayed_rect: Rect) -> WindowBorderInfo {
        WindowBorderInfo {
            id: WindowId(1),
            displayed_rect,
            border_width: 3,
            is_visible: true,
            is_hidden: false,
            is_floating: true,
            is_tiling_layout: true,
        }
    }

    fn scene_at(displayed_rect: Rect) -> BorderScene {
        BorderScene {
            windows: vec![window_at(displayed_rect)],
            popup_occluders: Vec::new(),
            selected_win: Some(WindowId(1)),
        }
    }

    #[test]
    fn border_geometry_surrounds_the_displayed_content_rect() {
        let window = window_at(Rect::new(100, 200, 800, 600));

        assert_eq!(
            generate_border_rectangles(window.bounding_rect(), window.border_width),
            vec![
                Rect::new(100, 200, 806, 3),
                Rect::new(100, 803, 806, 3),
                Rect::new(100, 203, 3, 600),
                Rect::new(903, 203, 3, 600),
            ]
        );
    }

    #[test]
    fn displayed_animation_step_changes_border_cache_identity() {
        let colors = BorderColorConfig::default();
        let first = scene_at(Rect::new(100, 200, 800, 600));
        let next = scene_at(Rect::new(108, 200, 800, 600));

        assert_ne!(first.cache_key(&colors), next.cache_key(&colors));
    }

    #[test]
    fn border_color_changes_cache_identity() {
        let scene = scene_at(Rect::new(100, 200, 800, 600));
        let first = BorderColorConfig::default();
        let mut next = first;
        next.float_focus = Rgba::rgb(0.25, 0.5, 0.75);

        assert_ne!(scene.cache_key(&first), scene.cache_key(&next));
    }

    #[test]
    fn hidden_windows_do_not_occlude_visible_borders() {
        let mut hidden = window_at(Rect::new(100, 200, 800, 600));
        hidden.is_hidden = true;

        assert_eq!(hidden.occluder(), None);
    }

    #[test]
    fn invisible_lower_window_does_not_shift_higher_occluder_index() {
        let mut invisible_lower = window_at(Rect::new(100, 100, 10, 10));
        invisible_lower.id = WindowId(1);
        invisible_lower.is_visible = false;

        let mut target = window_at(Rect::new(0, 0, 20, 20));
        target.id = WindowId(2);

        let mut higher = window_at(Rect::new(10, 0, 10, 10));
        higher.id = WindowId(3);

        let scene = BorderScene {
            windows: vec![invisible_lower, target, higher],
            popup_occluders: Vec::new(),
            selected_win: None,
        };
        let mut scratch = Vec::new();

        let parts = visible_border_parts(&scene, 1, &mut scratch);

        assert!(parts.contains(&Rect::new(0, 0, 10, 3)));
        assert!(!parts.contains(&Rect::new(0, 0, 26, 3)));
    }
}
