//! Raw FFI bindings for X11, Xft, and Fontconfig.
//!
//! This module is the single place where all `extern "C"` declarations live.
//! Nothing here is meant to be used directly outside of the `draw` module —
//! consumers should go through the safe wrappers in the parent module.

use std::os::raw::{c_char, c_int, c_ulong};

use x11rb::protocol::xproto::{Drawable, Point, Window};

// ── C-layout types ────────────────────────────────────────────────────────────

/// Opaque GC handle as returned by Xlib (a `*mut void` at the C level).
pub type XlibGc = *mut libc::c_void;

pub type FcBool = c_int;
pub type XftResult = c_int;

/// `XRenderColor` — 16-bit premultiplied RGBA.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XRenderColor {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub alpha: u16,
}

/// `XftColor` — pairs a pixel value with the full render color.
#[repr(C)]
pub struct XftColor {
    pub pixel: c_ulong,
    pub color: XRenderColor,
}

impl Clone for XftColor {
    fn clone(&self) -> Self {
        Self {
            pixel: self.pixel,
            color: self.color,
        }
    }
}

impl std::fmt::Debug for XftColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XftColor")
            .field("pixel", &self.pixel)
            .field("color", &self.color)
            .finish()
    }
}

/// `XftFont` — subset of the Xft font struct we need.
#[repr(C)]
pub struct XftFont {
    pub ascent: c_int,
    pub descent: c_int,
    pub height: c_int,
    pub max_advance_width: c_int,
    pub charset: *mut libc::c_void,
    pub pattern: *mut libc::c_void,
}

/// Opaque `XftDraw` handle.
#[repr(C)]
pub struct XftDraw {
    _private: [u8; 0],
}

/// `XGlyphInfo` — glyph metrics returned by `XftTextExtentsUtf8`.
#[repr(C)]
pub struct XGlyphInfo {
    pub width: u16,
    pub height: u16,
    pub x: i16,
    pub y: i16,
    pub x_off: i16,
    pub y_off: i16,
}

/// Opaque `FcPattern` handle.
#[repr(C)]
pub struct FcPattern {
    _private: [u8; 0],
}

/// Opaque `FcCharSet` handle.
#[repr(C)]
pub struct FcCharSet {
    _private: [u8; 0],
}

/// Fontconfig property key for charset matching.
pub const FC_CHARSET: &[u8] = b"charset\0";
/// Fontconfig property key for scalable-font filtering.
pub const FC_SCALABLE: &[u8] = b"scalable\0";

pub const FC_MATCH_PATTERN: c_int = 1;
pub const FC_TRUE: FcBool = 1;

// ── X11 (`libX11`) ────────────────────────────────────────────────────────────

#[link(name = "X11")]
unsafe extern "C" {
    pub fn XOpenDisplay(name: *const c_char) -> *mut libc::c_void;
    pub fn XCloseDisplay(display: *mut libc::c_void);
    pub fn XDefaultScreen(display: *mut libc::c_void) -> c_int;
    pub fn XDefaultRootWindow(display: *mut libc::c_void) -> Window;
    pub fn XDefaultVisual(display: *mut libc::c_void, screen: c_int) -> *mut libc::c_void;
    pub fn XDefaultColormap(display: *mut libc::c_void, screen: c_int) -> c_ulong;
    pub fn XDefaultDepth(display: *mut libc::c_void, screen: c_int) -> c_int;

    pub fn XCreatePixmap(
        display: *mut libc::c_void,
        d: Drawable,
        width: u32,
        height: u32,
        depth: u32,
    ) -> Drawable;
    pub fn XFreePixmap(display: *mut libc::c_void, pixmap: Drawable);

    pub fn XCreateGC(
        display: *mut libc::c_void,
        d: Drawable,
        valuemask: c_ulong,
        values: *mut libc::c_void,
    ) -> XlibGc;
    pub fn XFreeGC(display: *mut libc::c_void, gc: XlibGc);

    pub fn XSetForeground(display: *mut libc::c_void, gc: XlibGc, foreground: c_ulong);

    pub fn XFillRectangle(
        display: *mut libc::c_void,
        d: Drawable,
        gc: XlibGc,
        x: c_int,
        y: c_int,
        width: u32,
        height: u32,
    );
    pub fn XDrawRectangle(
        display: *mut libc::c_void,
        d: Drawable,
        gc: XlibGc,
        x: c_int,
        y: c_int,
        width: u32,
        height: u32,
    );

    pub fn XFillArc(
        display: *mut libc::c_void,
        d: Drawable,
        gc: XlibGc,
        x: c_int,
        y: c_int,
        width: u32,
        height: u32,
        angle1: c_int,
        angle2: c_int,
    );
    pub fn XDrawArc(
        display: *mut libc::c_void,
        d: Drawable,
        gc: XlibGc,
        x: c_int,
        y: c_int,
        width: u32,
        height: u32,
        angle1: c_int,
        angle2: c_int,
    );

    pub fn XFillPolygon(
        display: *mut libc::c_void,
        d: Drawable,
        gc: XlibGc,
        points: *mut Point,
        npoints: c_int,
        shape: c_int,
        mode: c_int,
    );

    pub fn XCopyArea(
        display: *mut libc::c_void,
        src: Drawable,
        dest: Drawable,
        gc: XlibGc,
        src_x: c_int,
        src_y: c_int,
        width: u32,
        height: u32,
        dest_x: c_int,
        dest_y: c_int,
    );

    pub fn XSync(display: *mut libc::c_void, discard: c_int);
    pub fn XFlush(display: *mut libc::c_void);

    pub fn XSetLineAttributes(
        display: *mut libc::c_void,
        gc: XlibGc,
        line_width: c_int,
        line_style: c_int,
        cap_style: c_int,
        join_style: c_int,
    );

    pub fn XCreateFontCursor(display: *mut libc::c_void, shape: u32) -> c_ulong;
    pub fn XFreeCursor(display: *mut libc::c_void, cursor: c_ulong);

}

