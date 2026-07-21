//! Persistent presentation algorithms.
//!
//! Manual tiling is computed from [`crate::layouts::tree::LayoutTree`] in the
//! layout manager. Only presentations that are not tree geometry live here.

mod float;
mod maximized;

pub use float::floating;
pub use maximized::maximized;
