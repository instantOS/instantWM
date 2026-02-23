//! Color and cursor types used by the drawing context.
//!
//! [`Clr`] wraps an `XftColor` (a pixel value + 16-bit RGBA components).
//! [`Cur`] wraps an X11 cursor id created via `XCreateFontCursor`.

use super::ffi::{XRenderColor, XftColor};

// ── Color indices into a color scheme ────────────────────────────────────────

/// Index of the foreground color within a scheme slice.
pub const COL_FG: usize = 0;
/// Index of the background color within a scheme slice.
pub const COL_BG: usize = 1;
/// Index of the detail / accent color within a scheme slice.
pub const COL_DETAIL: usize = 2;
/// Total number of color slots in a scheme.
pub const COL_LAST: usize = 3;

// ── Clr ──────────────────────────────────────────────────────────────────────

/// A single allocated X11/Xft color.
///
/// Cheaply cloneable — the underlying pixel value is just a `u64` and the
/// `XftColor` is a plain-old-data C struct.
#[derive(Debug, Clone)]
pub struct Clr {
    pub color: XftColor,
}

impl PartialEq for Clr {
    fn eq(&self, other: &Self) -> bool {
        self.color.pixel == other.color.pixel
    }
}

// SAFETY: instantWM is single-threaded; the pixel value is just an integer.
unsafe impl Send for Clr {}
unsafe impl Sync for Clr {}

impl Default for Clr {
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

impl Clr {
    /// Returns the 32-bit pixel value (suitable for passing to Xlib draw calls).
    pub fn pixel(&self) -> u32 {
        self.color.pixel as u32
    }
}

// ── Cur ──────────────────────────────────────────────────────────────────────

/// A loaded X11 cursor (created via `XCreateFontCursor`).
#[derive(Debug)]
pub struct Cur {
    pub cursor: u32,
}

// SAFETY: cursor ids are just integers; instantWM is single-threaded.
unsafe impl Send for Cur {}
unsafe impl Sync for Cur {}

impl Cur {
    pub fn new(cursor: u32) -> Self {
        Self { cursor }
    }
}
