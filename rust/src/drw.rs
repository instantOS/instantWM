use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_ulong};
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use x11rb::protocol::xproto::{Drawable, Point, Window};

use crate::util::{between, die, min};

pub const UTF_INVALID: u32 = 0xFFFD;
pub const UTF_SIZ: usize = 4;

pub const UTFBYTE: [u8; UTF_SIZ + 1] = [0x80, 0, 0xC0, 0xE0, 0xF0];
pub const UTFMASK: [u8; UTF_SIZ + 1] = [0xC0, 0x80, 0xE0, 0xF0, 0xF8];
pub const UTFMIN: [u32; UTF_SIZ + 1] = [0, 0, 0x80, 0x800, 0x10000];
pub const UTFMAX: [u32; UTF_SIZ + 1] = [0x10FFFF, 0x7F, 0x7FF, 0xFFFF, 0x10FFFF];

pub const COL_FG: usize = 0;
pub const COL_BG: usize = 1;
pub const COL_DETAIL: usize = 2;
pub const COL_LAST: usize = 3;

const NOMATCHES_LEN: usize = 64;

#[repr(C)]
pub struct XftColor {
    pub pixel: c_ulong,
    pub color: XRenderColor,
}

impl Clone for XftColor {
    fn clone(&self) -> Self {
        Self {
            pixel: self.pixel,
            color: XRenderColor {
                red: self.color.red,
                green: self.color.green,
                blue: self.color.blue,
                alpha: self.color.alpha,
            },
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

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct XRenderColor {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub alpha: u16,
}

#[repr(C)]
pub struct XftFont {
    pub ascent: c_int,
    pub descent: c_int,
    pub height: c_int,
    pub max_advance_width: c_int,
    pub charset: *mut libc::c_void,
    pub pattern: *mut libc::c_void,
}

#[repr(C)]
pub struct XftDraw {
    _private: [u8; 0],
}

#[repr(C)]
pub struct XGlyphInfo {
    pub width: u16,
    pub height: u16,
    pub x: i16,
    pub y: i16,
    pub xOff: i16,
    pub yOff: i16,
}

#[repr(C)]
pub struct FcPattern {
    _private: [u8; 0],
}

#[repr(C)]
pub struct FcCharSet {
    _private: [u8; 0],
}

pub type FcBool = c_int;
pub type FcResult = c_int;
pub type XftResult = c_int;
pub type XlibGc = *mut libc::c_void;

pub const FC_CHARSET: &[u8] = b"charset\0";
pub const FC_SCALABLE: &[u8] = b"scalable\0";

pub const FcMatchPattern: c_int = 1;
pub const FcTrue: FcBool = 1;

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
}

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

#[derive(Debug, Clone)]
pub struct Clr {
    pub color: XftColor,
}

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
    pub fn pixel(&self) -> u32 {
        self.color.pixel as u32
    }
}

#[derive(Debug)]
pub struct Cur {
    pub cursor: u32,
}

unsafe impl Send for Cur {}
unsafe impl Sync for Cur {}

impl Cur {
    pub fn new(cursor: u32) -> Self {
        Self { cursor }
    }
}

pub struct Fnt {
    display: *mut libc::c_void,
    pub h: u32,
    pub xfont: *mut XftFont,
    pub pattern: *mut FcPattern,
    pub next: Option<Box<Fnt>>,
    ascent: i32,
    owns_resources: bool,
}

unsafe impl Send for Fnt {}
unsafe impl Sync for Fnt {}

impl Clone for Fnt {
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
    pub fn height(&self) -> u32 {
        self.h
    }
}

impl Drop for Fnt {
    fn drop(&mut self) {
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

static NOMATCHES_IDX: AtomicU32 = AtomicU32::new(0);

pub struct Drw {
    pub w: u32,
    pub h: u32,
    display: *mut libc::c_void,
    screen: i32,
    root: Window,
    drawable: Drawable,
    gc: XlibGc,
    scheme: Option<Vec<Clr>>,
    pub fonts: Option<Box<Fnt>>,
    depth: u8,
    visual: *mut libc::c_void,
    colormap: c_ulong,
    nomatches: [u32; NOMATCHES_LEN],
    ellipsis_width: u32,
    owns_resources: bool,
}

impl Clone for Drw {
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

unsafe impl Send for Drw {}
unsafe impl Sync for Drw {}

impl Drw {
    pub fn new(display_name: Option<&str>) -> Result<Self, String> {
        eprintln!("TRACE: Drw::new - before FcInit");
        unsafe {
            FcInit();
        }
        eprintln!("TRACE: Drw::new - after FcInit, before XftInit");
        unsafe {
            XftInit();
        }
        eprintln!("TRACE: Drw::new - after XftInit, before XOpenDisplay");
        let display = unsafe {
            let name_ptr = display_name
                .and_then(|s| CString::new(s).ok())
                .map(|cs| cs.as_ptr())
                .unwrap_or(ptr::null());
            XOpenDisplay(name_ptr)
        };

        if display.is_null() {
            return Err("cannot open display".to_string());
        }
        eprintln!("TRACE: Drw::new - after XOpenDisplay");

        eprintln!("TRACE: Drw::new - entering unsafe block");
        unsafe {
            eprintln!("TRACE: Drw::new - before XDefaultScreen");
            let screen = XDefaultScreen(display);
            eprintln!("TRACE: Drw::new - screen = {}", screen);
            let root = XDefaultRootWindow(display);
            eprintln!("TRACE: Drw::new - root = {}", root);
            if root == 0 {
                XCloseDisplay(display);
                return Err("cannot get root window".to_string());
            }

            eprintln!("TRACE: Drw::new - before XDefaultVisual");
            let visual = XDefaultVisual(display, screen);
            eprintln!("TRACE: Drw::new - visual = {:p}", visual);
            if visual.is_null() {
                XCloseDisplay(display);
                return Err("cannot get default visual".to_string());
            }

            eprintln!("TRACE: Drw::new - before XDefaultColormap");
            let colormap = XDefaultColormap(display, screen);
            eprintln!("TRACE: Drw::new - colormap = {}", colormap);
            if colormap == 0 {
                XCloseDisplay(display);
                return Err("cannot get default colormap".to_string());
            }

            eprintln!("TRACE: Drw::new - before XDefaultDepth");
            let depth = XDefaultDepth(display, screen);
            eprintln!("TRACE: Drw::new - depth = {}", depth);
            if depth <= 0 {
                XCloseDisplay(display);
                return Err("cannot get default depth".to_string());
            }

            let w = 1;
            let h = 1;

            eprintln!("TRACE: Drw::new - before XCreatePixmap");
            let drawable = XCreatePixmap(display, root, w, h, depth as u32);
            eprintln!("TRACE: Drw::new - drawable = {}", drawable);
            if drawable == 0 {
                XCloseDisplay(display);
                return Err("cannot create pixmap".to_string());
            }

            eprintln!("TRACE: Drw::new - before XCreateGC");
            let gc = XCreateGC(display, root, 0, ptr::null_mut());
            eprintln!("TRACE: Drw::new - gc = {:p}", gc);
            if gc.is_null() {
                XFreePixmap(display, drawable);
                XCloseDisplay(display);
                return Err("cannot create graphics context".to_string());
            }

            eprintln!("TRACE: Drw::new - before XSetLineAttributes");
            XSetLineAttributes(display, gc, 1, 0, 0, 0);
            eprintln!("TRACE: Drw::new - before creating Self");

            Ok(Self {
                w,
                h,
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

    pub fn display(&self) -> *mut libc::c_void {
        self.display
    }

    pub fn screen(&self) -> i32 {
        self.screen
    }

    pub fn root(&self) -> Window {
        self.root
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;

        eprintln!("TRACE: Drw::resize - w={}, h={}", w, h);
        unsafe {
            XFreePixmap(self.display, self.drawable);
            self.drawable = XCreatePixmap(self.display, self.root, w, h, self.depth as u32);
            eprintln!("TRACE: Drw::resize - new drawable = {}", self.drawable);
        }
    }

    pub fn set_scheme(&mut self, scheme: Vec<Clr>) {
        self.scheme = Some(scheme);
    }

    pub fn get_scheme(&self) -> Option<&Vec<Clr>> {
        self.scheme.as_ref()
    }

    pub fn set_fontset(&mut self, font: Option<Box<Fnt>>) {
        self.fonts = font;
    }

    pub fn fontset_create(&mut self, fonts: &[&str]) -> Result<Option<Box<Fnt>>, String> {
        let mut ret: Option<Box<Fnt>> = None;

        for fontname in fonts.iter().rev() {
            if let Some(mut font) = self.xfont_create(Some(fontname), None)? {
                font.next = ret;
                ret = Some(font);
            }
        }

        self.fonts = ret;
        Ok(self.fonts.clone())
    }

    fn xfont_create(
        &mut self,
        fontname: Option<&str>,
        fontpattern: Option<*mut FcPattern>,
    ) -> Result<Option<Box<Fnt>>, String> {
        let mut xfont: *mut XftFont = ptr::null_mut();
        let mut pattern: *mut FcPattern = ptr::null_mut();

        if let Some(name) = fontname {
            let c_name = CString::new(name).map_err(|_| "Invalid font name")?;

            unsafe {
                xfont = XftFontOpenName(self.display, self.screen, c_name.as_ptr());
            }

            if xfont.is_null() {
                eprintln!("error, cannot load font from name: '{}'", name);
                return Ok(None);
            }

            unsafe {
                pattern = FcNameParse(c_name.as_ptr() as *const u8);
            }

            if pattern.is_null() {
                eprintln!("error, cannot parse font name to pattern: '{}'", name);
                unsafe {
                    XftFontClose(self.display, xfont);
                }
                return Ok(None);
            }
        } else if let Some(pat) = fontpattern {
            unsafe {
                xfont = XftFontOpenPattern(self.display, pat);
            }

            if xfont.is_null() {
                eprintln!("error, cannot load font from pattern.");
                return Ok(None);
            }
            pattern = pat;
        } else {
            return Err("no font specified.".to_string());
        }

        let (ascent, descent) = unsafe {
            let ascent = (*xfont).ascent;
            let descent = (*xfont).descent;
            (ascent, descent)
        };

        let font = Box::new(Fnt {
            display: self.display,
            h: (ascent + descent) as u32,
            xfont,
            pattern,
            next: None,
            ascent,
            owns_resources: true,
        });

        Ok(Some(font))
    }

    pub fn clr_create(&self, clrname: &str) -> Result<Clr, String> {
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
            let result = XftColorAllocName(
                self.display,
                self.visual,
                self.colormap,
                c_name.as_ptr(),
                &mut color,
            );

            if result == 0 {
                return Err(format!("error, cannot allocate color '{}'", clrname));
            }

            color.pixel |= 0xff << 24;
        }

        Ok(Clr { color })
    }

    pub fn scm_create(&self, clrnames: &[&str]) -> Result<Vec<Clr>, String> {
        if clrnames.len() < 2 {
            return Err("need at least two colors for a scheme".to_string());
        }

        let mut scheme = Vec::with_capacity(clrnames.len());
        for name in clrnames {
            scheme.push(self.clr_create(name)?);
        }

        Ok(scheme)
    }

    pub fn cur_create(&self, shape: u32) -> Cur {
        unsafe {
            let cursor = XCreateFontCursor(self.display, shape);
            Cur::new(cursor)
        }
    }

    pub fn cur_free(&self, cursor: &Cur) {
        unsafe {
            XFreeCursor(self.display, cursor.cursor);
        }
    }

    pub fn rect(&self, x: i32, y: i32, w: u32, h: u32, filled: bool, invert: bool) {
        let scheme = match &self.scheme {
            Some(s) => s,
            None => return,
        };

        let fg_pixel = if invert {
            scheme[COL_BG].pixel()
        } else {
            scheme[COL_FG].pixel()
        };

        unsafe {
            XSetForeground(self.display, self.gc, fg_pixel as c_ulong);

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

    pub fn circ(&self, x: i32, y: i32, w: u32, h: u32, filled: bool, invert: bool) {
        let scheme = match &self.scheme {
            Some(s) => s,
            None => return,
        };

        let fg_pixel = if invert {
            scheme[COL_BG].pixel()
        } else {
            scheme[COL_FG].pixel()
        };

        unsafe {
            XSetForeground(self.display, self.gc, fg_pixel as c_ulong);

            let (width, height) = if filled {
                (w, h)
            } else {
                (w.saturating_sub(1), h.saturating_sub(1))
            };

            if filled {
                XFillArc(
                    self.display,
                    self.drawable,
                    self.gc,
                    x,
                    y,
                    width,
                    height,
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
                    width,
                    height,
                    0,
                    360 * 64,
                );
            }
        }
    }

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
        let mut x = x;
        let mut w = w;
        let mut ellipsis_x = 0;
        let mut tmpw: u32;
        let mut ew: u32;
        let mut ellipsis_w: u32 = 0;
        let mut ellipsis_len: usize = 0;

        let mut d: *mut XftDraw = ptr::null_mut();
        let mut usedfont_idx: usize = 0;
        let mut nextfont_idx: Option<usize> = None;
        let mut utf8strlen: usize;
        let mut render = x != 0 || y != 0 || w != 0 || h != 0;
        let mut utf8codepoint: u32 = 0;
        let mut utf8str: &[u8];
        let mut charexists = false;
        let mut overflow = false;

        if self.fonts.is_none() {
            return 0;
        }

        let (fg_pixel, bg_pixel, detail_pixel) = match &self.scheme {
            Some(s) => (s[COL_FG].pixel(), s[COL_BG].pixel(), s[COL_DETAIL].pixel()),
            None => return 0,
        };

        let fg_color = match &self.scheme {
            Some(s) => s[COL_FG].color.clone(),
            None => return 0,
        };
        let bg_color = match &self.scheme {
            Some(s) => s[COL_BG].color.clone(),
            None => return 0,
        };

        if text.is_empty() {
            return 0;
        }

        if !render {
            w = u32::MAX;
        } else {
            unsafe {
                XSetForeground(
                    self.display,
                    self.gc,
                    if invert { fg_pixel } else { bg_pixel } as c_ulong,
                );

                if detail_height > 0 {
                    XFillRectangle(
                        self.display,
                        self.drawable,
                        self.gc,
                        x,
                        y,
                        w,
                        h.saturating_sub(detail_height as u32),
                    );

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
                render = false;
                w = u32::MAX;
            } else {
                x += lpad as i32;
                w = w.saturating_sub(lpad);
            }
        }

        if self.ellipsis_width == 0 && render && detail_height >= 0 {
            self.ellipsis_width = self.fontset_getwidth("...");
        }

        let text_bytes = text.as_bytes();
        let mut text_pos: usize = 0;

        fn get_font_at(mut font: &Fnt, idx: usize) -> Option<&Fnt> {
            for _ in 0..idx {
                match &font.next {
                    Some(next) => font = next,
                    None => return None,
                }
            }
            Some(font)
        }

        fn count_fonts(font: &Fnt) -> usize {
            let mut count = 1;
            let mut current = font;
            while let Some(next) = &current.next {
                count += 1;
                current = next;
            }
            count
        }

        loop {
            ew = 0;
            ellipsis_len = 0;
            utf8strlen = 0;
            utf8str = &text_bytes[text_pos..];
            nextfont_idx = None;

            while text_pos < text_bytes.len() {
                let (charlen, codepoint) = utf8decode(&text_bytes[text_pos..]);
                utf8codepoint = codepoint;

                while !charexists {
                    let font_count = count_fonts(self.fonts.as_ref().unwrap());
                    for cur_idx in 0..font_count {
                        let cur_font = get_font_at(self.fonts.as_ref().unwrap(), cur_idx).unwrap();

                        unsafe {
                            charexists =
                                XftCharExists(self.display, cur_font.xfont, utf8codepoint) != 0;
                        }

                        if charexists {
                            let text_slice = if text_pos + charlen <= text_bytes.len() {
                                &text_bytes[text_pos..text_pos + charlen]
                            } else {
                                &text_bytes[text_pos..]
                            };
                            tmpw = self.font_getexts(cur_font, text_slice);

                            if ew + self.ellipsis_width <= w {
                                ellipsis_x = x + ew as i32;
                                ellipsis_w = w.saturating_sub(ew);
                                ellipsis_len = utf8strlen;
                            }

                            if ew + tmpw > w {
                                overflow = true;
                                if !render {
                                    x += tmpw as i32;
                                } else {
                                    utf8strlen = ellipsis_len;
                                }
                            } else if cur_idx == usedfont_idx {
                                utf8strlen += charlen;
                                text_pos += charlen;
                                ew += tmpw;
                            } else {
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

            if utf8strlen > 0 {
                if render {
                    let f = get_font_at(self.fonts.as_ref().unwrap(), usedfont_idx).unwrap();
                    let ty = y + ((h - f.h) / 2) as i32 + f.ascent;

                    unsafe {
                        XftDrawStringUtf8(
                            d,
                            if invert { &bg_color } else { &fg_color } as *const XftColor,
                            f.xfont,
                            x as c_int,
                            ty as c_int,
                            utf8str.as_ptr(),
                            utf8strlen as c_int,
                        );
                    }
                }
                x += ew as i32;
                w = w.saturating_sub(ew);
            }

            if render && overflow && text != "..." {
                self.text_inner(
                    ellipsis_x,
                    y,
                    ellipsis_w,
                    h,
                    0,
                    "...",
                    invert,
                    detail_height,
                );
            }

            if text_pos >= text_bytes.len() || overflow {
                break;
            }

            if let Some(next_idx) = nextfont_idx {
                charexists = false;
                usedfont_idx = next_idx;
            } else {
                charexists = true;

                let mut found_nomatch = false;
                for i in 0..NOMATCHES_LEN {
                    if utf8codepoint == self.nomatches[i] {
                        found_nomatch = true;
                        break;
                    }
                }

                if !found_nomatch {
                    unsafe {
                        let fccharset = FcCharSetCreate();
                        FcCharSetAddChar(fccharset, utf8codepoint);

                        let fonts_ref = self.fonts.as_ref().unwrap();
                        if fonts_ref.pattern.is_null() {
                            die("the first font in the cache must be loaded from a font string.");
                        }

                        let fcpattern = FcPatternDuplicate(fonts_ref.pattern);
                        FcPatternAddCharSet(fcpattern, FC_CHARSET.as_ptr(), fccharset);
                        FcPatternAddBool(fcpattern, FC_SCALABLE.as_ptr(), FcTrue);

                        FcConfigSubstitute(ptr::null_mut(), fcpattern, FcMatchPattern);
                        FcDefaultSubstitute(fcpattern);

                        let mut result: XftResult = 0;
                        let match_pattern =
                            XftFontMatch(self.display, self.screen, fcpattern, &mut result);

                        FcCharSetDestroy(fccharset);
                        FcPatternDestroy(fcpattern);

                        if !match_pattern.is_null() {
                            if let Ok(Some(new_font)) = self.xfont_create(None, Some(match_pattern))
                            {
                                let f = &new_font;
                                if XftCharExists(self.display, f.xfont, utf8codepoint) != 0 {
                                    usedfont_idx = count_fonts(self.fonts.as_ref().unwrap());
                                    let mut last = self.fonts.as_mut().unwrap();
                                    while last.next.is_some() {
                                        last = last.next.as_mut().unwrap();
                                    }
                                    last.next = Some(new_font);
                                } else {
                                    let idx = NOMATCHES_IDX.fetch_add(1, Ordering::SeqCst);
                                    self.nomatches[idx as usize % NOMATCHES_LEN] = utf8codepoint;
                                    usedfont_idx = 0;
                                }
                            }
                        }
                    }
                } else {
                    usedfont_idx = 0;
                }
            }
        }

        if !d.is_null() {
            unsafe {
                XftDrawDestroy(d);
            }
        }

        x + if render { w as i32 } else { 0 }
    }

    fn text_inner(
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
        // When drawing ellipsis, don't allow recursive ellipsis drawing
        // to avoid infinite recursion. Pass h=0 to skip ellipsis calculation.
        if text == "..." {
            self.text(x, y, w, h, lpad, text, invert, -1)
        } else {
            self.text(x, y, w, h, lpad, text, invert, detail_height)
        }
    }

    pub fn arrow(&self, x: i16, y: i16, w: u16, h: u16, direction: bool, slash: bool) {
        let scheme = match &self.scheme {
            Some(s) => s,
            None => return,
        };

        let x = if direction { x } else { x + w as i16 };
        let w = if direction { w as i16 } else { -(w as i16) };
        let hh = if slash {
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

            let mut points = [
                Point { x, y },
                Point {
                    x: (x as i32 + w as i32) as i16,
                    y: y + hh,
                },
                Point { x, y: y + h as i16 },
            ];

            XFillPolygon(
                self.display,
                self.drawable,
                self.gc,
                points.as_mut_ptr(),
                3,
                1,
                0,
            );
        }
    }

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

    pub fn fontset_getwidth(&mut self, text: &str) -> u32 {
        if self.fonts.is_none() || text.is_empty() {
            return 0;
        }
        self.text(0, 0, 0, 0, 0, text, false, 0) as u32
    }

    pub fn fontset_getwidth_clamp(&mut self, text: &str, n: u32) -> u32 {
        if self.fonts.is_none() || text.is_empty() || n == 0 {
            return 0;
        }
        let tmp = self.text(0, 0, 0, 0, 0, text, false, 0) as u32;
        min(n, tmp)
    }

    pub fn font_getexts(&self, font: &Fnt, text: &[u8]) -> u32 {
        if text.is_empty() {
            return 0;
        }

        let mut ext = XGlyphInfo {
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            xOff: 0,
            yOff: 0,
        };

        unsafe {
            XftTextExtentsUtf8(
                self.display,
                font.xfont,
                text.as_ptr(),
                text.len() as c_int,
                &mut ext,
            );
        }

        ext.xOff as u32
    }

    pub fn font_getexts_h(&self, font: &Fnt, text: &[u8]) -> (u32, u32) {
        if text.is_empty() {
            return (0, font.h);
        }

        let mut ext = XGlyphInfo {
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            xOff: 0,
            yOff: 0,
        };

        unsafe {
            XftTextExtentsUtf8(
                self.display,
                font.xfont,
                text.as_ptr(),
                text.len() as c_int,
                &mut ext,
            );
        }

        (ext.xOff as u32, font.h)
    }

    pub fn drawable(&self) -> Drawable {
        self.drawable
    }

    pub fn gc(&self) -> XlibGc {
        self.gc
    }
}

impl Drop for Drw {
    fn drop(&mut self) {
        unsafe {
            if self.owns_resources && !self.display.is_null() {
                XFreePixmap(self.display, self.drawable);
                XFreeGC(self.display, self.gc);
                XCloseDisplay(self.display);
            }
        }
    }
}

pub fn utf8decode(bytes: &[u8]) -> (usize, u32) {
    if bytes.is_empty() {
        return (0, UTF_INVALID);
    }

    let len = utf8decode_byte(bytes[0]);

    if !between(len, 1, UTF_SIZ) {
        return (1, UTF_INVALID);
    }

    if bytes.len() < len {
        return (0, UTF_INVALID);
    }

    let mut udecoded = (bytes[0] as u32) & !(UTFMASK[len] as u32);

    for i in 1..len {
        let type_ = utf8decode_byte(bytes[i]);
        if type_ != 0 {
            return (i, UTF_INVALID);
        }
        udecoded = (udecoded << 6) | ((bytes[i] as u32) & !(UTFMASK[0] as u32));
    }

    let u = utf8validate(udecoded, len);
    (len, u)
}

fn utf8decode_byte(c: u8) -> usize {
    for i in 0..=UTF_SIZ {
        if (c & UTFMASK[i]) == UTFBYTE[i] {
            return i;
        }
    }
    0
}

fn utf8validate(u: u32, i: usize) -> u32 {
    if !between(u, UTFMIN[i], UTFMAX[i]) || between(u, 0xD800, 0xDFFF) {
        return UTF_INVALID;
    }
    u
}

pub fn drw_fontset_free(font: Option<Box<Fnt>>) {
    if let Some(mut f) = font {
        let next = f.next.take();
        drop(f);
        drw_fontset_free(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8decode_ascii() {
        let (len, cp) = utf8decode(b"a");
        assert_eq!(len, 1);
        assert_eq!(cp, 'a' as u32);
    }

    #[test]
    fn test_utf8decode_multibyte() {
        let (len, cp) = utf8decode("é".as_bytes());
        assert_eq!(len, 2);
        assert_eq!(cp, 0xE9);
    }

    #[test]
    fn test_utf8decode_empty() {
        let (len, cp) = utf8decode(b"");
        assert_eq!(len, 0);
        assert_eq!(cp, UTF_INVALID);
    }
}
