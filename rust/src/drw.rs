use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_ulong};
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use x11rb::protocol::xproto::{Arc, CreateGCAux, Drawable, Gcontext, Point, Rectangle, Window};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt;
use x11rb::x11_utils::X11Error;

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

#[repr(C)]
pub struct XRenderColor {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub alpha: u16,
}

#[repr(C)]
pub struct XftFont {
    _private: [u8; 0],
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

pub const FC_CHARSET: &[u8] = b"charset\0";
pub const FC_SCALABLE: &[u8] = b"scalable\0";

pub const FcMatchPattern: c_int = 1;
pub const FcTrue: FcBool = 1;

extern "C" {
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
    pub fn XftFontMatch(
        display: *mut libc::c_void,
        screen: c_int,
        pattern: *mut FcPattern,
        result: *mut XftResult,
    ) -> *mut FcPattern;

    pub fn FcCharSetCreate() -> *mut FcCharSet;
    pub fn FcCharSetAddChar(fcs: *mut FcCharSet, ucs4: u32) -> FcBool;
    pub fn FcCharSetDestroy(fcs: *mut FcCharSet);
}

#[derive(Debug, Clone)]
pub struct Clr {
    pub color: XftColor,
}

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

impl Cur {
    pub fn new(cursor: u32) -> Self {
        Self { cursor }
    }
}

#[derive(Debug)]
pub struct Fnt {
    pub display: *mut libc::c_void,
    pub h: u32,
    pub xfont: *mut XftFont,
    pub pattern: *mut FcPattern,
    pub next: Option<Box<Fnt>>,
    ascent: i32,
    descent: i32,
}

impl Fnt {
    pub fn height(&self) -> u32 {
        self.h
    }
}

impl Drop for Fnt {
    fn drop(&mut self) {
        unsafe {
            if !self.pattern.is_null() {
                FcPatternDestroy(self.pattern);
            }
            if !self.xfont.is_null() && !self.display.is_null() {
                XftFontClose(self.display, self.xfont);
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
    gc: Gcontext,
    scheme: Option<Vec<Clr>>,
    fonts: Option<Box<Fnt>>,
    depth: u8,
    visual: *mut libc::c_void,
    colormap: u32,
    nomatches: [u32; NOMATCHES_LEN],
    ellipsis_width: u32,
}

impl Drw {
    pub fn new(
        conn: &RustConnection,
        screen: usize,
        root: Window,
        w: u32,
        h: u32,
    ) -> Result<Self, X11Error> {
        let screen_info = conn.setup().roots.get(screen).ok_or_else(|| {
            X11Error::from(x11rb::x11_utils::X11Error::make_error(
                0,
                0,
                x11rb::x11_utils::Extension::Unknown,
            ))
        })?;

        let depth = screen_info.root_depth;
        let visual = screen_info.root_visual;
        let colormap = screen_info.default_colormap;

        let drawable = conn.generate_id()?;
        conn.create_pixmap(depth, drawable, root, w, h)?;

        let gc = conn.generate_id()?;
        conn.create_gc(gc, root, &CreateGCAux::new())?;

        let display_ptr = unsafe { x11rb::ffi::get_xcb_display(conn.get_raw_xcb_connection()) };

        Ok(Self {
            w,
            h,
            display: display_ptr,
            screen: screen as i32,
            root,
            drawable,
            gc,
            scheme: None,
            fonts: None,
            depth,
            visual: visual as *mut libc::c_void,
            colormap,
            nomatches: [0; NOMATCHES_LEN],
            ellipsis_width: 0,
        })
    }

    pub fn resize(&mut self, conn: &RustConnection, w: u32, h: u32) -> Result<(), X11Error> {
        self.w = w;
        self.h = h;

        conn.free_pixmap(self.drawable)?;

        let screen_info = conn.setup().roots.get(self.screen as usize).unwrap();
        let depth = screen_info.root_depth;

        self.drawable = conn.generate_id()?;
        conn.create_pixmap(depth, self.drawable, self.root, w, h)?;

        Ok(())
    }

    pub fn set_scheme(&mut self, scheme: Vec<Clr>) {
        self.scheme = Some(scheme);
    }

    pub fn set_fontset(&mut self, font: Option<Box<Fnt>>) {
        self.fonts = font;
    }

    pub fn fontset_create(&mut self, fonts: &[&str]) -> Result<Option<Box<Fnt>>, String> {
        let mut ret: Option<Box<Fnt>> = None;

        for fontname in fonts.iter().rev() {
            if let Some(font) = self.xfont_create(Some(fontname), None)? {
                font.next = ret;
                ret = Some(font);
            }
        }

        self.fonts = ret.clone();
        Ok(ret)
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
                pattern = FcNameParse(name.as_ptr());
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
            descent,
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

    pub fn cur_create(&self, conn: &RustConnection, shape: u16) -> Result<Cur, X11Error> {
        let cursor = conn.generate_id()?;
        conn.open_font(cursor, shape)?;
        Ok(Cur::new(cursor))
    }

    pub fn cur_free(&self, conn: &RustConnection, cursor: &Cur) -> Result<(), X11Error> {
        conn.free_cursor(cursor.cursor)
    }

    pub fn rect(
        &self,
        conn: &RustConnection,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        filled: bool,
        invert: bool,
    ) -> Result<(), X11Error> {
        let scheme = match &self.scheme {
            Some(s) => s,
            None => return Ok(()),
        };

        let fg_pixel = if invert {
            scheme[COL_BG].pixel()
        } else {
            scheme[COL_FG].pixel()
        };

        conn.change_gc(
            self.gc,
            &x11rb::protocol::xproto::ChangeGCAux::new().foreground(fg_pixel),
        )?;

        if filled {
            let rect = Rectangle {
                x,
                y,
                width: w,
                height: h,
            };
            conn.poly_fill_rectangle(self.drawable, self.gc, &[rect])?;
        } else {
            let rect = Rectangle {
                x,
                y,
                width: w.saturating_sub(1),
                height: h.saturating_sub(1),
            };
            conn.poly_rectangle(self.drawable, self.gc, &[rect])?;
        }

        Ok(())
    }

    pub fn circ(
        &self,
        conn: &RustConnection,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        filled: bool,
        invert: bool,
    ) -> Result<(), X11Error> {
        let scheme = match &self.scheme {
            Some(s) => s,
            None => return Ok(()),
        };

        let fg_pixel = if invert {
            scheme[COL_BG].pixel()
        } else {
            scheme[COL_FG].pixel()
        };

        conn.change_gc(
            self.gc,
            &x11rb::protocol::xproto::ChangeGCAux::new().foreground(fg_pixel),
        )?;

        let arc = Arc {
            x,
            y,
            width: if filled { w } else { w.saturating_sub(1) },
            height: if filled { h } else { h.saturating_sub(1) },
            angle1: 0,
            angle2: 360 * 64,
        };

        if filled {
            conn.poly_fill_arc(self.drawable, self.gc, &[arc])?;
        } else {
            conn.poly_arc(self.drawable, self.gc, &[arc])?;
        }

        Ok(())
    }

    pub fn text(
        &mut self,
        conn: &RustConnection,
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
        let mut usedfont: *mut Fnt;
        let mut curfont: *mut Fnt;
        let mut nextfont: *mut Fnt;
        let mut utf8strlen: usize;
        let mut utf8charlen: usize;
        let render = x != 0 || y != 0 || w != 0 || h != 0;
        let mut utf8codepoint: u32 = 0;
        let mut utf8str: &[u8];
        let mut charexists = false;
        let mut overflow = false;

        let fonts = match &self.fonts {
            Some(f) => f,
            None => return 0,
        };

        let scheme = match &self.scheme {
            Some(s) => s,
            None => return 0,
        };

        if text.is_empty() {
            return 0;
        }

        if !render {
            w = if invert != 0 {
                invert as u32
            } else {
                !invert as u32
            };
        } else {
            let fg_pixel = if invert {
                scheme[COL_FG].pixel()
            } else {
                scheme[COL_BG].pixel()
            };

            let _ = conn.change_gc(
                self.gc,
                &x11rb::protocol::xproto::ChangeGCAux::new().foreground(fg_pixel),
            );

            if detail_height > 0 {
                let _ = conn.poly_fill_rectangle(
                    self.drawable,
                    self.gc,
                    &[Rectangle {
                        x: x as i16,
                        y: y as i16,
                        width: w as u16,
                        height: (h as u16).saturating_sub(detail_height as u16),
                    }],
                );

                let detail_pixel = scheme[COL_DETAIL].pixel();
                let _ = conn.change_gc(
                    self.gc,
                    &x11rb::protocol::xproto::ChangeGCAux::new().foreground(detail_pixel),
                );

                let _ = conn.poly_fill_rectangle(
                    self.drawable,
                    self.gc,
                    &[Rectangle {
                        x: x as i16,
                        y: (y + h as i32 - detail_height) as i16,
                        width: w as u16,
                        height: detail_height as u16,
                    }],
                );
            } else {
                let _ = conn.poly_fill_rectangle(
                    self.drawable,
                    self.gc,
                    &[Rectangle {
                        x: x as i16,
                        y: y as i16,
                        width: w as u16,
                        height: h as u16,
                    }],
                );
            }

            unsafe {
                d = XftDrawCreate(self.display, self.drawable, self.visual, self.colormap);
            }

            x += lpad as i32;
            w -= lpad;
        }

        usedfont = fonts as *const Fnt as *mut Fnt;

        if self.ellipsis_width == 0 && render {
            self.ellipsis_width = self.fontset_getwidth("...");
        }

        let text_bytes = text.as_bytes();
        let mut text_pos: usize = 0;

        loop {
            ew = 0;
            ellipsis_len = 0;
            utf8strlen = 0;
            utf8str = &text_bytes[text_pos..];
            nextfont = ptr::null_mut();

            while text_pos < text_bytes.len() {
                let (charlen, codepoint) = utf8decode(&text_bytes[text_pos..]);
                utf8charlen = charlen;
                utf8codepoint = codepoint;

                while !charexists {
                    let mut cur_font = fonts as *const Fnt as *mut Fnt;
                    while !cur_font.is_null() {
                        unsafe {
                            charexists =
                                XftCharExists(self.display, (*cur_font).xfont, utf8codepoint) != 0;
                        }

                        if charexists {
                            let f = unsafe { &*cur_font };
                            tmpw = self.font_getexts(f, &text_bytes[text_pos..text_pos + charlen]);

                            if ew + self.ellipsis_width <= w {
                                ellipsis_x = x + ew as i32;
                                ellipsis_w = w - ew;
                                ellipsis_len = utf8strlen;
                            }

                            if ew + tmpw > w {
                                overflow = true;
                                if !render {
                                    x += tmpw as i32;
                                } else {
                                    utf8strlen = ellipsis_len;
                                }
                            } else if cur_font == usedfont {
                                utf8strlen += charlen;
                                text_pos += charlen;
                                ew += tmpw;
                            } else {
                                nextfont = cur_font;
                            }
                            break;
                        }

                        unsafe {
                            cur_font = if (*cur_font).next.is_some() {
                                (&*(cur_font).next.as_ref().unwrap()) as *const Fnt as *mut Fnt
                            } else {
                                ptr::null_mut()
                            };
                        }
                    }

                    if !charexists {
                        let (charlen2, _) = utf8decode(b"a");
                        utf8charlen = charlen2;
                    }
                }

                if overflow || !charexists || !nextfont.is_null() {
                    break;
                }

                charexists = false;
            }

            if utf8strlen > 0 {
                if render {
                    let f = unsafe { &*usedfont };
                    let ty = y + ((h - f.h) / 2) as i32 + f.ascent;

                    unsafe {
                        XftDrawStringUtf8(
                            d,
                            if invert {
                                &scheme[COL_BG].color
                            } else {
                                &scheme[COL_FG].color
                            } as *const XftColor,
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

            if render && overflow {
                let _ = self.text(
                    conn,
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

            if !nextfont.is_null() {
                charexists = false;
                usedfont = nextfont;
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
                                let f = new_font.as_ref();
                                unsafe {
                                    if XftCharExists(self.display, f.xfont, utf8codepoint) != 0 {
                                        let mut last = self.fonts.as_mut().unwrap();
                                        while last.next.is_some() {
                                            last = last.next.as_mut().unwrap();
                                        }
                                        usedfont = new_font.as_ref() as *const Fnt as *mut Fnt;
                                        last.next = Some(new_font);
                                    } else {
                                        let idx = NOMATCHES_IDX.fetch_add(1, Ordering::SeqCst);
                                        self.nomatches[idx as usize % NOMATCHES_LEN] =
                                            utf8codepoint;
                                        usedfont =
                                            self.fonts.as_ref().unwrap() as *const Fnt as *mut Fnt;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    usedfont = self.fonts.as_ref().unwrap() as *const Fnt as *mut Fnt;
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

    pub fn arrow(
        &self,
        conn: &RustConnection,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        direction: bool,
        slash: bool,
    ) -> Result<(), X11Error> {
        let scheme = match &self.scheme {
            Some(s) => s,
            None => return Ok(()),
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

        conn.change_gc(
            self.gc,
            &x11rb::protocol::xproto::ChangeGCAux::new().foreground(scheme[COL_BG].pixel()),
        )?;

        let points = [
            Point { x, y },
            Point {
                x: (x as i32 + w as i32) as i16,
                y: y + hh,
            },
            Point { x, y: y + h as i16 },
        ];

        conn.poly_fill_polygon(
            self.drawable,
            self.gc,
            x11rb::protocol::xproto::Shape::NONCONVEX,
            x11rb::protocol::xproto::CoordMode::ORIGIN,
            &points,
        )?;

        Ok(())
    }

    pub fn map(
        &self,
        conn: &RustConnection,
        win: Window,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
    ) -> Result<(), X11Error> {
        conn.copy_area(self.drawable, win, self.gc, x, y, w, h, x, y)?;
        conn.flush()?;
        Ok(())
    }

    pub fn fontset_getwidth(&mut self, text: &str) -> u32 {
        if self.fonts.is_none() || text.is_empty() {
            return 0;
        }
        self.text_inner_width(text, 0)
    }

    pub fn fontset_getwidth_clamp(&mut self, text: &str, n: u32) -> u32 {
        if self.fonts.is_none() || text.is_empty() || n == 0 {
            return 0;
        }
        let tmp = self.text_inner_width(text, n);
        min(n, tmp)
    }

    fn text_inner_width(&mut self, text: &str, clamp: u32) -> u32 {
        let result = self.text(
            unsafe {
                &x11rb::rust_connection::RustConnection::from_raw_xcb_connection(
                    x11rb::ffi::get_raw_xcb_display(self.display) as *mut _,
                    false,
                )
                .ok()
                .unwrap()
            },
            0,
            0,
            0,
            0,
            0,
            text,
            clamp as i32,
            0,
        );
        result as u32
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

    pub fn gc(&self) -> Gcontext {
        self.gc
    }
}

impl Drop for Drw {
    fn drop(&mut self) {
        unsafe {
            let _ = x11rb::rust_connection::RustConnection::from_raw_xcb_connection(
                self.display as *mut _,
                true,
            );
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

pub fn drw_fontset_free(mut font: Option<Box<Fnt>>) {
    if let Some(f) = font.take() {
        drw_fontset_free(f.next);
        drop(f);
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
