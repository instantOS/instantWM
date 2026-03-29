//! Client window management.
//!
//! This module is the public surface for generic client/window management.
//! X11-specific property plumbing lives under `crate::backend::x11`.
//!
//! # Sub-module map
//!
//! | Module          | Responsibility                                              |
//! |-----------------|-------------------------------------------------------------|
//! | `constants`     | WM_STATE, MWM hints, WM_HINTS, XSizeHints constants         |
//! | `geometry`      | Resize, size-hint enforcement, dimension helpers            |
//! | `visibility`    | Show / hide / show_hide, WM_STATE queries                   |
//! | `focus`         | Input focus, button grabs, ConfigureNotify, ClientMessage   |
//! | `fullscreen`    | Real and fake fullscreen transitions                        |
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
pub mod manager;
pub mod rules;
pub mod visibility;
pub mod x11_policy;

// ---------------------------------------------------------------------------
// Flat re-exports
//
// Only items actually imported from outside the `client` module are listed
// here.  Internal cross-module references use their direct paths
// (e.g. `crate::client::geometry::resize`) so they don't need to appear here.
// ---------------------------------------------------------------------------

// -- Rules ------------------------------------------------------------------
pub use rules::{WindowProperties, apply_rules, handle_property_change};

// -- Constants ---------------------------------------------------------------
pub use constants::WM_STATE_WITHDRAWN;

// -- Geometry ----------------------------------------------------------------
pub use geometry::{resize, sync_client_geometry};

// -- Visibility --------------------------------------------------------------
pub use visibility::{hide, hide_for_user, show, show_hide};

// -- Focus / input -----------------------------------------------------------
pub use focus::{
    clear_urgency_hint_x11, configure_x11, refresh_border_color_x11, send_event_x11, set_focus_x11,
    unfocus_win_x11,
};

// -- Fullscreen --------------------------------------------------------------
pub use fullscreen::{set_fullscreen_x11, toggle_fake_fullscreen_x11};

pub fn save_border_width(client: &mut crate::types::Client) {
    client.save_border_width();
}

pub fn restore_border_width(client: &mut crate::types::Client) {
    client.restore_border_width();
}

// -- Kill --------------------------------------------------------------------
pub use kill::{close_win, kill_client, shut_kill};

// -- Lifecycle ---------------------------------------------------------------
pub use lifecycle::{
    LaunchContext, PendingLaunch, current_launch_context, initial_tags_for_monitor, new_startup_id,
    record_pending_launch, select_client, take_pending_launch,
};

// -- Layout operations -------------------------------------------------------
pub use layout_ops::zoom;
