#![allow(clippy::too_many_arguments)]
//! The [`DrawContext`] drawing context.
//!
//! `DrawContext` owns an X11 display connection, a pixmap, a GC, and a fontset.
//! All bar rendering goes through this type.
//!
//! # Lifecycle
//!
//! ```text
//! DrawContext::new()          – open display, create pixmap + GC
//!   └─ fontset_create – load fonts into the fontset vector
//!   └─ set_scheme     – choose the active color scheme
//!   └─ text / rect / circ / arrow  – render into the pixmap
//!   └─ map            – blit the pixmap to the target window
//! Drop                – free pixmap, GC, close display
//! ```

mod text;
use text::zero_glyph_info;

use std::ffi::CString;
use std::os::raw::{c_int, c_ulong};
use std::ptr;

use x11rb::protocol::xproto::{Drawable, Point, Window};

use std::cmp::min;
use std::collections::VecDeque;

use super::color::{Color, Cursor};
use super::ffi::{
    FC_CHARSET, FC_MATCH_PATTERN, FC_SCALABLE, FC_TRUE, FcCharSetAddChar, FcCharSetCreate,
    FcCharSetDestroy, FcConfigSubstitute, FcDefaultSubstitute, FcInit, FcNameParse, FcPattern,
    FcPatternAddBool, FcPatternAddCharSet, FcPatternDestroy, FcPatternDuplicate, XCloseDisplay,
    XCopyArea, XCreateFontCursor, XCreateGC, XCreatePixmap, XDefaultColormap, XDefaultDepth,
    XDefaultRootWindow, XDefaultScreen, XDefaultVisual, XDrawArc, XDrawRectangle, XFillArc,
    XFillPolygon, XFillRectangle, XFreeCursor, XFreeGC, XFreePixmap, XGlyphInfo, XOpenDisplay,
    XRenderColor, XSetForeground, XSetLineAttributes, XSync, XftCharExists, XftColor,
    XftColorAllocName, XftColorAllocValue, XftDraw, XftDrawCreate, XftDrawDestroy,
    XftDrawStringUtf8, XftFont, XftFontClose, XftFontMatch, XftFontOpenName, XftFontOpenPattern,
    XftInit, XftResult, XftTextExtentsUtf8, XlibGc,
};
use super::font::Fnt;

use crate::types::{ColorScheme, Rect as WmRect};

/// How many "no-match" codepoints we remember to avoid repeatedly trying to
/// find a fallback font for the same unrenderable character.
const NOMATCHES_LEN: usize = 64;

// ── DrawContext ──────────────────────────────────────────────────────────────────────

/// The main drawing context.
///
/// Wraps an Xlib display, a server-side pixmap used as an off-screen buffer,
/// a graphics context (GC), the active color scheme, and the fontset.
// Note: the name `DrawContext` matches dwm's drawing context naming.
pub struct DrawContext {
    /// Pixmap / drawable width.
    pub w: u32,
    /// Pixmap / drawable height.
    pub h: u32,

    pub(super) display: *mut libc::c_void,
    pub(super) screen: i32,
    pub(super) root: Window,

    /// Off-screen pixmap — all drawing happens here, then [`DrawContext::map`] blits
    /// it to the real window.
    pub(super) drawable: Drawable,
    pub(super) gc: XlibGc,

    /// Active color scheme.
    scheme: Option<ColorScheme>,

    /// Loaded fontset (vector of fonts for fallback chain).
    pub fonts: Option<Vec<Fnt>>,

    depth: u8,
    visual: *mut libc::c_void,
    colormap: c_ulong,

    /// Ring buffer of codepoints for which no fallback font was found.
    nomatches: VecDeque<u32>,

    /// Cached pixel width of `"..."` for the current fontset.
    ellipsis_width: u32,

    /// `true` only for the *original* `DrawContext` — clones do **not** own resources.
    owns_resources: bool,
}

// SAFETY: instantWM is single-threaded; no concurrent mutation.
unsafe impl Send for DrawContext {}
unsafe impl Sync for DrawContext {}

