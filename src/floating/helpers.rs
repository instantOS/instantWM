//! Small stateless helper functions used throughout the floating module.
//!
//! None of these functions mutate floating state – they only inspect it.

use crate::model::WmModel;

// ── Layout query ─────────────────────────────────────────────────────────────

/// Returns `true` if the currently selected monitor has a tiling layout active.
///
/// Used as a guard throughout the floating module: floating-only operations
/// should be no-ops when a tiling layout is active and the window is not
/// explicitly floating.
pub fn has_tiling_layout(model: &WmModel) -> bool {
    model.expect_selected_monitor().is_tiling_layout()
}
