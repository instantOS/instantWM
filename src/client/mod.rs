//! Client window management.
//!
//! This module is the public surface for generic client/window management.
//! X11-specific property plumbing lives under `crate::backend::x11`.
//!
//! # Sub-module map
//!
//! | Module          | Responsibility                                              |
//! |-----------------|-------------------------------------------------------------|
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

pub mod focus;
pub mod fullscreen;
pub mod geometry;
pub mod kill;
pub mod layout_ops;
pub mod lifecycle;
pub mod mode;
pub mod rules;
pub mod visibility;

// ---------------------------------------------------------------------------
// Flat re-exports
//
// Only items actually imported from outside the `client` module are listed
// here.  Internal cross-module references use their direct paths
// (e.g. `crate::client::geometry::apply_size_hints`) so they don't need to
// appear here.
// ---------------------------------------------------------------------------

// -- Rules ------------------------------------------------------------------
pub use rules::{
    InitialRulePlacement, WindowProperties, apply_initial_rules, handle_property_change,
};

// -- Geometry ----------------------------------------------------------------
pub use geometry::{sane_floating_spawn_rect, sync_client_geometry};

// -- Visibility --------------------------------------------------------------
pub use visibility::{apply_visibility, hide, hide_for_user, show_window};

// -- Focus / input -----------------------------------------------------------
// X11-specific focus functions live in `client::focus` and are called
// directly via `crate::client::focus::*` rather than re-exported here.

// -- Fullscreen --------------------------------------------------------------
pub use fullscreen::set_fullscreen;

// -- Kill --------------------------------------------------------------------
pub use kill::{close_win, kill_client, shut_kill};

// -- Lifecycle ---------------------------------------------------------------
pub use lifecycle::{
    LaunchContext, PendingLaunch, current_launch_context, new_startup_id, record_pending_launch,
    select_client, take_pending_launch,
};

// -- Layout operations -------------------------------------------------------
pub use layout_ops::zoom;
