//! Color and cursor types used by the drawing context.
//!
//! [`Color`] wraps an `XftColor` (a pixel value + 16-bit RGBA components).
//! [`Cursor`] wraps an X11 cursor id created via `XCreateFontCursor`.

use super::ffi::{XRenderColor, XftColor};
use std::os::raw::c_ulong;

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

// ── Schemes ───────────────────────────────────────────────────────────────────

/// A color scheme of allocated X11/Xft colors.
///
/// X11-runtime counterpart of [`crate::types::ColorSchemeRgba`]: colors are
/// allocated against a display once and reused by pixel value.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorScheme {
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Detail/accent color.
    pub detail: Color,
}

impl ColorScheme {
    /// Create a color scheme from a single color (replicated to fg, bg, detail).
    ///
    /// Useful for things like borders that only need one color.
    pub fn from_single(color: Color) -> Self {
        Self {
            fg: color.clone(),
            bg: color.clone(),
            detail: color,
        }
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        let zero = Color::default();
        Self {
            fg: zero.clone(),
            bg: zero.clone(),
            detail: zero,
        }
    }
}

/// Color scheme variants for different border states.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BorderScheme {
    /// Normal/unfocused border colors.
    pub normal: ColorScheme,
    /// Focused tiled window border colors.
    pub tile_focus: ColorScheme,
    /// Focused floating window border colors.
    pub float_focus: ColorScheme,
    /// Snap indicator border colors.
    pub snap: ColorScheme,
    /// Destructive overview gesture threshold feedback.
    pub close: ColorScheme,
}

/// Color scheme for status bar elements.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusScheme {
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Detail/accent color.
    pub detail: Color,
}

impl StatusScheme {
    /// Create a new status scheme.
    pub fn new(fg: Color, bg: Color, detail: Color) -> Self {
        Self { fg, bg, detail }
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
