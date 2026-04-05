//! Sticky-client helpers.
//!
//! A "sticky" client is one that appears on every tag simultaneously.  When
//! such a client is moved to a specific tag (e.g. via a shift or monitor
//! transfer) it must lose its sticky status so it stops following every view.

use crate::contexts::CoreCtx;
use crate::types::{TagMask, WindowId};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Wrapper that resets sticky status when you only have a window ID.
///
/// This is useful when you need to reset sticky status but only have the window ID
/// and need to avoid borrow checker issues.
pub fn reset_sticky_win(core: &mut CoreCtx, win: WindowId) {
    // Extract data first to avoid borrow issues
    let mon = core.globals().selected_monitor();
    let target_tags = mon.current_tag.and_then(TagMask::single);

    if let Some(client) = core.globals_mut().clients.get_mut(&win)
        && client.issticky
    {
        client.issticky = false;
        if let Some(tags) = target_tags {
            client.set_tag_mask(tags);
        }
    }
}
