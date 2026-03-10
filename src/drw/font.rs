//! Font loading and management.
//!
//! [`Fnt`] is a singly-linked list node representing one loaded Xft font.
//! Multiple fonts are chained together to form a *fontset*: when rendering a
//! glyph the drawing code walks the list until it finds a font that contains
//! the required codepoint (Unicode fallback).
//!
//! # Ownership model
//!
//! Only the *head* node of a fontset that was created by [`Drw::fontset_create`]
//! carries `owns_resources = true`.  Clones produced by [`Clone`] always set
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

    /// Next font in the fallback chain, or `None` for the last font.
    pub next: Option<Box<Fnt>>,

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
            next: self.next.clone(),
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

    /// Iterate over every font in the linked list, starting with `self`.
    pub fn iter(&self) -> FontIter<'_> {
        FontIter {
            current: Some(self),
        }
    }

    /// Count the total number of fonts in this fontset (including `self`).
    pub fn count(&self) -> usize {
        self.iter().count()
    }

    /// Return the font at position `idx` in the linked list, or `None` if the
    /// list is shorter than `idx + 1`.
    pub fn get(&self, idx: usize) -> Option<&Fnt> {
        self.iter().nth(idx)
    }

    /// Append `font` to the tail of this linked list.
    pub fn push_back(&mut self, font: Box<Fnt>) {
        let mut tail = self;
        while tail.next.is_some() {
            tail = tail.next.as_mut().unwrap();
        }
        tail.next = Some(font);
    }
}

impl Drop for Fnt {
    fn drop(&mut self) {
        // SAFETY: pointers are valid as long as the display is still open,
        // which is guaranteed by the `Drw` owning both.
        unsafe {
            if self.owns_resources {
                if !self.pattern.is_null() {
                    FcPatternDestroy(self.pattern);
                }
                if !self.xfont.is_null() && !self.display.is_null() {
                    XftFontClose(self.display, self.xfont);
                }
            }
        }
    }
}

// ── FontIter ─────────────────────────────────────────────────────────────────

/// Iterator over the fonts in a [`Fnt`] linked list.
pub struct FontIter<'a> {
    current: Option<&'a Fnt>,
}

impl<'a> Iterator for FontIter<'a> {
    type Item = &'a Fnt;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.current?;
        self.current = node.next.as_deref();
        Some(node)
    }
}
