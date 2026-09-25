use crate::types::{ColorScheme, Rect, Rgba, Size};

/// Which color of the active [`ColorScheme`] a shape or text cell uses.
///
/// Replaces dwm's `invert` boolean, which meant the same thing in both
/// positions but read as a bare `true`/`false` at every call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemeColor {
    /// The scheme's foreground: the accent, or the inverted polarity.
    Foreground,
    /// The scheme's background: the resting fill.
    Background,
}

/// The color a shape is filled with.
pub fn fill_color(scheme: &ColorScheme, color: SchemeColor) -> Rgba {
    match color {
        SchemeColor::Foreground => scheme.foreground,
        SchemeColor::Background => scheme.background,
    }
}

/// The `(background, foreground)` pair a text cell is painted with.
///
/// `SchemeColor::Background` inverts the scheme, matching X11 drw semantics.
pub fn text_colors(scheme: &ColorScheme, color: SchemeColor) -> (Rgba, Rgba) {
    let (background, foreground) = (scheme.background, scheme.foreground);
    match color {
        SchemeColor::Background => (foreground, background),
        SchemeColor::Foreground => (background, foreground),
    }
}

/// Height of the bottom accent strip marking a hovered status block or menu entry.
pub const HOVER_INDICATOR_HEIGHT: i32 = 3;

/// Bottom accent strip in the hover colour, shared by status and systray.
pub fn draw_hover_accent(painter: &mut dyn BarPainter, bounds: Rect, color: Rgba) {
    let height = HOVER_INDICATOR_HEIGHT.min(bounds.h).max(0);
    // A uniform scheme keeps the accent on the cached allocation path: the X11
    // backend resolves `SchemeColor` against whatever scheme is active.
    painter.set_scheme(ColorScheme::new(color, color, color));
    painter.rect(
        Rect::new(bounds.x, bounds.bottom() - height, bounds.w, height),
        SchemeColor::Foreground,
    );
}

pub trait BarPainter {
    /// Measure the horizontal advance of `text` using the fonts active for the
    /// monitor currently being painted.
    fn text_width(&mut self, text: &str) -> i32;

    /// Make `scheme` the active palette for subsequent [`BarPainter::rect`]
    /// and [`BarPainter::text`] calls.
    ///
    /// Backends resolve scheme colors against this value rather than receiving
    /// resolved colors per call, so repeated fills cost one lookup instead of
    /// one native color allocation each.
    fn set_scheme(&mut self, scheme: ColorScheme);

    /// Fill `bounds` with the active scheme's `color`.
    fn rect(&mut self, bounds: Rect, color: SchemeColor);

    /// Paint one complete bar cell and return `bounds.right()`.
    ///
    /// The background and optional detail strip are painted even when `text`
    /// is empty. Text wider than the cell after subtracting `lpad` is
    /// ellipsized with [`crate::bar::text::fit_to_width`]. Implementors
    /// must not return a glyph advance: callers chain cells using the returned
    /// right edge.
    fn text(
        &mut self,
        bounds: Rect,
        lpad: i32,
        text: &str,
        color: SchemeColor,
        detail_height: i32,
    ) -> i32;
    /// Blit non-premultiplied RGBA8 pixels (row-major, 4 bytes per pixel)
    /// scaled to exactly fill `destination`. Used for compositor-rendered
    /// tray icons; alpha is blended over existing content.
    fn blit_rgba(&mut self, destination: Rect, source_size: Size, src_rgba: &[u8]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheme() -> ColorScheme {
        ColorScheme::new(
            Rgba::new(1.0, 0.0, 0.0, 1.0),
            Rgba::new(0.0, 0.0, 1.0, 1.0),
            Rgba::ZERO,
        )
    }

    /// `SchemeColor` replaces drw's `invert` bool, so it must resolve to
    /// exactly the colors the old `false`/`true` arguments did: shapes take
    /// the named color, and `Background` inverts the text pair.
    #[test]
    fn scheme_color_preserves_drw_invert_parity() {
        let s = scheme();

        assert_eq!(fill_color(&s, SchemeColor::Foreground), s.foreground);
        assert_eq!(fill_color(&s, SchemeColor::Background), s.background);

        assert_eq!(text_colors(&s, SchemeColor::Foreground), (s.background, s.foreground));
        assert_eq!(text_colors(&s, SchemeColor::Background), (s.foreground, s.background));
    }
}
