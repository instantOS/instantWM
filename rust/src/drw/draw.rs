//! The [`Drw`] drawing context.
//!
//! `Drw` owns an X11 display connection, a pixmap, a GC, and a fontset.
//! All bar rendering goes through this type.
//!
//! # Lifecycle
//!
//! ```text
//! Drw::new()          – open display, create pixmap + GC
//!   └─ fontset_create – load fonts into the linked-list fontset
//!   └─ set_scheme     – choose the active color scheme
//!   └─ text / rect / circ / arrow  – render into the pixmap
//!   └─ map            – blit the pixmap to the target window
//! Drop                – free pixmap, GC, close display
//! ```

use std::ffi::CString;
use std::os::raw::{c_int, c_ulong};
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use x11rb::protocol::xproto::{Drawable, Point, Window};

use crate::util::die;

use std::cmp::min;

use super::color::{Color, Cursor, COL_BG, COL_DETAIL, COL_FG};
use super::ffi::{
    FcCharSetAddChar, FcCharSetCreate, FcCharSetDestroy, FcConfigSubstitute, FcDefaultSubstitute,
    FcInit, FcNameParse, FcPattern, FcPatternAddBool, FcPatternAddCharSet, FcPatternDestroy,
    FcPatternDuplicate, XCloseDisplay, XCopyArea, XCreateFontCursor, XCreateGC, XCreatePixmap,
    XDefaultColormap, XDefaultDepth, XDefaultRootWindow, XDefaultScreen, XDefaultVisual, XDrawArc,
    XDrawRectangle, XFillArc, XFillPolygon, XFillRectangle, XFreeCursor, XFreeGC, XFreePixmap,
    XGlyphInfo, XOpenDisplay, XRenderColor, XSetForeground, XSetLineAttributes, XSync,
    XftCharExists, XftColor, XftColorAllocName, XftDraw, XftDrawCreate, XftDrawDestroy,
    XftDrawStringUtf8, XftFont, XftFontClose, XftFontMatch, XftFontOpenName, XftFontOpenPattern,
    XftInit, XftResult, XftTextExtentsUtf8, XlibGc, FC_CHARSET, FC_MATCH_PATTERN, FC_SCALABLE,
    FC_TRUE,
};
use super::font::Fnt;
use super::utf8::utf8decode;
use crate::types::ColorScheme;

/// How many "no-match" codepoints we remember to avoid repeatedly trying to
/// find a fallback font for the same unrenderable character.
const NOMATCHES_LEN: usize = 64;

/// Global ring-buffer index for the no-match cache (shared across all `Drw` instances).
static NOMATCHES_IDX: AtomicU32 = AtomicU32::new(0);

// ── Drw ──────────────────────────────────────────────────────────────────────

/// The main drawing context.
///
/// Wraps an Xlib display, a server-side pixmap used as an off-screen buffer,
/// a graphics context (GC), the active color scheme, and the fontset.
pub struct Drw {
    /// Pixmap / drawable width.
    pub w: u32,
    /// Pixmap / drawable height.
    pub h: u32,

    pub(super) display: *mut libc::c_void,
    pub(super) screen: i32,
    pub(super) root: Window,

    /// Off-screen pixmap — all drawing happens here, then [`Drw::map`] blits
    /// it to the real window.
    pub(super) drawable: Drawable,
    pub(super) gc: XlibGc,

    /// Active color scheme (`[fg, bg, detail]` slots).
    scheme: Option<Vec<Color>>,

    /// Loaded fontset (linked list, head node).
    pub fonts: Option<Box<Fnt>>,

    depth: u8,
    visual: *mut libc::c_void,
    colormap: c_ulong,

    /// Ring buffer of codepoints for which no fallback font was found.
    nomatches: [u32; NOMATCHES_LEN],

    /// Cached pixel width of `"..."` for the current fontset.
    ellipsis_width: u32,

    /// `true` only for the *original* `Drw` — clones do **not** own resources.
    owns_resources: bool,
}

