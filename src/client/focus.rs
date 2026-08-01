//! Shared focus state (no X11 types).

use crate::types::WindowId;

/// Non-backend-specific focus tracking state.
#[derive(Default)]
pub struct FocusState {
    /// The previously focused window (0 = none), used by focus-last-client logic.
    pub last_client: WindowId,
}
