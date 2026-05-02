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

pub use bar::get_tag_width;

pub use naming::{name_tag, reset_name_tag};

pub use view::{cancel_overview, follow_view, last_view, shift_view, toggle_overview, win_view};

// Re-export TagMask for convenience

pub use shift::{move_client, shift_tag};

pub use tag_mon_impl::send_to_monitor;

pub fn quit() {
    std::process::exit(0);
}
