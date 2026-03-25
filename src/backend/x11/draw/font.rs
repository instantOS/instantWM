//! Font loading and management.
//!
//! [`Fnt`] represents one loaded Xft font. Fonts are stored in a [`Vec`] to form
//! a *fontset*: when rendering a glyph the drawing code walks the vector until
//! it finds a font that contains the required codepoint (Unicode fallback).
//!
//! # Ownership model
//!
//! Only the original `Fnt` created by [`Drw::fontset_create`] carries
//! `owns_resources = true`.  Clones produced by [`Clone`] always set
//! `owns_resources = false` so that resources are freed exactly once when the
//! original is dropped.

use super::ffi::{FcPattern, FcPatternDestroy, XftFont, XftFontClose};

// ── Fnt ──────────────────────────────────────────────────────────────────────

/// A single font in a fontset linked list.
///
/// Fields are `pub` where callers (e.g. the text-drawing loop) need direct
/// read access; mutating internals should go through [`Drw`] methods.
///
/// [`Drw`]: super::Drw
pub struct Fnt {
    /// The Xlib display this font was loaded against.
    /// Stored here so `Drop` can call `XftFontClose` without needing a `Drw`.
    pub(super) display: *mut libc::c_void,

    /// Combined ascent + descent in pixels — the effective line height.
    pub h: u32,

    /// Raw Xft font pointer.
    pub xfont: *mut XftFont,

    /// Fontconfig pattern used to load this font.
    /// Required when performing fallback font matching (see `Drw::text`).
    pub pattern: *mut FcPattern,

    /// Whether this font node owns `pattern` and should destroy it on drop.
    ///
    /// Fonts opened with `XftFontOpenPattern` transfer pattern ownership into
    /// Xft, so destroying the pattern ourselves would double-free.
    pub(super) owns_pattern: bool,

    /// Font ascent in pixels (used to vertically centre glyphs).
    pub(super) ascent: i32,

    /// When `true` this node owns the Xft font and Fc pattern and will free
    /// them on drop.  Clones set this to `false`.
    pub(super) owns_resources: bool,
}

// SAFETY: Fnt holds raw pointers to X11 objects.  instantWM is
// single-threaded, so there are no concurrent accesses.
unsafe impl Send for Fnt {}
unsafe impl Sync for Fnt {}

impl Clone for Fnt {
    /// Produces a *shallow* clone that does **not** own the underlying
    /// resources.  The clone shares the same raw pointers but will not free
    /// them when dropped.
    fn clone(&self) -> Self {
        Self {
            display: self.display,
            h: self.h,
            xfont: self.xfont,
            pattern: self.pattern,
            owns_pattern: false,
            ascent: self.ascent,
            owns_resources: false,
        }
    }
}

impl Fnt {
    /// Returns the line height of this font in pixels (`ascent + descent`).
    #[inline]
    pub fn height(&self) -> u32 {
        self.h
    }

    /// Ascent in pixels (distance from baseline to top of glyphs).
    #[inline]
    pub fn ascent(&self) -> i32 {
        self.ascent
    }
}

impl Drop for Fnt {
    fn drop(&mut self) {
        // SAFETY: pointers are valid as long as the display is still open,
        // which is guaranteed by the `Drw` owning both.
        unsafe {
            if self.owns_resources {
                if self.owns_pattern && !self.pattern.is_null() {
                    FcPatternDestroy(self.pattern);
                }
                if !self.xfont.is_null() && !self.display.is_null() {
                    XftFontClose(self.display, self.xfont);
                }
            }
        }
    }
}