// ── Xft (`libXft`) ────────────────────────────────────────────────────────────

#[link(name = "Xft")]
unsafe extern "C" {
    pub fn XftInit() -> c_int;

    pub fn XftFontOpenName(
        display: *mut libc::c_void,
        screen: c_int,
        name: *const c_char,
    ) -> *mut XftFont;

    pub fn XftFontOpenPattern(display: *mut libc::c_void, pattern: *mut FcPattern) -> *mut XftFont;

    pub fn XftFontClose(display: *mut libc::c_void, font: *mut XftFont);

    pub fn XftCharExists(display: *mut libc::c_void, font: *mut XftFont, ucs4: u32) -> c_int;

    pub fn XftTextExtentsUtf8(
        display: *mut libc::c_void,
        font: *mut XftFont,
        string: *const u8,
        len: c_int,
        extents: *mut XGlyphInfo,
    );

    pub fn XftDrawCreate(
        display: *mut libc::c_void,
        drawable: Drawable,
        visual: *mut libc::c_void,
        colormap: c_ulong,
    ) -> *mut XftDraw;

    pub fn XftDrawDestroy(draw: *mut XftDraw);

    pub fn XftDrawStringUtf8(
        draw: *mut XftDraw,
        color: *const XftColor,
        font: *mut XftFont,
        x: c_int,
        y: c_int,
        string: *const u8,
        len: c_int,
    );

    pub fn XftColorAllocName(
        display: *mut libc::c_void,
        visual: *mut libc::c_void,
        cmap: c_ulong,
        name: *const c_char,
        result: *mut XftColor,
    ) -> c_int;

    pub fn XftColorAllocValue(
        display: *mut libc::c_void,
        visual: *mut libc::c_void,
        cmap: c_ulong,
        color: *mut XRenderColor,
        result: *mut XftColor,
    ) -> c_int;

    pub fn XftFontMatch(
        display: *mut libc::c_void,
        screen: c_int,
        pattern: *mut FcPattern,
        result: *mut XftResult,
    ) -> *mut FcPattern;
}

// ── Fontconfig (`libfontconfig`) ──────────────────────────────────────────────

#[link(name = "fontconfig")]
unsafe extern "C" {
    pub fn FcInit() -> FcBool;

    pub fn FcNameParse(name: *const u8) -> *mut FcPattern;

    pub fn FcPatternDuplicate(pattern: *mut FcPattern) -> *mut FcPattern;
    pub fn FcPatternDestroy(pattern: *mut FcPattern);

    pub fn FcPatternAddCharSet(
        pattern: *mut FcPattern,
        object: *const u8,
        charset: *mut FcCharSet,
    ) -> FcBool;

    pub fn FcPatternAddBool(pattern: *mut FcPattern, object: *const u8, value: FcBool) -> FcBool;

    pub fn FcConfigSubstitute(config: *mut libc::c_void, pattern: *mut FcPattern, kind: c_int);

    pub fn FcDefaultSubstitute(pattern: *mut FcPattern);

    pub fn FcCharSetCreate() -> *mut FcCharSet;
    pub fn FcCharSetAddChar(fcs: *mut FcCharSet, ucs4: u32) -> FcBool;
    pub fn FcCharSetDestroy(fcs: *mut FcCharSet);
}
