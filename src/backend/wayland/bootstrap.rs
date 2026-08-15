//! Backend-neutral Wayland WM initialization.

use crate::backend::WaylandBackendData;
use crate::config::init_config;
use crate::core_state::CoreState;

// ─────────────────────────────────────────────────────────────────────────────
// WM globals initialisation
// ─────────────────────────────────────────────────────────────────────────────

/// Initialise all WM globals that are shared between the nested and standalone
/// Wayland backends.
///
/// Reads `config.toml`, applies tag/key configuration, sets bar metrics, and
/// calls `update_geom` so that monitor layout is valid before the first frame.
///
/// The caller is responsible for setting `wm.core.config.screen_width` /
/// `screen_height` to the actual output dimensions afterwards (e.g. from the
/// winit window size or DRM connector mode).  The values written here
/// Wayland-specific globals initialization.
///
/// Sets up config, tags, and bar painter font size. This is called before
/// the Wayland compositor is fully initialized, so monitor geometry is not
/// available yet - that will be done via update_geom later.
/// Apply font-derived bar metrics to the runtime config and bar painter.
///
/// Computes `bar_height` and `horizontal_padding` from the font config and
/// applies them to the given `CoreState`. Also updates the bar painter's font
/// size. Shared by both startup (`init_globals`) and reload.
pub fn apply_bar_metrics(state: &mut CoreState, data: &mut WaylandBackendData) {
    let font_size = state.config.fonts.size();
    let font_families = state.config.fonts.families();
    let metrics = state.config.fonts.bar_metrics(state.config.bar.height);

    data.bar_painter.set_font_size(font_size);
    data.bar_painter.set_font_families(&font_families);

    state.config.derived.bar_height = metrics.height;
    state.config.derived.bar_horizontal_padding = metrics.horizontal_padding;
}

pub fn init_globals(state: &mut CoreState, wayland: &mut WaylandBackendData) {
    let cfg = init_config(crate::backend::BackendKind::Wayland);
    state.config.derived.display.width = 1280;
    state.config.derived.display.height = 800;
    crate::core_state::apply_config(state, &cfg);
    state.config.bar.show = true;

    apply_bar_metrics(state, wayland);

    // Monitor geometry will be set up after the compositor is ready via update_geom
}
