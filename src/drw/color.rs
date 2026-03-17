//! Color and cursor types used by the drawing context.
//!
//! [`Color`] wraps an `XftColor` (a pixel value + 16-bit RGBA components).
//! [`Cursor`] wraps an X11 cursor id created via `XCreateFontCursor`.

use super::ffi::{XRenderColor, XftColor};
use std::os::raw::c_ulong;

// ── Color indices into a color scheme ────────────────────────────────────────

/// Index of the foreground color within a scheme slice.
pub const COL_FG: usize = 0;
/// Index of the background color within a scheme slice.
pub const COL_BG: usize = 1;
/// Index of the detail / accent color within a scheme slice.
pub const COL_DETAIL: usize = 2;

// ── Color ──────────────────────────────────────────────────────────────────────

/// A single allocated X11/Xft color.
///
/// Cheaply cloneable — the underlying pixel value is just a `u64` and the
/// `XftColor` is a plain-old-data C struct.
#[derive(Debug, Clone)]
pub struct Color {
    pub color: XftColor,
}

impl PartialEq for Color {
    fn eq(&self, other: &Self) -> bool {
        self.color.pixel == other.color.pixel
    }
}

// SAFETY: instantWM is single-threaded; the pixel value is just an integer.
unsafe impl Send for Color {}
unsafe impl Sync for Color {}

impl Default for Color {
    fn default() -> Self {
        Self {
            color: XftColor {
                pixel: 0,
                color: XRenderColor {
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 0xFFFF,
                },
            },
        }
    }
}

impl Color {
    /// Returns the 32-bit pixel value (suitable for passing to Xlib draw calls).
    pub fn pixel(&self) -> u32 {
        self.color.pixel as u32
    }
}

// ── Cursor ────────────────────────────────────────────────────────────────────

/// A loaded X11 cursor (created via `XCreateFontCursor`).
#[derive(Debug, Clone)]
pub struct Cursor {
    pub cursor: c_ulong,
}

// SAFETY: cursor ids are just integers; instantWM is single-threaded.
unsafe impl Send for Cursor {}
unsafe impl Sync for Cursor {}

impl Cursor {
    pub fn new(cursor: c_ulong) -> Self {
        Self { cursor }
    }
}
