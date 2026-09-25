//! X11 keysym constants.
//!
//! The constants and the [`Keysym`] type they belong to now live in
//! [`crate::types::keysym`], next to the name resolution and binding
//! normalization that give them meaning.
//!
//! This module stays as the config-facing path so the binding tables and
//! user-facing docs can keep saying `keysyms::XK_RETURN` without caring which
//! module happens to own the value.
//!
//! ```ignore
//! use crate::config::keysyms::*;
//! ```

pub use crate::types::keysym::*;