impl Clone for DrawContext {
    /// Produces a shallow clone that shares raw pointers but does **not** free
    /// them on drop.  Useful for short-lived drawing in helper functions.
    fn clone(&self) -> Self {
        Self {
            w: self.w,
            h: self.h,
            display: self.display,
            screen: self.screen,
            root: self.root,
            drawable: self.drawable,
            gc: self.gc,
            scheme: self.scheme.clone(),
            fonts: self.fonts.clone(),
            depth: self.depth,
            visual: self.visual,
            colormap: self.colormap,
            nomatches: self.nomatches.clone(),
            ellipsis_width: self.ellipsis_width,
            owns_resources: false,
        }
    }
}

impl Drop for DrawContext {
    fn drop(&mut self) {
        // Drop fonts while the X display is still valid. `Fnt::drop()`
        // calls `XftFontClose`, which requires a live display.
        self.fonts.take();

        // SAFETY: only the owning instance frees resources.
        unsafe {
            if self.owns_resources && !self.display.is_null() {
                XFreePixmap(self.display, self.drawable);
                XFreeGC(self.display, self.gc);
                XCloseDisplay(self.display);
            }
        }
    }
}

// ── Construction & display accessors ─────────────────────────────────────────

impl DrawContext {
    /// Open an X display and initialise the drawing context.
    ///
    /// `display_name` overrides `$DISPLAY` when `Some`.
    pub fn new(display_name: Option<&str>) -> Result<Self, String> {
        unsafe {
            FcInit();
            XftInit();
        }

        let display_name_str = display_name
            .map(|s| s.to_string())
            .or_else(|| std::env::var("DISPLAY").ok());

        let display_name_cstring = display_name_str
            .as_ref()
            .and_then(|s| CString::new(s.as_str()).ok());
        let name_ptr = display_name_cstring
            .as_ref()
            .map(|cs| cs.as_ptr())
            .unwrap_or(ptr::null());

        let display = unsafe { XOpenDisplay(name_ptr) };
        if display.is_null() {
            return Err("cannot open display".to_string());
        }

        unsafe {
            let screen = XDefaultScreen(display);
            let root = XDefaultRootWindow(display);
            if root == 0 {
                XCloseDisplay(display);
                return Err("cannot get root window".to_string());
            }

            let visual = XDefaultVisual(display, screen);
            if visual.is_null() {
                XCloseDisplay(display);
                return Err("cannot get default visual".to_string());
            }

            let colormap = XDefaultColormap(display, screen);
            if colormap == 0 {
                XCloseDisplay(display);
                return Err("cannot get default colormap".to_string());
            }

            let depth = XDefaultDepth(display, screen);
            if depth <= 0 {
                XCloseDisplay(display);
                return Err("cannot get default depth".to_string());
            }

            let drawable = XCreatePixmap(display, root, 1, 1, depth as u32);
            if drawable == 0 {
                XCloseDisplay(display);
                return Err("cannot create pixmap".to_string());
            }

            let gc = XCreateGC(display, root, 0, ptr::null_mut());
            if gc.is_null() {
                XFreePixmap(display, drawable);
                XCloseDisplay(display);
                return Err("cannot create graphics context".to_string());
            }

            XSetLineAttributes(display, gc, 1, 0, 0, 0);

            Ok(Self {
                w: 1,
                h: 1,
                display,
                screen,
                root,
                drawable,
                gc,
                scheme: None,
                fonts: None,
                depth: depth as u8,
                visual,
                colormap,
                nomatches: VecDeque::with_capacity(NOMATCHES_LEN),
                ellipsis_width: 0,
                owns_resources: true,
            })
        }
    }

    /// Raw Xlib display pointer (for callers that must call Xlib directly).
    #[inline]
    pub fn display(&self) -> *mut libc::c_void {
        self.display
    }

    /// True when the X11 display pointer is valid.
    #[inline]
    pub fn has_display(&self) -> bool {
        !self.display.is_null()
    }

    /// X screen number.
    #[inline]
    pub fn screen(&self) -> i32 {
        self.screen
    }

    /// Root window id.
    #[inline]
    pub fn root(&self) -> Window {
        self.root
    }

    /// Current off-screen drawable id.
    #[inline]
    pub fn drawable(&self) -> Drawable {
        self.drawable
    }

    /// Current graphics context handle.
    #[inline]
    pub fn gc(&self) -> XlibGc {
        self.gc
    }
}

// ── Pixmap / target management ───────────────────────────────────────────────

