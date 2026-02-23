//! Tag management — the complete public surface of `tags/`.
//!
//! This module is the single import target for the rest of the codebase.
//! Everything under `tags/` is kept in focused sub-modules; this file
//! re-exports the full public API so callers never need to know the internal
//! layout:
//!
//! ```text
//! use crate::tags::{view, toggle_tag, tag_to_left, …};
//! ```
//!
//! # Sub-module map
//!
//! | Sub-module | Responsibility |
//! |---|---|
//! | [`bar`] | Pixel width / hit-test helpers consumed by bar rendering. |
//! | [`naming`] | Runtime tag-name assignment and reset. |
//! | [`client_tags`] | Assigning / toggling a client's tag membership. |
//! | [`view`] | Changing which tags are visible on a monitor. |
//! | [`shift`] | Sliding a client to an adjacent tag (with optional animation). |
//!
//! # Shared helpers
//!
//! [`compute_prefix`] lives here because it sits between the keybinding layer
//! and every tag-mutating operation — it is not specific to any one sub-module.

pub mod bar;
pub mod client_tags;
pub mod naming;
pub mod shift;
pub mod sticky;
pub mod view;

mod tag_mon_impl;

use crate::globals::{get_globals, get_globals_mut};
use crate::types::Arg;
use crate::util::get_sel_win;

// ---------------------------------------------------------------------------
// Re-exports — bar helpers
// ---------------------------------------------------------------------------

pub use bar::{get_tag_at_x, get_tag_width};

// ---------------------------------------------------------------------------
// Re-exports — tag naming
// ---------------------------------------------------------------------------

pub use naming::{name_tag, reset_name_tag};

// ---------------------------------------------------------------------------
// Re-exports — client-tag assignment
// ---------------------------------------------------------------------------

pub use client_tags::{follow_tag, set_client_tag, tag_all, toggle_tag};

// ---------------------------------------------------------------------------
// Re-exports — view navigation
// ---------------------------------------------------------------------------

pub use view::{
    follow_view, last_view, shift_view, swap_tags, toggle_fullscreen_overview, toggle_overview,
    toggle_view, view, view_to_left, view_to_right, win_view,
};

// ---------------------------------------------------------------------------
// Re-exports — tag shifting
// ---------------------------------------------------------------------------

pub use shift::{move_left, move_right, tag_to_left, tag_to_right};

// ---------------------------------------------------------------------------
// Re-exports — sticky reset (used by monitor.rs)
// ---------------------------------------------------------------------------

pub use sticky::reset_sticky;

// ---------------------------------------------------------------------------
// Re-exports — tag_mon (send client to another monitor)
// ---------------------------------------------------------------------------

pub use tag_mon_impl::tag_mon;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Apply the prefix modifier to `arg.ui` and return the resulting bitmask.
///
/// In *prefix mode* (`globals.tags.prefix == true`) the value in `arg.ui` is
/// shifted left by 10 bits — this is how instantWM's prefix-key feature maps
/// a single digit key to a higher-numbered tag set.  Prefix mode is
/// automatically cleared after this call so the next key press behaves
/// normally again.
///
/// If prefix mode is inactive the value is returned unchanged.
///
/// # Example
///
/// ```ignore
/// // In prefix mode, pressing "1" (arg.ui = 1) targets tag 11 (1 << 10 = 1024).
/// let mask = compute_prefix(&Arg { ui: 1, ..Default::default() });
/// ```
pub fn compute_prefix(arg: &Arg) -> u32 {
    let prefix_active = get_globals().tags.prefix;
    if prefix_active && arg.ui != 0 {
        get_globals_mut().tags.prefix = false;
        arg.ui << 10
    } else {
        arg.ui
    }
}

// ---------------------------------------------------------------------------
// tag() — canonical alias
//
// The C codebase called this function `tag()`.  Keybindings, the mouse drag
// code and external callers still use that name.  We keep it as a thin alias
// so every call site compiles unchanged, but it delegates entirely to the
// better-named `set_client_tag`.
// ---------------------------------------------------------------------------

/// Assign `arg.ui` as the sole tag(s) for the currently selected client.
///
/// This is a named alias for [`set_client_tag`] that preserves compatibility
/// with call sites ported directly from the C codebase.  Prefer
/// [`set_client_tag`] in new code.
#[inline]
pub fn tag(arg: &Arg) {
    set_client_tag(arg);
}

// ---------------------------------------------------------------------------
// desktop_set — stub
// ---------------------------------------------------------------------------

/// No-op stub retained for keybinding compatibility.
///
/// The C implementation was also a no-op in most configurations.
pub fn desktop_set(_arg: &Arg) {}

// ---------------------------------------------------------------------------
// zoom — delegate to client::pop
// ---------------------------------------------------------------------------

/// Promote the selected window to the top of the tiling stack (zoom).
///
/// Delegates to [`crate::client::pop`].  Lives here only because the original
/// C `tag.c` file contained it; new code should call `client::pop` directly.
pub fn zoom(_arg: &Arg) {
    if let Some(win) = get_sel_win() {
        crate::client::pop(win);
    }
}

// ---------------------------------------------------------------------------
// quit
// ---------------------------------------------------------------------------

/// Terminate the window manager immediately.
///
/// Registered as both a keybinding and an X-command entry point.
pub fn quit(_arg: &Arg) {
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// X-command entry points
//
// `commands.rs` stores function pointers for commands received over the
// `instantwmctl` socket.  These one-line wrappers give those entries a stable
// symbol name even if the underlying function is ever renamed.
// ---------------------------------------------------------------------------

/// X-command entry point: assign the current client to a tag.
#[inline]
pub fn command_tag(arg: &Arg) {
    set_client_tag(arg);
}

/// X-command entry point: switch the view to a tag.
#[inline]
pub fn command_view(arg: &Arg) {
    view(arg);
}

/// X-command entry point: toggle a tag's membership in the current view.
#[inline]
pub fn command_toggle_view(arg: &Arg) {
    toggle_view(arg);
}

/// X-command entry point: toggle a tag on the current client.
#[inline]
pub fn command_toggle_tag(arg: &Arg) {
    toggle_tag(arg);
}
