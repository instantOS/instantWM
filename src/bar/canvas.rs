//! A CPU-side ARGB8888 pixel surface for bar rasterization.
//!
//! The bar is painted by software: shapes, glyphs and tray icons are all
//! composited into one byte buffer per monitor before it is handed to the
//! compositor as a render buffer. [`Canvas`] owns that buffer together with its
//! dimensions, so "a point inside the canvas addresses a whole pixel" holds by
//! construction instead of being re-checked on every write.

use crate::bar::image::Rgba8Image;
use crate::types::{Point, Rect, Rgba8, Size};

/// Bytes per pixel in an ARGB8888 surface.
const BYTES_PER_PIXEL: usize = 4;

/// A row-major ARGB8888 pixel buffer that always matches its declared size.
///
/// The buffer is zeroed on creation and on every [`Canvas::resize`], so a
/// freshly sized canvas is fully transparent rather than undefined.
#[derive(Clone, Default)]
pub struct Canvas {
    pixels: Vec<u8>,
    size: Size,
}

impl Canvas {
    /// A fully transparent canvas of `size`. Non-positive sizes produce an
    /// empty canvas, matching the degenerate case the bar's `begin` handles.
    pub fn new(size: Size) -> Self {
        let mut canvas = Self {
            pixels: Vec::new(),
            size: Size::default(),
        };
        canvas.resize(size);
        canvas
    }

