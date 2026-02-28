use crate::bar::paint::{BarPainter, BarScheme};
use crate::drw::Drw;
use crate::types::ColorScheme;

pub struct X11BarPainter {
    drw: Drw,
    scheme: Option<BarScheme>,
}

impl X11BarPainter {
    pub fn new(drw: Drw) -> Self {
        Self { drw, scheme: None }
    }

    pub fn map(&self, win: crate::types::WindowId, x: i16, y: i16, w: u16, h: u16) {
        self.drw.map(win.into(), x, y, w, h);
    }
}

impl BarPainter for X11BarPainter {
    fn text_width(&mut self, text: &str) -> i32 {
        self.drw.fontset_getwidth(text) as i32
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        let cs = ColorScheme {
            fg: self.drw.clr_create_rgba(scheme.fg),
            bg: self.drw.clr_create_rgba(scheme.bg),
            detail: self.drw.clr_create_rgba(scheme.detail),
        };
        self.drw.set_scheme(cs);
        self.scheme = Some(scheme);
    }

    fn scheme(&self) -> Option<&BarScheme> {
        self.scheme.as_ref()
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, filled: bool, invert: bool) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.drw.rect(x, y, w as u32, h as u32, filled, invert);
    }

    fn text(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        lpad: i32,
        text: &str,
        invert: bool,
        detail_height: i32,
    ) -> i32 {
        if w <= 0 || h <= 0 {
            return x;
        }
        self.drw.text(
            x,
            y,
            w as u32,
            h as u32,
            lpad.max(0) as u32,
            text,
            invert,
            detail_height,
        )
    }
}
