//! Interactive mouse-drag operations.
//!
//! This module is split into focused sub-modules:
//!
//! - [`move_drop`] — Core move/drop logic: [`MoveState`], bar hover, edge snap,
//!   [`prepare_drag_target`], [`complete_move_drop`]
//! - [`tag`] — Tag bar drag: [`drag_tag_begin`], [`drag_tag_motion`], [`drag_tag_finish`]
//! - [`title`] — Title bar click/drag: [`title_drag_begin`], [`title_drag_motion`],
//!   [`title_drag_finish`], [`window_title_mouse_handler`]
//! - [`gesture`] — Sidebar gestures: [`sidebar_gesture_begin`]
//!
//! | Function                            | Description                                               |
//! |-------------------------------------|-----------------------------------------------------------|
//! | [`crate::backend::x11::mouse::move_mouse`] | Drag the focused window to a new position (X11 only) |
//! | [`sidebar_gesture_begin`]           | Vertical-swipe gesture recogniser on the sidebar edge     |
//! | [`drag_tag`]                        | Drag across the tag bar to switch/move tags               |
//! | [`window_title_mouse_handler`]      | Left-click/drag on a window title bar entry               |
//!
//! All loops follow the same skeleton:
//!
//! ```text
//! grab_pointer(cursor)
//! loop {
//!     ButtonRelease → break
//!     MotionNotify  → throttle → update
//!     _             → ignore
//! }
//! ungrab(ctx)
//! post-loop cleanup (bar drop, monitor switch, bar redraw, …)
//! ```

// Re-export from submodules
pub use gesture::{finish_sidebar_gesture, sidebar_gesture_begin, update_sidebar_gesture};
pub use move_drop::{
    MoveState, clear_bar_hover, complete_move_drop, on_motion, prepare_drag_target,
    snap_window_to_monitor_edges, update_bar_hover_simple,
};
pub use tag::{drag_tag, drag_tag_begin, drag_tag_finish, drag_tag_motion};
pub use title::{
    thresholded_client_drag, title_drag_finish, title_drag_motion, window_title_mouse_handler,
};

use crate::contexts::WmCtx;
use crate::types::*;

// Submodules
pub mod gesture;
pub mod lifecycle;
pub mod move_drop;
pub mod tag;
pub mod title;

/// Shared post-move-drag teardown used by both X11 and Wayland backends.
///
/// Restores the bar hover highlight and runs the shared drop-completion logic
/// (bar drop, edge snap, monitor switch). The caller must finish the
/// interaction lifecycle before invoking this cleanup.
pub fn finish_drag_move(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    edge_hint: Option<SnapPosition>,
    pointer_override: Option<Point>,
    modifiers: u32,
) {
    debug_assert!(ctx.core().drag_state().interactive().is_idle());
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
pub fn finish_drag_resize(ctx: &mut WmCtx, win: WindowId) {
    debug_assert!(ctx.core().drag_state().interactive().is_idle());
    ctx.set_cursor_style(crate::types::AltCursor::Default);
    crate::mouse::monitor::handle_client_monitor_switch(ctx, win);
    ctx.raise_client(win);
}
