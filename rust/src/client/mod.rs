//! Client window management.
//!
//! This module is the public surface for everything related to managing X11
//! client windows.  The implementation is split across focused sub-modules;
//! this file re-exports the public API so that callers can write
//! `crate::client::manage(...)` instead of
//! `crate::client::lifecycle::manage(...)`.
//!
//! # Sub-module map
//!
//! | Module          | Responsibility                                              |
//! |-----------------|-------------------------------------------------------------|
//! | `constants`     | WM_STATE, MWM hints, WM_HINTS, XSizeHints constants         |
//! | `list`          | Intrusive linked-list helpers (attach / detach / traverse)  |
//! | `geometry`      | Resize, size-hint enforcement, dimension helpers            |
//! | `visibility`    | Show / hide / show_hide, WM_STATE queries                   |
//! | `focus`         | Input focus, button grabs, ConfigureNotify, ClientMessage   |
//! | `fullscreen`    | Real and fake fullscreen transitions                        |
//! | `state`         | X11 property read/write (titles, rules, hints, lists)       |
//! | `kill`          | Graceful and forceful window termination                    |
//! | `lifecycle`     | manage() / unmanage() – adopting and releasing windows      |
//! | `layout_ops`    | zoom (promote to master)                                    |

// ---------------------------------------------------------------------------
// Sub-modules
// ---------------------------------------------------------------------------

pub mod constants;
pub mod focus;
pub mod fullscreen;
pub mod geometry;
pub mod kill;
pub mod layout_ops;
pub mod lifecycle;
pub mod list;
pub mod state;
pub mod visibility;

// ---------------------------------------------------------------------------
// Flat re-exports
//
// Only items actually imported from outside the `client` module are listed
// here.  Internal cross-module references use their direct paths
// (e.g. `crate::client::geometry::resize`) so they don't need to appear here.
// ---------------------------------------------------------------------------

// -- Constants ---------------------------------------------------------------
pub use constants::{WM_STATE_ICONIC, WM_STATE_WITHDRAWN};

// -- Linked-list management --------------------------------------------------
pub use list::{
    attach_ctx, attach_stack_ctx, detach_ctx, detach_stack_ctx, next_tiled_ctx, pop, win_to_client,
};

// -- Geometry ----------------------------------------------------------------
pub use geometry::{
    apply_size_hints, client_height, client_width, resize, resize_client, scale_client,
    update_size_hints_win,
};

// -- Visibility --------------------------------------------------------------
pub use visibility::{hide, is_hidden, show, show_hide};

// -- Focus / input -----------------------------------------------------------
pub use focus::{configure, send_event, set_focus, unfocus_win};

// -- Fullscreen --------------------------------------------------------------
pub use fullscreen::{
    restore_border_width, restore_border_width_ctx, save_border_width, save_border_width_ctx,
    set_fullscreen, toggle_fake_fullscreen,
};

// -- X11 state / properties --------------------------------------------------
pub use state::{set_client_state, set_client_tag_prop, set_urgent, update_title, update_wm_hints};

// -- Kill --------------------------------------------------------------------
pub use kill::{close_win, kill_client, selected_window, shut_kill};

// -- Lifecycle ---------------------------------------------------------------
pub use lifecycle::{get_transient_for_hint, manage, unmanage};

// -- Layout operations -------------------------------------------------------
pub use layout_ops::zoom;
