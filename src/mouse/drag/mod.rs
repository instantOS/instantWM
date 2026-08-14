//! Interactive mouse-drag operations.
//!
//! This module is split into focused sub-modules:
//!
//! - [`interactive`] — Backend-neutral active window move/resize lifecycle
//! - [`move_drop`] — Shared bar hover, edge snap, and drop completion
//! - [`tag`] — Tag bar drag: [`apply_drag_tag_motion`], [`drag_tag_begin`], [`drag_tag_finish`]
//! - [`title`] — Title bar click/drag: [`title_drag_begin`], [`process_title_drag_motion`],
//!   [`title_drag_finish`], [`handle_window_title_mouse`], and the bar-title
//!   strip reorder ([`process_title_reorder_motion`], [`title_reorder_finish`])
//! - [`gesture`] — Sidebar and bottom-bar gestures: [`sidebar_gesture_begin`],
//!   [`bottom_bar_gesture_begin`]
//!
//! Native backends only acquire input and translate it to
//! [`crate::mouse::interaction::InteractionEvent`]. Recognition, mutation,
//! cursor policy, and cleanup live in this shared subsystem.

// Re-export from submodules
pub use gesture::{
    bottom_bar_gesture_begin, bottom_bar_gesture_finish, sidebar_gesture_begin,
    sidebar_gesture_finish, update_bottom_bar_gesture, update_sidebar_gesture,
};
pub use move_drop::{
    clear_bar_hover, complete_move_drop, snap_window_to_monitor_edges, update_bar_hover_simple,
};
pub use tag::{apply_drag_tag_motion, drag_tag_begin, drag_tag_finish};
pub use title::{
    DragInput, begin_thresholded_client_drag, handle_window_title_mouse, process_title_drag_motion,
    process_title_reorder_motion, title_drag_finish, title_reorder_finish,
};

use crate::contexts::WmCtx;
use crate::types::*;

// Submodules
pub mod gesture;
pub mod interactive;
pub mod lifecycle;
pub mod move_drop;
pub mod tag;
pub mod title;

/// Resolve a root-space point against one monitor's bar.
///
/// Returns `None` when the monitor is absent, its bar is hidden, or the point
/// is outside the bar. Shared by the tag and title-strip drag gestures so both
/// resolve hover targets identically.
pub(crate) fn bar_position_on_monitor(
    ctx: &WmCtx<'_>,
    monitor_id: MonitorId,
    root: Point,
) -> Option<BarPosition> {
    let local_x = bar_local_x_on_monitor(ctx, monitor_id, root)?;
    let core = ctx.core();
    let monitor = core.model().monitor(monitor_id)?;
    Some(monitor.bar_position_at_x(core, local_x))
}

/// Validate a root-space point against one monitor's visible bar and return
/// its monitor-local x coordinate.
pub(crate) fn bar_local_x_on_monitor(
    ctx: &WmCtx<'_>,
    monitor_id: MonitorId,
    root: Point,
) -> Option<i32> {
    let core = ctx.core();
    let monitor = core.model().monitor(monitor_id)?;
    let mask = monitor.selected_tags();
    if !monitor.show_bar_for_mask(mask)
        || !monitor.y_in_bar(root.y)
        || root.x < monitor.monitor_rect.x
        || root.x >= monitor.monitor_rect.right()
    {
        return None;
    }
    Some(monitor.local_work_point(root).x)
}

pub use interactive::{
    active_drag_finish, apply_active_drag_motion, directional_resize_begin,
    directional_resize_begin_with_policy, hover_drag_begin, tree_resize_begin,
};

/// Shared post-move-drag teardown used by both X11 and Wayland backends.
///
/// Restores the bar hover highlight and runs the shared drop-completion logic
/// (bar drop, edge snap, monitor switch). The caller must finish the
/// interaction lifecycle before invoking this cleanup.
pub fn drag_move_finish(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    edge_hint: Option<SnapPosition>,
    pointer_override: Option<Point>,
    modifiers: u32,
) {
    debug_assert!(!ctx.core().drag_state().has_capture());
    ctx.set_cursor_style(crate::types::AltCursor::Default);
    clear_bar_hover(ctx);
    complete_move_drop(
        ctx,
        win,
        grab_start_rect,
        edge_hint,
        pointer_override,
        modifiers,
    );
}

/// Shared post-resize-drag teardown used by both X11 and Wayland backends.
///
/// Resets the cursor to the default, handles a potential monitor switch, and
/// re-raises the client. The caller must finish the interaction lifecycle
/// before invoking this cleanup.
pub fn drag_resize_finish(ctx: &mut WmCtx, win: WindowId) {
    debug_assert!(!ctx.core().drag_state().has_capture());
    ctx.set_cursor_style(crate::types::AltCursor::Default);
    crate::mouse::monitor::handle_client_monitor_switch(ctx, win);
    ctx.raise_client(win);
}
