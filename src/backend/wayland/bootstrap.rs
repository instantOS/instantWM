//! Backend-neutral Wayland WM initialization.

use crate::config::load_startup_config;
use crate::core_state::CoreState;

// ─────────────────────────────────────────────────────────────────────────────
// WM globals initialisation
// ─────────────────────────────────────────────────────────────────────────────

/// Initialize WM configuration shared by nested and DRM/KMS Wayland modes.
///
/// Loads and applies the Wayland configuration, seeds fallback display
/// dimensions for pre-output initialization. Bar metrics are derived from
/// configuration when output monitors are established.
/// Output discovery replaces the fallback dimensions and establishes monitor
/// geometry after the compositor backend is ready.
pub fn init_globals(state: &mut CoreState) {
    let cfg = load_startup_config(crate::backend::BackendKind::Wayland);
    state.derived.display.width = 1280;
    state.derived.display.height = 800;
    state
        .apply_config(cfg)
        .expect("startup tag state must be valid");
}
