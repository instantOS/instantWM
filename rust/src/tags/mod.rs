//! Tag management — the complete public surface of `tags/`.

pub mod bar;
pub mod client_tags;
pub mod naming;
pub mod shift;
pub mod sticky;
pub mod view;

mod tag_mon_impl;

/// Type-safe tag operations with improved DX.
///
/// This module provides ergonomic wrappers using `TagMask` and `TagSelection`
/// types, offering better type safety and clearer semantics than raw `u32` bitmasks.
pub mod tag_ops;

// Re-export key types for convenience
pub use tag_ops::{ClientTagExt, TagViewBuilder};

use crate::globals::{get_globals, get_globals_mut};
use crate::util::get_sel_win;

pub use bar::{get_tag_at_x, get_tag_width};

pub use naming::{name_tag, reset_name_tag};

pub use client_tags::{follow_tag, set_client_tag, tag_all, toggle_tag};

pub use view::{
    follow_view, last_view, shift_view, swap_tags, toggle_fullscreen_overview, toggle_overview,
    toggle_view, view, view_to_left, view_to_right, win_view,
};

// Re-export TagMask for convenience
pub use crate::types::TagMask;

pub use shift::{move_left, move_right, tag_to_left, tag_to_right};

pub use sticky::reset_sticky;

pub use tag_mon_impl::tag_mon;

pub fn compute_prefix(arg: u32) -> u32 {
    let prefix_active = get_globals().tags.prefix;
    if prefix_active && arg != 0 {
        get_globals_mut().tags.prefix = false;
        arg << 10
    } else {
        arg
    }
}

pub fn zoom() {
    if let Some(win) = get_sel_win() {
        crate::client::pop(win);
    }
}

pub fn quit() {
    std::process::exit(0);
}
