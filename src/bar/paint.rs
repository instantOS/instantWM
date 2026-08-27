use crate::types::{ColorSchemeRgba, Rect, Rgba, Size};

/// How text that is wider than its cell is represented.
///
/// This is presentation policy, not a rasterizer detail: every backend must
/// produce the same fitted string before handing it to Xft, cosmic-text, or a
/// future text engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOverflow {
    Clip,
    Ellipsis,
}

#[derive(Clone, Debug)]
pub struct BarScheme {
    pub foreground: Rgba,
    pub background: Rgba,
    pub detail: Rgba,
}

impl BarScheme {
    /// Rectangle fill color parity with X11 drw semantics:
    /// invert=true => background, invert=false => foreground.
    pub fn rect_color(&self, invert: bool) -> Rgba {
        if invert {
            self.background
        } else {
            self.foreground
        }
    }

    /// Text colors parity with X11 drw semantics.
    /// Returns (background, foreground).
    pub fn text_colors(&self, invert: bool) -> (Rgba, Rgba) {
        let bg = if invert {
            self.foreground
        } else {
            self.background
        };
        let fg = if invert {
            self.background
        } else {
            self.foreground
        };
        (bg, fg)
    }
}

impl From<&ColorSchemeRgba> for BarScheme {
    fn from(colors: &ColorSchemeRgba) -> Self {
        Self {
            foreground: colors.fg,
            background: colors.bg,
            detail: colors.detail,
        }
    }
}

pub trait BarPainter {
    /// Measure the horizontal advance of `text` using the fonts active for the
    /// monitor currently being painted.
    fn text_width(&mut self, text: &str) -> i32;
    fn set_scheme(&mut self, scheme: BarScheme);

    /// Fill `bounds` with the scheme foreground (`invert=false`) or background
    /// (`invert=true`).
    fn rect(&mut self, bounds: Rect, invert: bool);

    /// Paint one complete bar cell and return `bounds.right()`.
    ///
    /// The background and optional detail strip are painted even when `text`
    /// is empty. `overflow` is applied after subtracting `lpad`. Implementors
    /// must not return a glyph advance: callers chain cells using the returned
    /// right edge.
    fn text(
        &mut self,
        bounds: Rect,
        lpad: i32,
        text: &str,
        invert: bool,
        detail_height: i32,
        overflow: TextOverflow,
    ) -> i32;
    /// Blit non-premultiplied RGBA8 pixels (row-major, 4 bytes per pixel)
    /// scaled to exactly fill `destination`. Used for compositor-rendered
    /// tray icons; alpha is blended over existing content.
    fn blit_rgba(&mut self, destination: Rect, source_size: Size, src_rgba: &[u8]);
}