impl DrawContext {
    /// Resize the off-screen pixmap to `w × h` pixels.
    pub fn resize(&mut self, w: u32, h: u32) {
        if self.display.is_null() {
            return;
        }
        if w == 0 || h == 0 {
            return;
        }
        unsafe {
            let new_drawable = XCreatePixmap(self.display, self.root, w, h, self.depth as u32);
            if new_drawable == 0 {
                return;
            }
            if self.drawable != 0 {
                XFreePixmap(self.display, self.drawable);
            }
            self.drawable = new_drawable;
        }
        self.w = w;
        self.h = h;
    }

    /// Blit a region of the off-screen pixmap to `window`.
    pub fn map(&self, window: Window, bounds: WmRect) {
        if self.display.is_null()
            || window == 0
            || self.drawable == 0
            || !bounds.size().is_positive()
        {
            return;
        }
        unsafe {
            XCopyArea(
                self.display,
                self.drawable,
                window,
                self.gc,
                bounds.x,
                bounds.y,
                bounds.w as u32,
                bounds.h as u32,
                bounds.x,
                bounds.y,
            );
            XSync(self.display, 0);
        }
    }
}

// ── Color scheme ─────────────────────────────────────────────────────────────

impl DrawContext {
    /// Replace the active color scheme.
    pub fn set_scheme(&mut self, scheme: ColorScheme) {
        self.scheme = Some(scheme);
    }

    /// Read-only access to the active color scheme, if one is set.
    pub fn get_scheme(&self) -> Option<&ColorScheme> {
        self.scheme.as_ref()
    }

    /// Allocate a single color by name (e.g. `"#ff0000"` or `"red"`).
    pub fn clr_create(&self, clrname: &str) -> Result<Color, String> {
        if self.display.is_null() {
            return Err("X11 display not available".to_string());
        }
        let c_name = CString::new(clrname).map_err(|_| "Invalid color name")?;

        let mut color = XftColor {
            pixel: 0,
            color: XRenderColor {
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0xFFFF,
            },
        };

        unsafe {
            let ok = XftColorAllocName(
                self.display,
                self.visual,
                self.colormap,
                c_name.as_ptr(),
                &mut color,
            );
            if ok == 0 {
                return Err(format!("cannot allocate color '{}'", clrname));
            }
            // Xlib drops the high byte; restore the full alpha.
            color.pixel |= 0xff << 24;
        }

        Ok(Color { color })
    }

    pub fn clr_create_rgba(&self, rgba: crate::bar::color::Rgba) -> Color {
        let clamp = |v: f32| -> u16 {
            let v = v.clamp(0.0, 1.0);
            (v * 65535.0).round() as u16
        };
        let mut color = XftColor {
            pixel: 0,
            color: XRenderColor {
                red: clamp(rgba.r()),
                green: clamp(rgba.g()),
                blue: clamp(rgba.b()),
                alpha: clamp(rgba.a()),
            },
        };

        if self.display.is_null() {
            return Color { color };
        }

        unsafe {
            let mut render = color.color;
            let ok = XftColorAllocValue(
                self.display,
                self.visual,
                self.colormap,
                &mut render,
                &mut color,
            );
            if ok != 0 {
                color.pixel |= 0xff << 24;
            }
        }

        Color { color }
    }

    /// Allocate a color scheme from a slice of color name strings.
    ///
    /// Requires exactly 3 colors: foreground, background, detail.
    pub fn scm_create(&self, clrnames: &[&str]) -> Result<ColorScheme, String> {
        if clrnames.len() != 3 {
            return Err(format!(
                "scm_create requires exactly 3 colors (fg, bg, detail), got {}",
                clrnames.len()
            ));
        }
        let fg = self.clr_create(clrnames[0])?;
        let bg = self.clr_create(clrnames[1])?;
        let detail = self.clr_create(clrnames[2])?;
        Ok(ColorScheme { fg, bg, detail })
    }
}

// ── Cursor management ─────────────────────────────────────────────────────────

impl DrawContext {
    /// Create a cursor from one of the standard X11 cursor shapes.
    pub fn cur_create(&self, shape: u32) -> Cursor {
        if self.display.is_null() {
            return Cursor::new(0);
        }
        let cursor = unsafe { XCreateFontCursor(self.display, shape) };
        Cursor::new(cursor)
    }

    /// Free a cursor previously created with [`cur_create`].
    pub fn cur_free(&self, cursor: &Cursor) {
        if self.display.is_null() || cursor.cursor == 0 {
            return;
        }
        unsafe { XFreeCursor(self.display, cursor.cursor) };
    }
}

