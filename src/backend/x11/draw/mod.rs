//! Drawing context for the instantWM status bar.
//!
//! This module is the public face of everything rendering-related.
//! Internal implementation is split across focused sub-modules:
//!
//! | Sub-module | Contents                                               |
//! |------------|--------------------------------------------------------|
//! | [`ffi`]    | Raw `extern "C"` bindings (X11, Xft, Fontconfig)      |
//! | [`color`]  | [`Color`] (color) and [`Cursor`] (cursor)              |
//! | [`font`]   | [`Fnt`] font / fontset linked-list                     |
//! |            |                                                       |
//! | [`draw`]   | [`Drw`] drawing context — the main public type         |
//!
//! # Typical usage
//!
//! ```ignore
//! // Create a drawing context (opens the X display).
//! let mut drw = Drw::new(None)?;
//!
//! // Load fonts.
//! drw.fontset_create(&["monospace:size=10", "Noto Color Emoji:size=10"])?;
//!
//! // Allocate a color scheme and activate it.
//! let scheme = drw.scm_create(&["#eeeeee", "#222222", "#005577"])?;
//! drw.set_scheme(ColorScheme::from_vec(scheme));
//!
//! // Draw text into the off-screen pixmap…
//! drw.text(0, 0, 200, bar_height, horizontal_padding as u32 / 2, "Hello, world!", false, 0);
//!
//! // …then blit it to the bar window.
//! drw.map(bar_win, 0, 0, 200, bar_height as u16);
//! ```

// Sub-modules — ffi is pub(crate) so other modules can reach raw bindings if
// absolutely necessary; the rest are private implementation details.
pub(crate) mod ffi;

mod color;
mod draw;
mod font;

// ── Public re-exports ─────────────────────────────────────────────────────────
//
// Everything below forms the *stable* public API of this module.  Callers
// should import from `crate::backend::x11::draw::*` rather than from the sub-modules
// directly.

// Color / cursor types.
pub use color::{Color, Cursor};

// The main drawing context.
pub use draw::Drw;

// Raw FFI symbols used externally.
pub use ffi::{XEventsQueued, XFlush};
