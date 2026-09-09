//! Backend-neutral Wayland WM initialization.

use crate::config::load_startup_config;
use crate::core_state::CoreState;

// ─────────────────────────────────────────────────────────────────────────────
// WM globals initialisation
// ─────────────────────────────────────────────────────────────────────────────

/// Apply font-derived bar metrics to the runtime config.
///
/// Computes `bar_height` and `horizontal_padding` from the font config and
/// applies them to the given `CoreState`. The raster worker receives the full
/// per-monitor font configuration in each render snapshot. Shared by both
/// startup (`init_globals`) and reload.
pub fn apply_bar_metrics(state: &mut CoreState) {
    let metrics = state.config.fonts.bar_metrics(state.config.bar.height);

    state.derived.bar_height = metrics.height;
    state.derived.bar_horizontal_padding = metrics.horizontal_padding;
}

/// Initialize WM configuration shared by nested and DRM/KMS Wayland modes.
///
/// Loads and applies the Wayland configuration, seeds fallback display
/// dimensions for pre-output initialization, and configures bar metrics.
/// Output discovery replaces the fallback dimensions and establishes monitor
/// geometry after the compositor backend is ready.
pub fn init_globals(state: &mut CoreState) {
    let cfg = load_startup_config(crate::backend::BackendKind::Wayland);
    state.derived.display.width = 1280;
    state.derived.display.height = 800;
    crate::core_state::apply_config(state, cfg);
    apply_bar_metrics(state);
}