// ── Font / fontset management ─────────────────────────────────────────────────

impl DrawContext {
    /// Replace the active fontset.
    pub fn set_fontset(&mut self, font: Option<Vec<Fnt>>) {
        self.fonts = font;
    }

    /// Load a fontset from an ordered list of font name strings.
    ///
    /// Fonts are stored in the order given; the first font is tried first when
    /// rendering each glyph.  Returns a clone of `self.fonts` for convenience.
    pub fn fontset_create(&mut self, font_names: &[&str]) -> Result<Option<Vec<Fnt>>, String> {
        if self.display.is_null() {
            return Ok(None);
        }
        let mut fonts = Vec::new();
        for &name in font_names {
            if let Some(fnt) = self.load_font_by_name(name)? {
                fonts.push(fnt);
            }
        }
        if fonts.is_empty() {
            return Ok(None);
        }
        self.fonts = Some(fonts);
        Ok(self.fonts.clone())
    }

    /// Load a single font by name string, returning a [`Fnt`].
    fn load_font_by_name(&self, name: &str) -> Result<Option<Fnt>, String> {
        self.xfont_create(Some(name), None)
    }

    /// Core font-loading helper: either open by name or by Fontconfig pattern.
    ///
    /// Exactly one of `fontname` / `fontpattern` must be `Some`.
    pub(super) fn xfont_create(
        &self,
        fontname: Option<&str>,
        fontpattern: Option<*mut FcPattern>,
    ) -> Result<Option<Fnt>, String> {
        if self.display.is_null() {
            return Ok(None);
        }
        let xfont: *mut XftFont;
        let pattern: *mut FcPattern;
        let owns_pattern: bool;

        if let Some(name) = fontname {
            let c_name = CString::new(name).map_err(|_| "Invalid font name")?;
            xfont = unsafe { XftFontOpenName(self.display, self.screen, c_name.as_ptr()) };
            if xfont.is_null() {
                eprintln!("draw: cannot load font '{}'", name);
                return Ok(None);
            }
            pattern = unsafe { FcNameParse(c_name.as_ptr() as *const u8) };
            owns_pattern = true;
            if pattern.is_null() {
                eprintln!("draw: cannot parse font name '{}' to Fc pattern", name);
                unsafe { XftFontClose(self.display, xfont) };
                return Ok(None);
            }
        } else if let Some(pat) = fontpattern {
            xfont = unsafe { XftFontOpenPattern(self.display, pat) };
            if xfont.is_null() {
                eprintln!("draw: cannot load font from Fc pattern");
                return Ok(None);
            }
            pattern = pat;
            owns_pattern = false;
        } else {
            return Err("xfont_create: no font name or pattern provided".to_string());
        }

        let (ascent, descent) = unsafe { ((*xfont).ascent, (*xfont).descent) };

        Ok(Some(Fnt {
            display: self.display,
            h: (ascent + descent) as u32,
            xfont,
            pattern,
            owns_pattern,
            ascent,
            owns_resources: true,
        }))
    }
}

// ── Text metrics ─────────────────────────────────────────────────────────────

impl DrawContext {
    /// Return the pixel width of `text` rendered with the current fontset.
    ///
    /// Returns `0` if no fontset is loaded or `text` is empty.
    pub fn fontset_getwidth(&mut self, text: &str) -> u32 {
        if self.fonts.is_none() || text.is_empty() {
            return 0;
        }
        // `text(x=0, y=0, w=0, h=0, …)` measures without rendering.
        self.text(WmRect::default(), 0, text, false, 0) as u32
    }

    /// Like [`fontset_getwidth`] but clamped to at most `n` pixels.
    pub fn fontset_getwidth_clamp(&mut self, text: &str, n: u32) -> u32 {
        if self.fonts.is_none() || text.is_empty() || n == 0 {
            return 0;
        }
        min(n, self.fontset_getwidth(text))
    }

    /// Return the advance width (in pixels) of `text` rendered with `font`.
    pub fn font_getexts(&self, font: &Fnt, text: &[u8]) -> u32 {
        if text.is_empty() {
            return 0;
        }
        if self.display.is_null() {
            return 0;
        }
        let mut ext = zero_glyph_info();
        unsafe {
            XftTextExtentsUtf8(
                self.display,
                font.xfont,
                text.as_ptr(),
                text.len() as c_int,
                &mut ext,
            );
        }
        ext.x_off as u32
    }

