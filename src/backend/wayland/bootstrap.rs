//! Backend-neutral Wayland WM initialization.

use crate::backend::WaylandBackendData;
use crate::config::load_startup_config;
use crate::core_state::CoreState;

// ─────────────────────────────────────────────────────────────────────────────
// WM globals initialisation
// ─────────────────────────────────────────────────────────────────────────────

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

    state.derived.bar_height = metrics.height;
    state.derived.bar_horizontal_padding = metrics.horizontal_padding;
}

/// Initialize WM configuration shared by nested and DRM/KMS Wayland modes.
///
/// Loads and applies the Wayland configuration, seeds fallback display
/// dimensions for pre-output initialization, and configures bar metrics.
/// Output discovery replaces the fallback dimensions and establishes monitor
/// geometry after the compositor backend is ready.
pub fn init_globals(state: &mut CoreState, wayland: &mut WaylandBackendData) {
    let cfg = load_startup_config(crate::backend::BackendKind::Wayland);
    state.derived.display.width = 1280;
    state.derived.display.height = 800;
    crate::core_state::apply_config(state, cfg);
    apply_bar_metrics(state, wayland);
}
