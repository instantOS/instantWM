//! Raw FFI bindings for X11, Xft, and Fontconfig.
//!
//! This module is the single place where all `extern "C"` declarations live.
//! Nothing here is meant to be used directly outside of the `drw` module —
//! consumers should go through the safe wrappers in the parent module.

use std::os::raw::{c_char, c_int, c_ulong};

use x11rb::protocol::xproto::{Drawable, Point, Window};

// ── C-layout types ────────────────────────────────────────────────────────────

/// Opaque GC handle as returned by Xlib (a `*mut void` at the C level).
pub type XlibGc = *mut libc::c_void;

pub type FcBool = c_int;
pub type FcResult = c_int;
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

// ── `XWindowAttributes` ───────────────────────────────────────────────────────

/// Mirrors the C `XWindowAttributes` struct (used in `XGetWindowAttributes`).
#[repr(C)]
pub struct XWindowAttributes {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub border_width: u32,
    pub depth: u32,
    pub visual: *mut libc::c_void,
    pub root: Window,
    pub c_class: u8,
    pub bit_gravity: u8,
    pub win_gravity: u8,
    pub backing_store: i32,
    pub backing_planes: u64,
    pub backing_pixel: u64,
    pub save_under: u8,
    pub colormap: u64,
    pub map_installed: u8,
    pub map_state: u8,
    pub all_event_masks: i64,
    pub your_event_mask: i64,
    pub do_not_propagate_mask: u16,
    pub override_redirect: u8,
    pub screen: *mut libc::c_void,
}

// ── `XSetWindowAttributes` ────────────────────────────────────────────────────

/// Mirrors the C `XSetWindowAttributes` struct.
#[repr(C)]
pub struct XSetWindowAttributes {
    pub background_pixmap: u64,
    pub background_pixel: u64,
    pub border_pixmap: u64,
    pub border_pixel: u64,
    pub bit_gravity: u32,
    pub win_gravity: u32,
    pub backing_store: u32,
    pub backing_planes: u64,
    pub backing_pixel: u64,
    pub save_under: u32,
    pub event_mask: u64,
    pub do_not_propagate_mask: u32,
    pub override_redirect: u32,
    pub colormap: u64,
    pub cursor: u64,
}

impl Default for XSetWindowAttributes {
    fn default() -> Self {
        // SAFETY: all-zero is a valid initialisation for this POD C struct.
        unsafe { std::mem::zeroed() }
    }
}

// ── X11 (`libX11`) ────────────────────────────────────────────────────────────

