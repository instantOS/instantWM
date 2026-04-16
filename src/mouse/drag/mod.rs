//! Interactive mouse-drag operations.
//!
//! This module is split into focused sub-modules:
//!
//! - [`move_drop`] — Core move/drop logic: [`MoveState`], bar hover, edge snap,
//!   [`prepare_drag_target`], [`complete_move_drop`]
//! - [`tag`] — Tag bar drag: [`drag_tag_begin`], [`drag_tag_motion`], [`drag_tag_finish`]
//! - [`title`] — Title bar click/drag: [`title_drag_begin`], [`title_drag_motion`],
//!   [`title_drag_finish`], [`window_title_mouse_handler`]
//! - [`gesture`] — Root-window gestures: [`gesture_mouse`]
//!
//! | Function                            | Description                                               |
//! |-------------------------------------|-----------------------------------------------------------|
//! | [`begin_keyboard_move`]             | Keyboard-initiated window drag (works on X11 and Wayland) |
//! | [`crate::backend::x11::mouse::move_mouse_x11`] | Drag the focused window to a new position (X11 only) |
//! | [`gesture_mouse`]                   | Vertical-swipe gesture recogniser on the root window      |
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
pub use gesture::gesture_mouse;
pub use move_drop::{
    MoveState, clear_bar_hover, complete_move_drop, on_motion, prepare_drag_target,
    snap_window_to_monitor_edges, update_bar_hover_simple,
};
pub use tag::{drag_tag, drag_tag_begin, drag_tag_finish, drag_tag_motion};
pub use title::{
    title_drag_begin, title_drag_finish, title_drag_motion, window_title_mouse_handler,
};

use crate::contexts::WmCtx;
use crate::floating::{WindowMode, set_window_mode};
use crate::types::*;

// Submodules
pub mod gesture;
pub mod move_drop;
pub mod tag;
pub mod title;

/// Keyboard-initiated window move — works on both X11 and Wayland.
///
/// On **X11** this is identical to calling [`crate::backend::x11::mouse::move_mouse_x11`]
/// directly: the pointer is grabbed and a synchronous event loop drives the drag
/// until the button is released.
///
/// On **Wayland** a synchronous grab loop is not possible (no `XGrabPointer`
/// equivalent in the protocol).  Instead we arm the `DragInteraction`
/// machinery in move mode at the current pointer
/// position.  Subsequent `MotionNotify` events delivered through calloop then
/// drive the drag, and `wayland_hover_resize_drag_finish` (called on button
/// release inside `handle_pointer_button`) performs the drop logic via the
/// shared `complete_move_drop` helper.
///
/// The button used to end the drag defaults to `MouseButton::Left` on Wayland
/// (matching the most common keyboard-move UX on other compositors).
pub fn begin_keyboard_move(ctx: &mut WmCtx) {
    // Pre-flight checks are shared: exit maximized state, un-snap, etc.
    let Some(win) = prepare_drag_target(ctx) else {
        return;
    };

    match ctx {
        WmCtx::X11(x11) => {
            // X11: synchronous grab loop, unchanged behaviour.
            crate::backend::x11::mouse::move_mouse_x11(x11, MouseButton::Left, None);
        }
        WmCtx::Wayland(wl) => {
            // Wayland: arm the hover-resize state in move mode so that calloop
            // motion/release events drive the drag asynchronously.
            let Some((root_x, root_y)) = wl.wayland.backend.pointer_location() else {
                return;
            };
            let (geo, is_floating) = match wl.core.client(win) {
                Some(c) => (c.geo, c.is_floating),
                None => return,
            };

            // Ensure the window is floating so the move makes sense.
            if !is_floating {
                set_window_mode(
                    &mut WmCtx::Wayland(wl.reborrow()),
                    win,
                    WindowMode::Floating,
                );
                let selmon_id = wl.core.globals().selected_monitor_id();
                crate::layouts::arrange(&mut WmCtx::Wayland(wl.reborrow()), Some(selmon_id));
            }

            wl.core.globals_mut().drag.interactive = crate::globals::DragInteraction {
                active: true,
                win,
                button: MouseButton::Left,
                dragging: true,
                drag_type: crate::globals::DragType::Move,
                start_x: root_x,
                start_y: root_y,
                win_start_geo: geo,
                drop_restore_geo: geo,
                last_root_x: root_x,
                last_root_y: root_y,
                ..Default::default()
            };
            crate::mouse::set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(wl.reborrow()),
                crate::types::AltCursor::Move,
            );
            crate::contexts::WmCtx::Wayland(wl.reborrow()).raise_client(win);
        }
    }
}