    /// Reuse the allocation, clearing it to transparent and adopting `size`.
    pub fn resize(&mut self, size: Size) {
        // The invariant every per-pixel index relies on: the buffer is exactly
        // `w * h` pixels, or empty. A size too large to allocate fails cleanly
        // here rather than handing out a short buffer.
        let len = (size.w as usize)
            .checked_mul(size.h as usize)
            .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
            .filter(|len| *len <= isize::MAX as usize);
        self.pixels.clear();
        match len {
            Some(len) if size.is_positive() => {
                self.pixels.resize(len, 0);
                self.size = size;
            }
            _ => self.size = Size::default(),
        }
        debug_assert_eq!(self.pixels.len(), self.byte_len());
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The canvas area in canvas coordinates, i.e. the origin and size.
    pub fn bounds(&self) -> Rect {
        Rect::from_position_and_size(Point::new(0, 0), self.size)
    }

    /// The pixel buffer, for handing to a compositor render buffer.
    pub fn as_slice(&self) -> &[u8] {
        &self.pixels
    }

    /// Blend `color` over the pixel at `point`, ignoring points outside the
    /// canvas.
    pub fn fill_pixel(&mut self, point: Point, color: Rgba8) {
        if color.is_transparent() {
            return;
        }
        if let Some(pixel) = self.pixel_mut(point) {
            blend_pixel(pixel, color);
        }
    }

    /// Blend `color` over `rect`, clipped to the canvas.
    pub fn fill_rect(&mut self, rect: Rect, color: Rgba8) {
        if color.is_transparent() {
            return;
        }
        let Some(clipped) = self.bounds().intersection(&rect) else {
            return;
        };
        let opaque = color.components()[3] == u8::MAX;
        let columns = clipped.x as usize..clipped.right() as usize;
        let rows = clipped.y as usize..clipped.bottom() as usize;
        let stride = self.size.w as usize;
        // `clipped` is inside the canvas, so both ranges are in bounds.
        let Some(pixels) = self.pixels_mut() else {
            return;
        };
        // The buffer holds exactly `w * h` pixels, so the row count the
        // intersection admitted is always available.
        for row in pixels
            .chunks_exact_mut(stride)
            .skip(rows.start)
            .take(rows.len())
        {
            let Some(span) = row.get_mut(columns.clone()) else {
                continue;
            };
            if opaque {
                span.fill(color.little_endian_argb());
            } else {
                for pixel in span {
                    blend_pixel(pixel, color);
                }
            }
        }
    }

    /// Scale the non-premultiplied RGBA8 `source` to exactly fill `dst` and
    /// blend it over the canvas. A source too short for `source_size` is
    /// rejected without painting.
    pub fn blit_rgba(&mut self, dst: Rect, source: &[u8], source_size: Size) {
        let Some(source) = Rgba8Image::new(source, source_size) else {
            return;
        };
        let Some(clipped) = self.bounds().intersection(&dst) else {
            return;
        };
        // Walk row by row: the vertical scale factor depends only on `y`, so
        // resolving the source row once per row instead of once per pixel keeps
        // the inner loop down to a multiply and a bounds check.
        let columns = clipped.x as usize..clipped.right() as usize;
        let rows = clipped.y as usize..clipped.bottom() as usize;
        let stride = self.size.w as usize;
        let Some(pixels) = self.pixels_mut() else {
            return;
        };
        for (offset, row) in pixels
            .chunks_exact_mut(stride)
            .skip(rows.start)
            .take(rows.len())
            .enumerate()
        {
            let Some(source_row) = source.scaled_row(dst, clipped.y + offset as i32) else {
                continue;
            };
            let Some(span) = row.get_mut(columns.clone()) else {
                continue;
            };
            for (index, pixel) in span.iter_mut().enumerate() {
                let Some(color) = source_row.sample_scaled(clipped.x + index as i32) else {
                    continue;
                };
                blend_pixel(pixel, color);
            }
        }
    }

    /// The four bytes of one pixel, or `None` if it lies outside the canvas.
    fn pixel_mut(&mut self, point: Point) -> Option<&mut [u8; BYTES_PER_PIXEL]> {
        if point.x < 0 || point.y < 0 || point.x >= self.size.w || point.y >= self.size.h {
            return None;
        }
        // `y < h` and the buffer holds `w * h` pixels, so this cannot overflow.
        let index = point.y as usize * self.size.w as usize + point.x as usize;
        self.pixels
            .as_chunks_mut::<BYTES_PER_PIXEL>()
            .0
            .get_mut(index)
    }

    /// The whole buffer as pixels, or `None` for an empty canvas.
    fn pixels_mut(&mut self) -> Option<&mut [[u8; BYTES_PER_PIXEL]]> {
        let (pixels, _) = self.pixels.as_chunks_mut::<BYTES_PER_PIXEL>();
        (!pixels.is_empty()).then_some(pixels)
    }

    fn byte_len(&self) -> usize {
        (self.size.w as usize) * (self.size.h as usize) * BYTES_PER_PIXEL
    }
}

/// Source-over blend a non-premultiplied color into one ARGB8888 pixel.
fn blend_pixel(pixel: &mut [u8; BYTES_PER_PIXEL], color: Rgba8) {
    let [r, g, b, a] = color.components();
    if a == u8::MAX {
        *pixel = color.little_endian_argb();
        return;
    }
    if a == 0 {
        return;
    }
    let source = u32::from(a);
    let inverse = 255 - source;
    // ARGB8888 on a little-endian host stores [B, G, R, A].
    pixel[0] = blend_channel(b, source, pixel[0], inverse);
    pixel[1] = blend_channel(g, source, pixel[1], inverse);
    pixel[2] = blend_channel(r, source, pixel[2], inverse);
    pixel[3] = (source + u32::from(pixel[3]) * inverse / 255) as u8;
}

#[inline]
fn blend_channel(value: u8, source: u32, target: u8, inverse: u32) -> u8 {
    ((u32::from(value) * source + u32::from(target) * inverse) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 source with a fully opaque, a translucent, a fully transparent and
    /// a premultiplied-looking channel spread across its four pixels.
    const SOURCE: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 40, 80, 120, 255,
    ];

    #[test]
    fn clipped_blits_preserve_source_sampling_and_alpha_blending() {
        let canvas = Size::new(5, 4);
        let source = Size::new(2, 2);

        for x in -7..7 {
            for y in -6..6 {
                for w in 1..9 {
                    for h in 1..8 {
                        let dst = Rect::new(x, y, w, h);
                        // Reference: sample the whole destination, one pixel at
                        // a time, and let clipping drop whatever lands outside.
                        let mut expected = Canvas::new(canvas);
                        for dy in 0..h {
                            for dx in 0..w {
                                let sx = dx * source.w / w;
                                let sy = dy * source.h / h;
                                let offset = ((sy * source.w + sx) * 4) as usize;
                                let components: [u8; 4] =
                                    SOURCE[offset..offset + 4].try_into().unwrap();
                                let color = Rgba8::from(components);
                                expected.fill_pixel(Point::new(x + dx, y + dy), color);
                            }
                        }
                        let mut actual = Canvas::new(canvas);
                        actual.blit_rgba(dst, &SOURCE, source);
                        assert_eq!(
                            actual.as_slice(),
                            expected.as_slice(),
                            "destination: {dst:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn enormous_blit_only_visits_the_visible_canvas() {
        let mut canvas = Canvas::new(Size::new(2, 2));
        canvas.blit_rgba(
            Rect::new(-1_000_000_000, -1_000_000_000, i32::MAX, i32::MAX),
            &[10, 20, 30, 255],
            Size::new(1, 1),
        );
        // Source [10, 20, 30, 255] lands as [B, G, R, A].
        assert_eq!(canvas.as_slice(), [30, 20, 10, 255].repeat(4));
    }

    #[test]
    fn fills_clip_overflowing_edges() {
        let mut canvas = Canvas::new(Size::new(3, 2));
        canvas.fill_rect(
            Rect::new(1, 0, i32::MAX, i32::MAX),
            Rgba8::new(255, 0, 0, 255),
        );
        // Clips to columns 1..3 of both rows, so four pixels are painted. The
        // old raw-buffer API also accepted a short buffer, which silently
        // dropped the second row; the type makes that unrepresentable.
        let mut expected = vec![0; 24];
        expected[4..12].copy_from_slice(&[0, 0, 255, 255].repeat(2));
        expected[16..24].copy_from_slice(&[0, 0, 255, 255].repeat(2));
        assert_eq!(canvas.as_slice(), expected);
    }

    /// A tray icon is a small, fixed, non-premultiplied RGBA8 buffer. The two
    /// backends composite it very differently (the canvas blends in place, the
    /// X11 path premultiplies into an upload buffer), so this pins the
    /// premultiplied conversion the X11 path performs to the same result.
    #[test]
    fn x11_premultiplied_upload_matches_the_source() {
        let canvas_size = Size::new(4, 4);
        let source_size = Size::new(2, 2);
        let source = [
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 40, 80, 120, 255,
        ];
        let mut canvas = Canvas::new(canvas_size);
        canvas.blit_rgba(
            Rect::new(0, 0, canvas_size.w, canvas_size.h),
            &source,
            source_size,
        );
        let premultiply =
            |value: u8, alpha: u32| -> u8 { ((u16::from(value) * alpha as u16 + 127) / 255) as u8 };
        let image = Rgba8Image::new(&source, source_size).unwrap();
        let dst = Rect::new(0, 0, canvas_size.w, canvas_size.h);
        for row in 0..canvas_size.h {
            let source_row = image.scaled_row(dst, row).unwrap();
            for col in 0..canvas_size.w {
                let [r, g, b, a] = source_row.sample_scaled(col).unwrap().components();
                let alpha = u32::from(a);
                // X11 wants premultiplied BGRA on a little-endian host.
                let expected = [
                    premultiply(b, alpha),
                    premultiply(g, alpha),
                    premultiply(r, alpha),
                    a,
                ];
                let offset = ((row * canvas_size.w + col) * 4) as usize;
                assert_eq!(
                    &canvas.as_slice()[offset..offset + 4],
                    &expected,
                    "pixel {col},{row}"
                );
            }
        }
    }

    #[test]
    fn translucent_rects_blend_over_existing_content() {
        let mut canvas = Canvas::new(Size::new(2, 1));
        canvas.fill_rect(canvas.bounds(), Rgba8::new(0, 0, 0, 255));
        canvas.fill_rect(canvas.bounds(), Rgba8::new(255, 255, 255, 128));
        // Halfway to white.
        assert_eq!(canvas.as_slice(), [128, 128, 128, 255].repeat(2));
    }

    #[test]
    fn transparent_colors_are_no_ops() {
        let mut canvas = Canvas::new(Size::new(1, 1));
        canvas.fill_rect(canvas.bounds(), Rgba8::new(255, 0, 0, 255));
        let before = canvas.as_slice().to_vec();
        canvas.fill_rect(canvas.bounds(), Rgba8::ZERO);
        canvas.fill_pixel(Point::new(0, 0), Rgba8::ZERO);
        canvas.blit_rgba(canvas.bounds(), &[0, 255, 0, 0], Size::new(1, 1));
        assert_eq!(canvas.as_slice(), before);
    }

    #[test]
    fn points_and_rectangles_outside_the_canvas_do_not_paint() {
        let mut canvas = Canvas::new(Size::new(1, 1));
        for point in [
            Point::new(-1, 0),
            Point::new(0, -1),
            Point::new(1, 0),
            Point::new(0, 1),
            Point::new(i32::MIN, i32::MIN),
            Point::new(i32::MAX, i32::MAX),
        ] {
            canvas.fill_pixel(point, Rgba8::new(255, 255, 255, 255));
        }
        for rect in [
            Rect::new(1, 0, 1, 1),
            Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            Rect::new(5, 5, 1, 1),
            Rect::new(0, 0, 0, 5),
        ] {
            canvas.fill_rect(rect, Rgba8::new(255, 255, 255, 255));
        }
        canvas.blit_rgba(
            Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            &[255; 4],
            Size::new(1, 1),
        );
        assert_eq!(canvas.as_slice(), [0; 4]);
    }

    #[test]
    fn degenerate_sizes_yield_an_empty_canvas() {
        for size in [
            Size::new(0, 0),
            Size::new(-1, 10),
            Size::new(10, -1),
            Size::new(i32::MAX, i32::MAX),
        ] {
            let canvas = Canvas::new(size);
            assert!(canvas.as_slice().is_empty(), "size: {size:?}");
            assert_eq!(canvas.size(), Size::default());
        }
    }

    #[test]
    fn resize_keeps_the_buffer_consistent_with_the_size() {
        let mut canvas = Canvas::new(Size::new(4, 2));
        assert_eq!(canvas.as_slice().len(), 32);
        canvas.resize(Size::new(2, 2));
        assert_eq!(canvas.as_slice().len(), 16);
        canvas.resize(Size::new(0, 0));
        assert!(canvas.as_slice().is_empty());
    }

    #[test]
    fn truncated_source_is_rejected_without_painting() {
        let mut canvas = Canvas::new(Size::new(1, 1));
        canvas.blit_rgba(canvas.bounds(), &[255; 4], Size::new(2, 2));
        assert_eq!(canvas.as_slice(), [0; 4]);
    }
}
