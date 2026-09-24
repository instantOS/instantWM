//! Tag management — the complete public surface of `tags/`.

pub mod bar;
pub mod client_tags;
pub mod naming;
pub mod shift;
pub mod sticky;
pub mod view;

mod send_mon_impl;

pub use naming::{name_tag, reset_name_tag};

pub use view::{follow_view, last_view, shift_view, win_view};

pub use crate::overview::{cancel_overview, toggle_overview};

pub use shift::{move_client_follow_view, shift_tag};

pub use send_mon_impl::send_to_monitor;
