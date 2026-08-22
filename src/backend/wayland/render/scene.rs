//! Scene construction shared by nested and DRM renderers.

use std::rc::Rc;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Element, Id};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::PopupManager;
use smithay::output::Output;
use smithay::wayland::seat::WaylandFocus;

use crate::backend::Backend;
use crate::backend::wayland::compositor::WaylandState;
use crate::contexts::CoreCtx;
use crate::wm::Wm;

// ─────────────────────────────────────────────────────────────────────────────
// Bar render elements
// ─────────────────────────────────────────────────────────────────────────────

/// Build the `MemoryRenderBufferRenderElement` list for the status bar.
///
/// Returns an empty `Vec` when `wm.core.config.showbar` is `false`.
///
/// The caller is responsible for adding the returned elements to its own
/// custom-element list under the appropriate backend-specific wrapper variant
/// (e.g. `DrmExtras::Memory` or `WaylandExtras::Memory`).
pub fn build_bar_buffers(
    wm: &mut Wm,
    state: &mut WaylandState,
) -> Vec<(MemoryRenderBuffer, crate::types::Point)> {
    let show_top = wm.core.config.bar.show;
    let show_bottom = wm.core.config.bar.show_bottom
        || wm
            .core
            .model
            .monitors_iter()
            .any(|(_, m)| m.shows_bottom_bar());
    if !show_top && !show_bottom {
        return Vec::new();
    }

    let mut core = CoreCtx::new(
        &mut wm.core,
        &mut wm.work,
        &mut wm.running,
        &mut wm.bar,
        &mut wm.focus,
    );

    let mut buffers = if show_top {
        let Backend::Wayland(data) = &mut wm.backend else {
            return Vec::new();
        };

        data.bar_painter
            .set_render_ping(state.runtime.render_ping.clone());
        crate::bar::wayland::render_bar_buffers(
            &mut core,
            &mut data.bar_painter,
            smithay::utils::Scale::from(1.0),
        )
    } else {
        Vec::new()
    };

    if show_bottom {
        buffers.extend(crate::bar::wayland::build_bottom_bar_buffers(&mut core));
    }

    buffers
}

/// Shared render elements captured once and reused across output renders in
/// the same frame. Bar buffers retain independent shared ownership because
/// they are expensive and remain stable while borders advance each animation
/// frame; borders belong directly to this displayed-scene snapshot.
#[derive(Clone)]
pub struct SharedSceneElements {
    pub bar_buffers: Rc<Vec<(MemoryRenderBuffer, crate::types::Point)>>,
    pub borders: Vec<SolidColorRenderElement>,
    pub layout_preview_color: crate::types::color::Rgba,
}

/// Renderer-owned cache shared across consecutive frames.
#[derive(Default)]
pub struct SceneCache {
    entry: Option<(u64, u64, Rc<SharedSceneElements>)>,
}

/// Capture shared scene pieces that do not depend on the target output.
pub fn build_shared_scene_elements(
    wm: &mut Wm,
    state: &mut WaylandState,
    cache: &mut SceneCache,
) -> Rc<SharedSceneElements> {
    let layout_preview_color = match state.layout_preview_style() {
        crate::types::InteractionOutlineStyle::Layout => wm.core.config.colors.border.snap,
        crate::types::InteractionOutlineStyle::Close => {
            wm.core.config.colors.close_button.gesture_color()
        }
    };
    let bar_seq = wm.bar.update_seq();
    let border_scene =
        crate::backend::wayland::render::borders::BorderScene::capture(&wm.core.model, state);
    let borders_hash = border_scene.cache_key(&wm.core.config.colors.border, layout_preview_color);
    let bar_dirty = wm.bar.needs_redraw();

    if !bar_dirty
        && let Some((cached_bar, cached_borders, ref elements)) = cache.entry
        && cached_bar == bar_seq
        && cached_borders == borders_hash
        && elements.layout_preview_color == layout_preview_color
    {
        return elements.clone();
    }

    let cached_bar_buffers = cache
        .entry
        .as_ref()
        .filter(|(cached_bar, _, _)| *cached_bar == bar_seq && !bar_dirty)
        .map(|(_, _, elements)| elements.bar_buffers.clone());
    let bar_buffers = cached_bar_buffers.unwrap_or_else(|| Rc::new(build_bar_buffers(wm, state)));
    let borders = border_scene.render(&wm.core.config.colors.border, layout_preview_color);

    let elements = Rc::new(SharedSceneElements {
        bar_buffers,
        borders,
        layout_preview_color,
    });

    if !wm.bar.needs_redraw() {
        cache.entry = Some((bar_seq, borders_hash, elements.clone()));
    }
    elements
}

/// Backend-agnostic render element buckets used by both Wayland startup paths.
pub struct CommonSceneElements {
    pub overlays: Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
    pub bar: Vec<MemoryRenderBufferRenderElement<GlesRenderer>>,
    pub borders: Vec<SolidColorRenderElement>,
}

/// Build the shared set of scene extras used by both startup renderers.
pub fn build_common_scene_elements(
    wm: &mut Wm,
    state: &mut WaylandState,
    cache: &mut SceneCache,
    renderer: &mut GlesRenderer,
    output: &Output,
) -> CommonSceneElements {
    let shared = build_shared_scene_elements(wm, state, cache);
    build_common_scene_elements_from_shared(state, renderer, output, &shared)
}

