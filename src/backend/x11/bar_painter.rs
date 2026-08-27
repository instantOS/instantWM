use crate::backend::x11::draw::DrawContext;
use crate::bar::paint::{BarPainter, BarScheme, TextOverflow};
use crate::types::{Rect, Size};

pub struct X11BarPainter<'a> {
    drw: &'a mut DrawContext,
}

impl<'a> X11BarPainter<'a> {
    pub fn new(drw: &'a mut DrawContext) -> Self {
        Self { drw }
    }

    pub fn map(&self, window: crate::types::WindowId, bounds: Rect) {
        self.drw.map(window.into(), bounds);
    }
}

impl BarPainter for X11BarPainter<'_> {
    fn text_width(&mut self, text: &str) -> i32 {
        self.drw.fontset_getwidth(text) as i32
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        self.drw.set_bar_scheme(&scheme);
    }

    fn rect(&mut self, bounds: Rect, invert: bool) {
        if bounds.w <= 0 || bounds.h <= 0 {
            return;
        }
        self.drw.rect(bounds, true, invert);
    }

    fn text(
        &mut self,
        bounds: Rect,
        lpad: i32,
        text: &str,
        invert: bool,
        detail_height: i32,
        overflow: TextOverflow,
    ) -> i32 {
        if bounds.w <= 0 || bounds.h <= 0 {
            return bounds.x;
        }
        let lpad = lpad.max(0).min(bounds.w);
        let fitted = crate::bar::text::fit_to_width(text, bounds.w - lpad, overflow, |candidate| {
            self.drw.fontset_getwidth(candidate) as i32
        });
        self.drw
            .text(bounds, lpad as u32, fitted.as_ref(), invert, detail_height);
        bounds.right()
    }

    fn blit_rgba(&mut self, destination: Rect, source_size: Size, src_rgba: &[u8]) {
        if destination.w <= 0 || destination.h <= 0 {
            return;
        }
        self.drw.blit_rgba(destination, source_size, src_rgba);
    }
}
