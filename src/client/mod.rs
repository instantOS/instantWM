//! Client window management.
//!
//! This module is the public surface for everything related to managing X11
//! client windows.  The implementation is split across focused sub-modules;
//! this file re-exports the public API so that callers can write
//! `crate::backend::x11::lifecycle::manage(...)` for X11 lifecycle details.
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
//! | `lifecycle`     | internal X11 lifecycle implementation details                  |
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
pub mod manager;
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
pub use constants::WM_STATE_WITHDRAWN;

// -- Linked-list management --------------------------------------------------
pub use list::{attach, attach_stack, detach, detach_stack};

// -- Geometry ----------------------------------------------------------------
pub use geometry::resize;

// -- Visibility --------------------------------------------------------------
pub use visibility::{hide, show, show_hide};

// -- Focus / input -----------------------------------------------------------
pub use focus::{
    configure_x11, refresh_border_color_x11, send_event_x11, set_focus_x11, unfocus_win_x11,
};

// -- Fullscreen --------------------------------------------------------------
pub use fullscreen::{set_fullscreen_x11, toggle_fake_fullscreen, toggle_fake_fullscreen_x11};

pub fn save_border_width(core: &mut crate::contexts::CoreCtx, win: crate::types::WindowId) {
    core.g.clients.save_border_width(win);
}

pub fn restore_border_width(core: &mut crate::contexts::CoreCtx, win: crate::types::WindowId) {
    core.g.clients.restore_border_width(win);
}

// -- X11 state / properties --------------------------------------------------
pub use state::{
    set_client_state, set_client_tag_prop, set_urgent, update_title_x11, update_wm_hints,
};

// -- Kill --------------------------------------------------------------------
pub use kill::{close_win, kill_client, shut_kill};

// -- Lifecycle ---------------------------------------------------------------
pub use lifecycle::initial_tags_for_monitor;

// -- Layout operations -------------------------------------------------------
pub use layout_ops::zoom;