/// Build the full scene for one output from reusable shared pieces.
pub fn build_common_scene_elements_from_shared(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    output: &Output,
    shared: &SharedSceneElements,
) -> CommonSceneElements {
    use smithay::backend::renderer::element::AsRenderElements;

    let output_scale = output.current_scale().fractional_scale();
    let render_scale = smithay::utils::Scale::from(output_scale);
    let mut overlays = Vec::new();
    for (window, logical_loc) in state.overlay_windows_for_render(output) {
        let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            AsRenderElements::render_elements(
                &window,
                renderer,
                logical_loc.to_physical_precise_round(output_scale),
                render_scale,
                1.0,
            );
        overlays.extend(elems);
    }
    append_native_popup_elements(state, renderer, output, output_scale, &mut overlays);

    let mut bar = Vec::new();
    for (buffer, position) in shared.bar_buffers.iter() {
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (position.x as f64, position.y as f64),
            buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            Ok(elem) => bar.push(elem),
            Err(e) => log::warn!("bar buffer upload failed: {:?}", e),
        }
    }

    let mut borders = shared.borders.clone();
    crate::backend::wayland::render::borders::append_layout_preview(
        &mut borders,
        (state.layout_preview_style() == crate::types::InteractionOutlineStyle::Layout)
            .then(|| state.layout_preview_rect())
            .flatten(),
        shared.layout_preview_color,
    );

    CommonSceneElements {
        overlays,
        bar,
        borders,
    }
}

/// Render native XDG popups as a compositor-wide foreground layer.
///
/// Smithay normally emits a popup together with its parent window. That makes
/// the popup disappear behind any higher sibling as soon as it crosses the
/// parent's boundary. A grabbed menu is transient compositor UI and must stay
/// above ordinary toplevels, independently of its parent's persistent stack
/// position. Duplicate popup elements are removed from the Space bucket later.
fn append_native_popup_elements(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    output: &Output,
    output_scale: f64,
    overlays: &mut Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
) {
    let Some(output_rect) = state.space.output_geometry(output) else {
        return;
    };
    let scale = smithay::utils::Scale::from(output_scale);

    for (window, window_type) in state.windows_in_z_order() {
        // Explicit overlay windows are already rendered in full above.
        if window_type.is_overlay() {
            continue;
        }
        let Some(toplevel) = window.wl_surface() else {
            continue;
        };
        let Some(window_loc) = state.space.element_location(window) else {
            continue;
        };

        for (popup, popup_offset) in PopupManager::popups_for_surface(&toplevel) {
            let logical_loc = window_loc + popup_offset - popup.geometry().loc - output_rect.loc;
            overlays.extend(render_elements_from_surface_tree(
                renderer,
                popup.wl_surface(),
                logical_loc.to_physical_precise_round(output_scale),
                scale,
                1.0,
                Kind::Unspecified,
            ));
        }
    }
}

/// Remove Smithay-space surface elements already emitted in the foreground
/// bucket. Render-element IDs are stable across both paths, so this removes
/// both explicit overlay windows and promoted native popups without drawing
/// either surface tree twice.
pub fn remove_duplicate_overlay_elements<E: Element>(
    scene: &CommonSceneElements,
    space_elements: &mut Vec<E>,
) {
    if scene.overlays.is_empty() {
        return;
    }
    let overlay_ids: Vec<Id> = scene
        .overlays
        .iter()
        .map(|element| element.id().clone())
        .collect();
    space_elements.retain(|element| !overlay_ids.iter().any(|id| id == element.id()));
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc
// ─────────────────────────────────────────────────────────────────────────────

pub fn output_has_real_fullscreen(wm: &Wm, output: &Output) -> bool {
    let output_name = output.name();
    let Some(monitor) = wm
        .core
        .model
        .monitors
        .iter_all()
        .find(|m| m.name == output_name)
    else {
        return false;
    };
    let selected_tags = monitor.visible_tags();
    monitor
        .iter_clients(&wm.core.model.clients)
        .any(|(_, client)| client.mode().is_true_fullscreen() && client.is_visible(selected_tags))
}

// ─────────────────────────────────────────────────────────────────────────────
// Layer shell rendering helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Count the number of render elements in upper layer shells (Overlay/Top).
///
/// This is used by both backends to determine how many space render elements
/// to place before the bar and borders.
pub fn count_upper_layer_render_elements(renderer: &mut GlesRenderer, output: &Output) -> usize {
    let layer_map = smithay::desktop::layer_map_for_output(output);
    let output_scale = output.current_scale().fractional_scale();
    let mut num_upper = 0;

    for surface in layer_map.layers().rev() {
        if matches!(
            surface.layer(),
            smithay::wayland::shell::wlr_layer::Layer::Background
                | smithay::wayland::shell::wlr_layer::Layer::Bottom
        ) {
            continue;
        }
        if let Some(geo) = layer_map.layer_geometry(surface) {
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::AsRenderElements::render_elements(
                    surface,
                    renderer,
                    geo.loc.to_physical_precise_round(output_scale),
                    smithay::utils::Scale::from(output_scale),
                    1.0,
                );
            num_upper += elems.len();
        }
    }

    num_upper
}

/// Helper struct to track element counts for pre-allocating the render vector.
#[derive(Default)]
pub struct RenderElementCounts {
    pub overlays: usize,
    pub upper_layers: usize,
    pub bar: usize,
    pub borders: usize,
    pub space: usize,
}

impl RenderElementCounts {
    /// Calculate total capacity needed.
    pub fn total(&self) -> usize {
        self.overlays + self.upper_layers + self.bar + self.borders + self.space
    }
}

/// Get the render element counts for a frame.
///
/// This helps pre-allocate the render element vector with the right capacity.
pub fn get_render_element_counts(
    scene: &CommonSceneElements,
    space_render_elements_len: usize,
    num_upper: usize,
) -> RenderElementCounts {
    RenderElementCounts {
        overlays: scene.overlays.len(),
        upper_layers: num_upper,
        bar: scene.bar.len(),
        borders: scene.borders.len(),
        space: space_render_elements_len.saturating_sub(num_upper),
    }
}