// SAFETY: instantWM is single-threaded; no concurrent mutation.
unsafe impl Send for Drw {}
unsafe impl Sync for Drw {}

impl Clone for Drw {
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
            nomatches: self.nomatches,
            ellipsis_width: self.ellipsis_width,
            owns_resources: false,
        }
    }
}

impl Drop for Drw {
    fn drop(&mut self) {
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

impl Drw {
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
                nomatches: [0; NOMATCHES_LEN],
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

impl Drw {
    /// Resize the off-screen pixmap to `w × h` pixels.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
        unsafe {
            XFreePixmap(self.display, self.drawable);
            self.drawable = XCreatePixmap(self.display, self.root, w, h, self.depth as u32);
        }
    }

    /// Blit the off-screen pixmap to `win` at position `(x, y)` with size `w × h`.
    pub fn map(&self, win: Window, x: i16, y: i16, w: u16, h: u16) {
        unsafe {
            XCopyArea(
                self.display,
                self.drawable,
                win,
                self.gc,
                x as c_int,
                y as c_int,
                w as u32,
                h as u32,
                x as c_int,
                y as c_int,
            );
            XSync(self.display, 0);
        }
    }
}

// ── Color scheme ─────────────────────────────────────────────────────────────

impl Drw {
    /// Replace the active color scheme.
    pub fn set_scheme(&mut self, scheme: ColorScheme) {
        self.scheme = Some(scheme.as_vec());
    }

    /// Read-only access to the active color scheme slice, if one is set.
    pub fn get_scheme(&self) -> Option<&Vec<Color>> {
        self.scheme.as_ref()
    }

    /// Allocate a single color by name (e.g. `"#ff0000"` or `"red"`).
    pub fn clr_create(&self, clrname: &str) -> Result<Color, String> {
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

    /// Allocate a color scheme from a slice of color name strings.
    ///
    /// The slice must have at least two entries (`[fg, bg, ...]`).
    pub fn scm_create(&self, clrnames: &[&str]) -> Result<Vec<Color>, String> {
        if clrnames.len() < 2 {
            return Err("need at least two colors for a scheme".to_string());
        }
        clrnames.iter().map(|name| self.clr_create(name)).collect()
    }
}

// ── Cursor management ─────────────────────────────────────────────────────────

impl Drw {
    /// Create a cursor from one of the standard X11 cursor shapes.
    pub fn cur_create(&self, shape: u32) -> Cursor {
        let cursor = unsafe { XCreateFontCursor(self.display, shape) };
        Cursor::new(cursor)
    }

    /// Free a cursor previously created with [`cur_create`].
    pub fn cur_free(&self, cursor: &Cursor) {
        unsafe { XFreeCursor(self.display, cursor.cursor) };
    }
}

// ── Font / fontset management ─────────────────────────────────────────────────

impl Drw {
    /// Replace the active fontset.
    pub fn set_fontset(&mut self, font: Option<Box<Fnt>>) {
        self.fonts = font;
    }

    /// Load a fontset from an ordered list of font name strings.
    ///
    /// Fonts are stored in the order given; the first font is tried first when
    /// rendering each glyph.  Returns a clone of `self.fonts` for convenience.
    pub fn fontset_create(&mut self, fonts: &[&str]) -> Result<Option<Box<Fnt>>, String> {
        // Load in reverse so prepending produces the correct order.
        let mut head: Option<Box<Fnt>> = None;
        for &name in fonts.iter().rev() {
            if let Some(mut fnt) = self.load_font_by_name(name)? {
                fnt.next = head;
                head = Some(fnt);
            }
        }
        self.fonts = head;
        Ok(self.fonts.clone())
    }

    /// Load a single font by name string, returning a heap-allocated [`Fnt`].
    fn load_font_by_name(&self, name: &str) -> Result<Option<Box<Fnt>>, String> {
        self.xfont_create(Some(name), None)
    }

    /// Core font-loading helper: either open by name or by Fontconfig pattern.
    ///
    /// Exactly one of `fontname` / `fontpattern` must be `Some`.
    pub(super) fn xfont_create(
        &self,
        fontname: Option<&str>,
        fontpattern: Option<*mut FcPattern>,
    ) -> Result<Option<Box<Fnt>>, String> {
        let xfont: *mut XftFont;
        let pattern: *mut FcPattern;

        if let Some(name) = fontname {
            let c_name = CString::new(name).map_err(|_| "Invalid font name")?;
            xfont = unsafe { XftFontOpenName(self.display, self.screen, c_name.as_ptr()) };
            if xfont.is_null() {
                eprintln!("drw: cannot load font '{}'", name);
                return Ok(None);
            }
            pattern = unsafe { FcNameParse(c_name.as_ptr() as *const u8) };
            if pattern.is_null() {
                eprintln!("drw: cannot parse font name '{}' to Fc pattern", name);
                unsafe { XftFontClose(self.display, xfont) };
                return Ok(None);
            }
        } else if let Some(pat) = fontpattern {
            xfont = unsafe { XftFontOpenPattern(self.display, pat) };
            if xfont.is_null() {
                eprintln!("drw: cannot load font from Fc pattern");
                return Ok(None);
            }
            pattern = pat;
        } else {
            return Err("xfont_create: no font name or pattern provided".to_string());
        }

        let (ascent, descent) = unsafe { ((*xfont).ascent, (*xfont).descent) };

        Ok(Some(Box::new(Fnt {
            display: self.display,
            h: (ascent + descent) as u32,
            xfont,
            pattern,
            next: None,
            ascent,
            owns_resources: true,
        })))
    }
}

// ── Text metrics ─────────────────────────────────────────────────────────────

impl Drw {
    /// Return the pixel width of `text` rendered with the current fontset.
    ///
    /// Returns `0` if no fontset is loaded or `text` is empty.
    pub fn fontset_getwidth(&mut self, text: &str) -> u32 {
        if self.fonts.is_none() || text.is_empty() {
            return 0;
        }
        // `text(x=0, y=0, w=0, h=0, …)` measures without rendering.
        self.text(0, 0, 0, 0, 0, text, false, 0) as u32
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

impl Drw {
    /// Fill or stroke a rectangle.
    ///
    /// * `filled` — fill if `true`, stroke outline if `false`.
    /// * `invert` — swap fg/bg colors.
    pub fn rect(&self, x: i32, y: i32, w: u32, h: u32, filled: bool, invert: bool) {
        let Some(ref scheme) = self.scheme else {
            return;
        };

        let pixel = if invert {
            scheme[COL_BG].pixel()
        } else {
            scheme[COL_FG].pixel()
        };

        unsafe {
            XSetForeground(self.display, self.gc, pixel as c_ulong);
            if filled {
                XFillRectangle(self.display, self.drawable, self.gc, x, y, w, h);
            } else {
                XDrawRectangle(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y,
                    w.saturating_sub(1),
                    h.saturating_sub(1),
                );
            }
        }
    }

    /// Fill or stroke an ellipse inscribed in the given bounding box.
    ///
    /// * `filled` — fill if `true`, stroke if `false`.
    /// * `invert` — swap fg/bg colors.
    pub fn circ(&self, x: i32, y: i32, w: u32, h: u32, filled: bool, invert: bool) {
        let Some(ref scheme) = self.scheme else {
            return;
        };

        let pixel = if invert {
            scheme[COL_BG].pixel()
        } else {
            scheme[COL_FG].pixel()
        };

        let (aw, ah) = if filled {
            (w, h)
        } else {
            (w.saturating_sub(1), h.saturating_sub(1))
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
    ///                 than a horizontal midpoint.
    pub fn arrow(&self, x: i16, y: i16, w: u16, h: u16, direction: bool, slash: bool) {
        let Some(ref scheme) = self.scheme else {
            return;
        };

        let origin_x = if direction { x } else { x + w as i16 };
        let delta_x = if direction { w as i16 } else { -(w as i16) };
        let tip_y = if slash {
            if direction {
                0
            } else {
                h as i16
            }
        } else {
            h as i16 / 2
        };

        unsafe {
            XSetForeground(self.display, self.gc, scheme[COL_BG].pixel() as c_ulong);
            let mut pts = [
                Point { x: origin_x, y },
                Point {
                    x: (origin_x as i32 + delta_x as i32) as i16,
                    y: y + tip_y,
                },
                Point {
                    x: origin_x,
                    y: y + h as i16,
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

// ── Text rendering ────────────────────────────────────────────────────────────

impl Drw {
    /// Render `text` into the rectangle `(x, y, w, h)`.
    ///
    /// When `x == 0 && y == 0 && w == 0 && h == 0` the function only measures
    /// the text and returns the advance width without drawing anything.
    ///
    /// # Parameters
    ///
    /// * `lpad`          — horizontal padding added before the first glyph.
    /// * `invert`        — swap fg/bg colors.
    /// * `detail_height` — if `> 0`, the bottom `detail_height` pixels of the
    ///                     background are painted in the *detail* color.
    ///                     Pass `0` for a plain background, `-1` to skip the
    ///                     ellipsis-width calculation (used when drawing `"..."`
    ///                     itself to avoid infinite recursion).
    ///
    /// # Returns
    ///
    /// * In **render** mode (`w > 0`): `x + remaining_width` (the x position
    ///   just past the drawn area, suitable for chaining draw calls).
    /// * In **measure** mode (`w == 0`): total advance width of the text.
    pub fn text(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        lpad: u32,
        text: &str,
        invert: bool,
        detail_height: i32,
    ) -> i32 {
        if self.fonts.is_none() || text.is_empty() {
            return 0;
        }

        let mut x = x;
        let mut w = w;

        // ── Measure-only mode ────────────────────────────────────────────────
        // Measuring text width requires only the fontset, not a color scheme.
        let mut render = x != 0 || y != 0 || w != 0 || h != 0;
        if !render {
            w = u32::MAX;
        }

        // ── Extract scheme colors (render path only) ─────────────────────────
        let (fg_pixel, bg_pixel, detail_pixel, fg_color, bg_color);
        if render {
            let Some(ref scheme) = self.scheme else {
                return 0;
            };
            fg_pixel = scheme[COL_FG].pixel();
            bg_pixel = scheme[COL_BG].pixel();
            detail_pixel = scheme[COL_DETAIL].pixel();
            fg_color = scheme[COL_FG].color.clone();
            bg_color = scheme[COL_BG].color.clone();
        } else {
            fg_pixel = 0;
            bg_pixel = 0;
            detail_pixel = 0;
            fg_color = unsafe { std::mem::zeroed() };
            bg_color = unsafe { std::mem::zeroed() };
        }

        // ── Lazy-initialise ellipsis width ───────────────────────────────────
        // Skip when `detail_height < 0` — that signals we are *drawing* the
        // ellipsis itself and must not recurse.
        if self.ellipsis_width == 0 && render && detail_height >= 0 {
            self.ellipsis_width = self.fontset_getwidth("...");
        }

        // ── Prepare background + Xft draw surface ────────────────────────────
        let mut d: *mut XftDraw = ptr::null_mut();

        if render {
            unsafe {
                let bg = if invert { fg_pixel } else { bg_pixel };
                XSetForeground(self.display, self.gc, bg as c_ulong);

                if detail_height > 0 {
                    // Main background (above the detail strip).
                    XFillRectangle(
                        self.display,
                        self.drawable,
                        self.gc,
                        x,
                        y,
                        w,
                        h.saturating_sub(detail_height as u32),
                    );
                    // Colored detail strip at the bottom.
                    XSetForeground(self.display, self.gc, detail_pixel as c_ulong);
                    XFillRectangle(
                        self.display,
                        self.drawable,
                        self.gc,
                        x,
                        y + h as i32 - detail_height,
                        w,
                        detail_height as u32,
                    );
                } else {
                    XFillRectangle(self.display, self.drawable, self.gc, x, y, w, h);
                }

                d = XftDrawCreate(self.display, self.drawable, self.visual, self.colormap);
            }

            if d.is_null() {
                // Fallback to measure-only if Xft surface creation failed.
                render = false;
                w = u32::MAX;
            } else {
                x += lpad as i32;
                w = w.saturating_sub(lpad);
            }
        }

        // ── Main text rendering loop ─────────────────────────────────────────
        let text_bytes = text.as_bytes();
        let mut text_pos: usize = 0;

        let mut usedfont_idx: usize = 0;
        let mut charexists = false;

        // Ellipsis tracking — when text overflows we draw "..." at `ellipsis_x`.
        let mut ellipsis_x: i32 = 0;
        let mut ellipsis_w: u32 = 0;
        let mut overflow = false;

        loop {
            let mut ew: u32 = 0;
            let mut ellipsis_len: usize = 0;
            let mut utf8strlen: usize = 0;
            let utf8str_start = text_pos; // byte offset of this font-run's start
            let mut nextfont_idx: Option<usize> = None;
            let mut utf8codepoint: u32 = 0;

            // ── Walk codepoints in the current font run ──────────────────────
            while text_pos < text_bytes.len() {
                let (charlen, codepoint) = utf8decode(&text_bytes[text_pos..]);
                utf8codepoint = codepoint;

                // Find which font in the fallback chain can render this char.
                while !charexists {
                    let font_count = self.fonts.as_ref().unwrap().count();
                    for cur_idx in 0..font_count {
                        let cur_font = self.fonts.as_ref().unwrap().get(cur_idx).unwrap();

                        charexists = unsafe {
                            XftCharExists(self.display, cur_font.xfont, utf8codepoint) != 0
                        };

                        if charexists {
                            let slice_end = (text_pos + charlen).min(text_bytes.len());
                            let glyph_bytes = &text_bytes[text_pos..slice_end];
                            let tmpw = self.font_getexts(cur_font, glyph_bytes);

                            // Update ellipsis position if there is still room.
                            if ew + self.ellipsis_width <= w {
                                ellipsis_x = x + ew as i32;
                                ellipsis_w = w.saturating_sub(ew);
                                ellipsis_len = utf8strlen;
                            }

                            if ew + tmpw > w {
                                // This glyph would overflow — stop the run here.
                                overflow = true;
                                if !render {
                                    x += tmpw as i32;
                                } else {
                                    utf8strlen = ellipsis_len;
                                }
                            } else if cur_idx == usedfont_idx {
                                // Same font — extend the current run.
                                utf8strlen += charlen;
                                text_pos += charlen;
                                ew += tmpw;
                            } else {
                                // Different font — end the current run; the
                                // outer loop will start a new one.
                                nextfont_idx = Some(cur_idx);
                            }
                            break;
                        }
                    }

                    if !charexists {
                        break;
                    }
                }

                if overflow || !charexists || nextfont_idx.is_some() {
                    break;
                }
                charexists = false;
            }

            // ── Render the accumulated run ────────────────────────────────────
            if utf8strlen > 0 {
                if render {
                    let f = self.fonts.as_ref().unwrap().get(usedfont_idx).unwrap();
                    let ty = y + ((h as i32 - f.h as i32) / 2) + f.ascent();

                    let run_bytes = &text_bytes[utf8str_start..utf8str_start + utf8strlen];
                    unsafe {
                        XftDrawStringUtf8(
                            d,
                            if invert { &bg_color } else { &fg_color } as *const XftColor,
                            f.xfont,
                            x as c_int,
                            ty as c_int,
                            run_bytes.as_ptr(),
                            utf8strlen as c_int,
                        );
                    }
                }
                x += ew as i32;
                w = w.saturating_sub(ew);
            }

            // Draw the ellipsis if we overflowed (but guard against recursion).
            if render && overflow && text != "..." {
                self.draw_ellipsis(ellipsis_x, y, ellipsis_w, h, invert, detail_height);
            }

            if text_pos >= text_bytes.len() || overflow {
                break;
            }

            // ── Advance to the next font run ──────────────────────────────────
            if let Some(next_idx) = nextfont_idx {
                charexists = false;
                usedfont_idx = next_idx;
            } else {
                // No font in the set could render the codepoint — try dynamic
                // fallback via Fontconfig.
                charexists = true;

                if !self.is_nomatch(utf8codepoint) {
                    self.try_load_fallback_font(utf8codepoint, &mut usedfont_idx);
                } else {
                    usedfont_idx = 0;
                }
            }
        }

        // ── Tear down Xft draw surface ────────────────────────────────────────
        if !d.is_null() {
            unsafe { XftDrawDestroy(d) };
        }

        x + if render { w as i32 } else { 0 }
    }

    // ── Internal helpers for `text` ───────────────────────────────────────────

    /// Draw `"..."` at `(x, y)` within `w × h`, used when text overflows.
    ///
    /// Passes `detail_height = -1` to suppress recursive ellipsis measurement.
    fn draw_ellipsis(&mut self, x: i32, y: i32, w: u32, h: u32, invert: bool, detail_height: i32) {
        // detail_height == -1 prevents a further recursive call back here.
        let dh = if detail_height >= 0 { detail_height } else { 0 };
        self.text(x, y, w, h, 0, "...", invert, -1.max(dh - 1));
    }

    /// Return `true` if `codepoint` is in the no-match cache.
    fn is_nomatch(&self, codepoint: u32) -> bool {
        self.nomatches.contains(&codepoint)
    }

    /// Attempt to find a Fontconfig fallback font that contains `codepoint`.
    ///
    /// On success the new font is appended to `self.fonts` and `usedfont_idx`
    /// is updated to point at it.  On failure the codepoint is recorded in the
    /// no-match cache and `usedfont_idx` is reset to 0.
    fn try_load_fallback_font(&mut self, codepoint: u32, usedfont_idx: &mut usize) {
        unsafe {
            let fccharset = FcCharSetCreate();
            FcCharSetAddChar(fccharset, codepoint);

            let fonts_ref = self.fonts.as_ref().unwrap();
            if fonts_ref.pattern.is_null() {
                die("drw: the first font in the cache must be loaded from a font name string.");
            }

            let fcpattern = FcPatternDuplicate(fonts_ref.pattern);
            FcPatternAddCharSet(fcpattern, FC_CHARSET.as_ptr(), fccharset);
            FcPatternAddBool(fcpattern, FC_SCALABLE.as_ptr(), FC_TRUE);
            FcConfigSubstitute(ptr::null_mut(), fcpattern, FC_MATCH_PATTERN);
            FcDefaultSubstitute(fcpattern);

            let mut result: XftResult = 0;
            let match_pattern = XftFontMatch(self.display, self.screen, fcpattern, &mut result);

            FcCharSetDestroy(fccharset);
            FcPatternDestroy(fcpattern);

            if match_pattern.is_null() {
                // No fallback found.
                let idx = NOMATCHES_IDX.fetch_add(1, Ordering::SeqCst);
                self.nomatches[idx as usize % NOMATCHES_LEN] = codepoint;
                *usedfont_idx = 0;
                return;
            }

            match self.xfont_create(None, Some(match_pattern)) {
                Ok(Some(new_font)) => {
                    if XftCharExists(self.display, new_font.xfont, codepoint) != 0 {
                        *usedfont_idx = self.fonts.as_ref().unwrap().count();
                        self.fonts.as_mut().unwrap().push_back(new_font);
                    } else {
                        drop(new_font);
                        let idx = NOMATCHES_IDX.fetch_add(1, Ordering::SeqCst);
                        self.nomatches[idx as usize % NOMATCHES_LEN] = codepoint;
                        *usedfont_idx = 0;
                    }
                }
                _ => {
                    let idx = NOMATCHES_IDX.fetch_add(1, Ordering::SeqCst);
                    self.nomatches[idx as usize % NOMATCHES_LEN] = codepoint;
                    *usedfont_idx = 0;
                }
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn zero_glyph_info() -> XGlyphInfo {
    XGlyphInfo {
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        x_off: 0,
        y_off: 0,
    }
}
