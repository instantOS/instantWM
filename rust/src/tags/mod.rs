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

use crate::contexts::WmCtx;

pub use bar::{get_tag_at_x, get_tag_width};

pub use naming::{name_tag, reset_name_tag};

pub use client_tags::{follow_tag, set_client_tag, tag_all, toggle_tag};

pub use view::{
    follow_view, last_view, shift_view, swap_tags, toggle_fullscreen_overview, toggle_overview,
    view, win_view,
};

// Re-export TagMask for convenience

pub use shift::{move_client, shift_tag_by};

pub use sticky::reset_sticky_win;

pub use tag_mon_impl::send_to_monitor;

pub fn compute_prefix(ctx: &mut WmCtx, arg: u32) -> u32 {
    let prefix_active = ctx.g.tags.prefix;
    if prefix_active && arg != 0 {
        ctx.g.tags.prefix = false;
        arg << 10
    } else {
        arg
    }
}

pub fn zoom(ctx: &mut WmCtx) {
    let selected_window = ctx.g.selected_monitor().and_then(|mon| mon.sel);
    if let Some(win) = selected_window {
        crate::client::pop(ctx, win);
    }
}

pub fn quit() {
    std::process::exit(0);
}
