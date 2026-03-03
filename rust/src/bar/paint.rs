use crate::bar::color::Rgba;

#[derive(Clone, Debug)]
pub struct BarScheme {
    pub fg: Rgba,
    pub bg: Rgba,
    pub detail: Rgba,
}

impl BarScheme {
    pub fn swap_fg_bg(&self) -> Self {
        Self {
            fg: self.bg,
            bg: self.fg,
            detail: self.detail,
        }
    }

    /// Rectangle fill color parity with X11 drw semantics:
    /// invert=true => background, invert=false => foreground.
    pub fn rect_color(&self, invert: bool) -> Rgba {
        if invert {
            self.bg
        } else {
            self.fg
        }
    }

    /// Text colors parity with X11 drw semantics.
    /// Returns (background, foreground).
    pub fn text_colors(&self, invert: bool) -> (Rgba, Rgba) {
        let bg = if invert { self.fg } else { self.bg };
        let fg = if invert { self.bg } else { self.fg };
        (bg, fg)
    }
}

pub trait BarPainter {
    fn text_width(&mut self, text: &str) -> i32;
    fn set_scheme(&mut self, scheme: BarScheme);
    fn scheme(&self) -> Option<&BarScheme>;
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, filled: bool, invert: bool);
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
    ) -> i32;

    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.rect(x, y, w, h, true, false);
    }

    fn clear_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.rect(x, y, w, h, true, true);
    }
}
