use crate::backend::x11::draw::DrawContext;
use crate::bar::paint::{BarPainter, BarScheme};
use crate::types::ColorScheme;
use crate::types::Rect;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SchemeKey {
    foreground: [u32; 4],
    background: [u32; 4],
    detail: [u32; 4],
}

impl SchemeKey {
    fn from_scheme(s: &BarScheme) -> Self {
        Self {
            foreground: s.foreground.into_array().map(f32::to_bits),
            background: s.background.into_array().map(f32::to_bits),
            detail: s.detail.into_array().map(f32::to_bits),
        }
    }
}

pub struct X11BarPainter {
    drw: DrawContext,
    scheme: Option<BarScheme>,
    scheme_cache: HashMap<SchemeKey, ColorScheme>,
}

impl X11BarPainter {
    pub fn new(drw: DrawContext) -> Self {
        Self {
            drw,
            scheme: None,
            scheme_cache: HashMap::new(),
        }
    }

    pub fn map(&self, window: crate::types::WindowId, bounds: Rect) {
        self.drw.map(window.into(), bounds);
    }
}

impl BarPainter for X11BarPainter {
    fn text_width(&mut self, text: &str) -> i32 {
        self.drw.fontset_getwidth(text) as i32
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        let key = SchemeKey::from_scheme(&scheme);
        let cs = if let Some(existing) = self.scheme_cache.get(&key) {
            existing.clone()
        } else {
            let built = ColorScheme {
                fg: self.drw.clr_create_rgba(scheme.foreground),
                bg: self.drw.clr_create_rgba(scheme.background),
                detail: self.drw.clr_create_rgba(scheme.detail),
            };
            self.scheme_cache.insert(key, built.clone());
            built
        };
        self.drw.set_scheme(cs);
        self.scheme = Some(scheme);
    }

    fn rect(&mut self, bounds: Rect, filled: bool, invert: bool) {
        if bounds.w <= 0 || bounds.h <= 0 {
            return;
        }
        self.drw.rect(bounds, filled, invert);
    }

    fn text(
        &mut self,
        bounds: Rect,
        lpad: i32,
        text: &str,
        invert: bool,
        detail_height: i32,
    ) -> i32 {
        if bounds.w <= 0 || bounds.h <= 0 {
            return bounds.x;
        }
        self.drw
            .text(bounds, lpad.max(0) as u32, text, invert, detail_height)
    }
}
