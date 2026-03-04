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
//! | [`utf8`]   | UTF-8 decoding utilities                               |
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
mod utf8;

// ── Public re-exports ─────────────────────────────────────────────────────────
//
// Everything below forms the *stable* public API of this module.  Callers
// should import from `crate::drw::*` rather than from the sub-modules
// directly.
//
// Many items below are re-exported solely so that external code that was
// ported directly from C can keep using the same flat namespace.  The
// `#[allow(unused_imports)]` attribute on each group suppresses the "unused
// import" lint that fires when a particular symbol isn't referenced inside
// *this* crate (it may still be used by downstream consumers).

// Color / cursor types and scheme-index constants.
#[allow(unused_imports)]
pub use color::{Color, Cursor, COL_BG, COL_DETAIL, COL_FG, COL_LAST};

// Font linked-list type and its iterator.
#[allow(unused_imports)]
pub use font::{Fnt, FontIter};

// The main drawing context.
pub use draw::Drw;

// UTF-8 decoding — exposed for modules that need to walk raw byte strings
// (e.g. the status-bar parser) and for unit tests.
#[allow(unused_imports)]
pub use utf8::{utf8decode, UTFBYTE, UTFMASK, UTFMAX, UTFMIN, UTF_INVALID, UTF_SIZ};

// Raw FFI symbols that other modules reference directly.
//
// Keep this list as small as possible — new code should go through the safe
// wrappers on `Drw`.  These exist only for legacy call-sites that call Xlib /
// Xft functions by hand (e.g. bar widgets that do custom XFillRectangle calls).
#[allow(unused_imports)]
pub use ffi::{
    // Fontconfig
    FcBool,
    FcCharSet,
    FcPattern,
    FcResult,
    // X11 — window management
    XChangeWindowAttributes,
    // X11 — display / screen / root
    XCloseDisplay,
    XConfigureWindow,
    // X11 — drawing primitives
    XCopyArea,
    XCreateFontCursor,
    // X11 — pixmap / GC
    XCreateGC,
    XCreatePixmap,
    XCreateSimpleWindow,
    XCreateWindow,
    XDefaultColormap,
    XDefaultDepth,
    XDefaultRootWindow,
    XDefaultScreen,
    XDefaultVisual,
    XDrawArc,
    XDrawRectangle,
    XEventsQueued,
    XFillArc,
    XFillPolygon,
    XFillRectangle,
    XFlush,
    XFreeCursor,
    XFreeGC,
    XFreePixmap,
    XGetWindowAttributes,
    XGetXCBConnection,
    XGlyphInfo,
    XMapWindow,
    XOpenDisplay,
    XRenderColor,
    XSelectInput,
    XSetForeground,
    XSetLineAttributes,
    XSetWindowAttributes,
    XSync,
    XWindowAttributes,
    // Xft
    XftCharExists,
    XftColor,
    XftColorAllocName,
    XftDraw,
    XftDrawCreate,
    XftDrawDestroy,
    XftDrawStringUtf8,
    XftFont,
    XftFontClose,
    XftFontMatch,
    XftFontOpenName,
    XftFontOpenPattern,
    XftInit,
    XftResult,
    XftTextExtentsUtf8,
    XlibGc,
    FC_CHARSET,
    FC_MATCH_PATTERN,
    FC_SCALABLE,
    FC_TRUE,
};

#[inline]
pub(crate) fn x11_supported() -> bool {
    cfg!(feature = "x11_backend")
}

// ── Compatibility helpers ─────────────────────────────────────────────────────