#[cfg(feature = "x11_backend")]
#[link(name = "X11")]
extern "C" {
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
    pub fn XEventsQueued(display: *mut libc::c_void, mode: c_int) -> c_int;

    pub fn XSetLineAttributes(
        display: *mut libc::c_void,
        gc: XlibGc,
        line_width: c_int,
        line_style: c_int,
        cap_style: c_int,
        join_style: c_int,
    );

    pub fn XCreateFontCursor(display: *mut libc::c_void, shape: u32) -> u32;
    pub fn XFreeCursor(display: *mut libc::c_void, cursor: u32);

    pub fn XGetXCBConnection(display: *mut libc::c_void) -> *mut libc::c_void;

    pub fn XCreateWindow(
        display: *mut libc::c_void,
        parent: Window,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        border_width: u32,
        depth: i32,
        class: u32,
        visual: *mut libc::c_void,
        valuemask: u64,
        attributes: *mut libc::c_void,
    ) -> Window;

    pub fn XMapWindow(display: *mut libc::c_void, window: Window);

    pub fn XConfigureWindow(
        display: *mut libc::c_void,
        w: Window,
        changes: u32,
        values: *mut libc::c_void,
    );

    pub fn XSelectInput(display: *mut libc::c_void, w: Window, event_mask: i64);

    pub fn XGetWindowAttributes(
        display: *mut libc::c_void,
        w: Window,
        attrs: *mut XWindowAttributes,
    ) -> i32;

    pub fn XCreateSimpleWindow(
        display: *mut libc::c_void,
        parent: Window,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        border_width: u32,
        border: u32,
        background: u32,
    ) -> Window;

    pub fn XChangeWindowAttributes(
        display: *mut libc::c_void,
        w: Window,
        valuemask: u64,
        attributes: *mut libc::c_void,
    );
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XOpenDisplay(_name: *const c_char) -> *mut libc::c_void {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCloseDisplay(_display: *mut libc::c_void) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDefaultScreen(_display: *mut libc::c_void) -> c_int {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDefaultRootWindow(_display: *mut libc::c_void) -> Window {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDefaultVisual(_display: *mut libc::c_void, _screen: c_int) -> *mut libc::c_void {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDefaultColormap(_display: *mut libc::c_void, _screen: c_int) -> c_ulong {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDefaultDepth(_display: *mut libc::c_void, _screen: c_int) -> c_int {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCreatePixmap(
    _display: *mut libc::c_void,
    _d: Drawable,
    _width: u32,
    _height: u32,
    _depth: u32,
) -> Drawable {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFreePixmap(_display: *mut libc::c_void, _pixmap: Drawable) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCreateGC(
    _display: *mut libc::c_void,
    _d: Drawable,
    _valuemask: c_ulong,
    _values: *mut libc::c_void,
) -> XlibGc {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFreeGC(_display: *mut libc::c_void, _gc: XlibGc) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XSetForeground(_display: *mut libc::c_void, _gc: XlibGc, _foreground: c_ulong) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFillRectangle(
    _display: *mut libc::c_void,
    _d: Drawable,
    _gc: XlibGc,
    _x: c_int,
    _y: c_int,
    _width: u32,
    _height: u32,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDrawRectangle(
    _display: *mut libc::c_void,
    _d: Drawable,
    _gc: XlibGc,
    _x: c_int,
    _y: c_int,
    _width: u32,
    _height: u32,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFillArc(
    _display: *mut libc::c_void,
    _d: Drawable,
    _gc: XlibGc,
    _x: c_int,
    _y: c_int,
    _width: u32,
    _height: u32,
    _angle1: c_int,
    _angle2: c_int,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XDrawArc(
    _display: *mut libc::c_void,
    _d: Drawable,
    _gc: XlibGc,
    _x: c_int,
    _y: c_int,
    _width: u32,
    _height: u32,
    _angle1: c_int,
    _angle2: c_int,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFillPolygon(
    _display: *mut libc::c_void,
    _d: Drawable,
    _gc: XlibGc,
    _points: *mut Point,
    _npoints: c_int,
    _shape: c_int,
    _mode: c_int,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCopyArea(
    _display: *mut libc::c_void,
    _src: Drawable,
    _dest: Drawable,
    _gc: XlibGc,
    _src_x: c_int,
    _src_y: c_int,
    _width: u32,
    _height: u32,
    _dest_x: c_int,
    _dest_y: c_int,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XSync(_display: *mut libc::c_void, _discard: c_int) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFlush(_display: *mut libc::c_void) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XEventsQueued(_display: *mut libc::c_void, _mode: c_int) -> c_int {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XSetLineAttributes(
    _display: *mut libc::c_void,
    _gc: XlibGc,
    _line_width: c_int,
    _line_style: c_int,
    _cap_style: c_int,
    _join_style: c_int,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCreateFontCursor(_display: *mut libc::c_void, _shape: u32) -> u32 {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XFreeCursor(_display: *mut libc::c_void, _cursor: u32) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XGetXCBConnection(_display: *mut libc::c_void) -> *mut libc::c_void {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCreateWindow(
    _display: *mut libc::c_void,
    _parent: Window,
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
    _border_width: u32,
    _depth: i32,
    _class: u32,
    _visual: *mut libc::c_void,
    _valuemask: u64,
    _attributes: *mut libc::c_void,
) -> Window {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XMapWindow(_display: *mut libc::c_void, _window: Window) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XConfigureWindow(
    _display: *mut libc::c_void,
    _w: Window,
    _changes: u32,
    _values: *mut libc::c_void,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XSelectInput(_display: *mut libc::c_void, _w: Window, _event_mask: i64) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XGetWindowAttributes(
    _display: *mut libc::c_void,
    _w: Window,
    _attrs: *mut XWindowAttributes,
) -> i32 {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XCreateSimpleWindow(
    _display: *mut libc::c_void,
    _parent: Window,
    _x: i32,
    _y: i32,
    _width: u32,
    _height: u32,
    _border_width: u32,
    _border: u32,
    _background: u32,
) -> Window {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XChangeWindowAttributes(
    _display: *mut libc::c_void,
    _w: Window,
    _valuemask: u64,
    _attributes: *mut libc::c_void,
) {
}

// ── Xft (`libXft`) ────────────────────────────────────────────────────────────

#[cfg(feature = "x11_backend")]
#[link(name = "Xft")]
extern "C" {
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

    pub fn XftFontMatch(
        display: *mut libc::c_void,
        screen: c_int,
        pattern: *mut FcPattern,
        result: *mut XftResult,
    ) -> *mut FcPattern;
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftInit() -> c_int {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftFontOpenName(
    _display: *mut libc::c_void,
    _screen: c_int,
    _name: *const c_char,
) -> *mut XftFont {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftFontOpenPattern(
    _display: *mut libc::c_void,
    _pattern: *mut FcPattern,
) -> *mut XftFont {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftFontClose(_display: *mut libc::c_void, _font: *mut XftFont) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftCharExists(_display: *mut libc::c_void, _font: *mut XftFont, _ucs4: u32) -> c_int {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftTextExtentsUtf8(
    _display: *mut libc::c_void,
    _font: *mut XftFont,
    _string: *const u8,
    _len: c_int,
    extents: *mut XGlyphInfo,
) {
    if let Some(extents) = extents.as_mut() {
        *extents = XGlyphInfo {
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            x_off: 0,
            y_off: 0,
        };
    }
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftDrawCreate(
    _display: *mut libc::c_void,
    _drawable: Drawable,
    _visual: *mut libc::c_void,
    _colormap: c_ulong,
) -> *mut XftDraw {
    std::ptr::null_mut()
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftDrawDestroy(_draw: *mut XftDraw) {}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftDrawStringUtf8(
    _draw: *mut XftDraw,
    _color: *const XftColor,
    _font: *mut XftFont,
    _x: c_int,
    _y: c_int,
    _string: *const u8,
    _len: c_int,
) {
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftColorAllocName(
    _display: *mut libc::c_void,
    _visual: *mut libc::c_void,
    _cmap: c_ulong,
    _name: *const c_char,
    _result: *mut XftColor,
) -> c_int {
    0
}

#[cfg(not(feature = "x11_backend"))]
#[allow(non_snake_case)]
pub unsafe fn XftFontMatch(
    _display: *mut libc::c_void,
    _screen: c_int,
    _pattern: *mut FcPattern,
    _result: *mut XftResult,
) -> *mut FcPattern {
    std::ptr::null_mut()
}

// ── Fontconfig (`libfontconfig`) ──────────────────────────────────────────────

#[link(name = "fontconfig")]
extern "C" {
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