    /// Return `(advance_width, line_height)` for `text` with `font`.
    pub fn font_getexts_h(&self, font: &Fnt, text: &[u8]) -> (u32, u32) {
        if text.is_empty() {
            return (0, font.h);
        }
        if self.display.is_null() {
            return (0, font.h);
        }
        let mut ext = zero_glyph_info();
        unsafe {
            XftTextExtentsUtf8(
                self.display,
                font.xfont,
                text.as_ptr(),
                text.len() as c_int,
                &mut ext,
            );
        }
        (ext.x_off as u32, font.h)
    }
}

// ── Primitive drawing ─────────────────────────────────────────────────────────

impl DrawContext {
    /// Fill or stroke a rectangle.
    ///
    /// * `filled` — fill if `true`, stroke outline if `false`.
    /// * `invert` — swap fg/bg colors.
    pub fn rect(&self, bounds: WmRect, filled: bool, invert: bool) {
        if self.display.is_null() || !bounds.size().is_positive() {
            return;
        }
        let Some(ref scheme) = self.scheme else {
            return;
        };

        let pixel = if invert {
            scheme.bg.pixel()
        } else {
            scheme.fg.pixel()
        };
        let WmRect { x, y, w, h } = bounds;
        let width = w as u32;
        let height = h as u32;

        unsafe {
            XSetForeground(self.display, self.gc, pixel as c_ulong);
            if filled {
                XFillRectangle(self.display, self.drawable, self.gc, x, y, width, height);
            } else {
                XDrawRectangle(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y,
                    width.saturating_sub(1),
                    height.saturating_sub(1),
                );
            }
        }
    }

    /// Fill or stroke an ellipse inscribed in the given bounding box.
    ///
    /// * `filled` — fill if `true`, stroke if `false`.
    /// * `invert` — swap fg/bg colors.
    pub fn circ(&self, bounds: WmRect, filled: bool, invert: bool) {
        if self.display.is_null() || !bounds.size().is_positive() {
            return;
        }
        let Some(ref scheme) = self.scheme else {
            return;
        };

        let pixel = if invert {
            scheme.bg.pixel()
        } else {
            scheme.fg.pixel()
        };
        let WmRect { x, y, w, h } = bounds;
        let width = w as u32;
        let height = h as u32;

        let (aw, ah) = if filled {
            (width, height)
        } else {
            (width.saturating_sub(1), height.saturating_sub(1))
        };

        unsafe {
            XSetForeground(self.display, self.gc, pixel as c_ulong);
            if filled {
                XFillArc(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y,
                    aw,
                    ah,
                    0,
                    360 * 64,
                );
            } else {
                XDrawArc(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y,
                    aw,
                    ah,
                    0,
                    360 * 64,
                );
            }
        }
    }

    /// Draw a filled arrow (triangle) pointing left or right.
    ///
    /// * `direction` — `true` = left-to-right, `false` = right-to-left.
    /// * `slash`     — if `true` the trailing edge is a vertical slash rather
    ///   than a horizontal midpoint.
    pub fn arrow(&self, bounds: WmRect, direction: bool, slash: bool) {
        if self.display.is_null() || !bounds.size().is_positive() {
            return;
        }
        let Some(ref scheme) = self.scheme else {
            return;
        };

        let (Ok(x), Ok(y), Ok(width), Ok(height)) = (
            i16::try_from(bounds.x),
            i16::try_from(bounds.y),
            i16::try_from(bounds.w),
            i16::try_from(bounds.h),
        ) else {
            return;
        };
        let origin_x = if direction { x } else { x + width };
        let delta_x = if direction { width } else { -width };
        let tip_y = if slash {
            if direction { 0 } else { height }
        } else {
            height / 2
        };

        unsafe {
            XSetForeground(self.display, self.gc, scheme.bg.pixel() as c_ulong);
            let mut pts = [
                Point { x: origin_x, y },
                Point {
                    x: (origin_x as i32 + delta_x as i32) as i16,
                    y: y + tip_y,
                },
                Point {
                    x: origin_x,
                    y: y + height,
                },
            ];
            XFillPolygon(
                self.display,
                self.drawable,
                self.gc,
                pts.as_mut_ptr(),
                3,
                1, // Complex shape
                0, // CoordModeOrigin
            );
        }
    }
}
