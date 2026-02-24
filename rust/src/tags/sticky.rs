//! Sticky-client helpers.
//!
//! A "sticky" client is one that appears on every tag simultaneously.  When
//! such a client is moved to a specific tag (e.g. via a shift or monitor
//! transfer) it must lose its sticky status so it stops following every view.
//!
//! This module is intentionally small — it contains only the one function that
//! operates on an already-borrowed `&mut Client` without touching the global
//! state machine, which is why it lives separately from [`super::shift`].

use crate::contexts::WmCtx;
use crate::types::Client;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Clear the sticky flag on `c` and pin it to the monitor's currently active
/// tag.
///
/// This should be called whenever a sticky client is about to be assigned to a
/// specific tag (e.g. during a tag shift or a monitor transfer) so that it
/// stops appearing on every tag on its new home monitor.
///
/// If `c.issticky` is already `false` this is a no-op.
pub fn reset_sticky(ctx: &mut WmCtx, c: &mut Client) {
    if !c.issticky {
        return;
    }

    c.issticky = false;

    if let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) {
        if mon.current_tag > 0 {
            c.tags = 1 << (mon.current_tag - 1);
        }
    }
}
