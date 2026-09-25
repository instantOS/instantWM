//! Non-premultiplied RGBA8 source images shared by both bar backends.
//!
//! Tray icons arrive as raw RGBA8 bitmaps and have to be scaled into a
//! destination rectangle. Both backends do that scaling, so both derive the
//! sample position from the same rule here rather than each re-deriving it from
//! the other's comments.

use crate::types::{Rect, Rgba8, Size};

/// Bytes per pixel in a non-premultiplied RGBA8 image.
const BYTES_PER_PIXEL: usize = 4;

/// A row-major, tightly packed, non-premultiplied RGBA8 image.
///
/// Construction validates that the buffer actually holds `size` pixels, so
/// sampling can index without a length check on every pixel.
#[derive(Clone, Copy)]
pub struct Rgba8Image<'a> {
    pixels: &'a [u8],
    size: Size,
}

impl<'a> Rgba8Image<'a> {
    /// View `pixels` as an image of `size`, or `None` if the dimensions are not
    /// positive or the buffer is too short to hold them.
    pub fn new(pixels: &'a [u8], size: Size) -> Option<Self> {
        if !size.is_positive() {
            return None;
        }
        let needed = (size.w as usize)
            .checked_mul(size.h as usize)?
            .checked_mul(BYTES_PER_PIXEL)?;
        (pixels.len() >= needed).then_some(Self { pixels, size })
    }

    /// The source row that destination row `y` scales from, or `None` if `y`
    /// lies outside `dst`.
    ///
    /// Callers walking a destination row by row should use this rather than
    /// [`Rgba8Image::sample_scaled`]: the vertical scale factor and the
    /// destination's `y` bounds only depend on `y`, so hoisting them out of the
    /// inner loop removes a division and four comparisons per pixel.
    pub fn scaled_row(&self, dst: Rect, y: i32) -> Option<ScaledRow<'_>> {
        // `i64` throughout: `Rect::bottom` overflows for the extreme
        // rectangles the scene can hand us, and `dst` here is unclipped.
        let dy = i64::from(y) - i64::from(dst.y);
        if dy < 0 || dy >= i64::from(dst.h) {
            return None;
        }
        let sy = scale_axis(dy, dst.h, self.size.h);
        Some(ScaledRow {
            pixels: self.pixels,
            // A byte offset, so the inner loop only has to add the column.
            start: sy * self.size.w as usize * BYTES_PER_PIXEL,
            first: dst.x,
            width: dst.w,
            source_width: self.size.w,
        })
    }
}

/// One source row, resolved for a destination row, ready to sample columns.
///
/// Produced by [`Rgba8Image::scaled_row`]; it cannot be constructed any other
/// way, so a `ScaledRow` always refers to a row that actually exists in the
/// image it came from.
#[derive(Clone, Copy, Debug)]
pub struct ScaledRow<'a> {
    pixels: &'a [u8],
    /// Byte offset of the row's first pixel within the image.
    start: usize,
    /// The destination this row was resolved for, as `x`, `w` and the source
    /// width needed to sample a column.
    first: i32,
    width: i32,
    source_width: i32,
}

impl ScaledRow<'_> {
    /// The nearest-neighbour source color for destination column `x`, or `None`
    /// if `x` lies outside the destination this row was resolved for.
    pub fn sample_scaled(&self, x: i32) -> Option<Rgba8> {
        let dx = i64::from(x) - i64::from(self.first);
        if dx < 0 || dx >= i64::from(self.width) {
            return None;
        }
        let sx = scale_axis(dx, self.width, self.source_width);
        let offset = self.start + sx * BYTES_PER_PIXEL;
        let components: [u8; BYTES_PER_PIXEL] = self
            .pixels
            .get(offset..offset + BYTES_PER_PIXEL)?
            .try_into()
            .ok()?;
        Some(Rgba8::from(components))
    }
}

/// Map an in-range destination offset onto a source axis.
#[inline]
fn scale_axis(offset: i64, dst_len: i32, src_len: i32) -> usize {
    debug_assert!(offset >= 0 && offset < i64::from(dst_len));
    (offset * i64::from(src_len) / i64::from(dst_len)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Point;

    fn image() -> Rgba8Image<'static> {
        Rgba8Image::new(
            &[
                255, 0, 0, 255, // (0,0) red
                0, 255, 0, 255, // (1,0) green
                0, 0, 255, 255, // (0,1) blue
                40, 80, 120, 255, // (1,1) grey
            ],
            Size::new(2, 2),
        )
        .unwrap()
    }

    /// Sample one pixel the way a caller walking the destination would.
    fn sample(image: &Rgba8Image<'_>, dst: Rect, point: Point) -> Option<Rgba8> {
        image.scaled_row(dst, point.y)?.sample_scaled(point.x)
    }

    #[test]
    fn rejects_non_positive_and_truncated_buffers() {
        assert!(Rgba8Image::new(&[0; 4], Size::new(0, 1)).is_none());
        assert!(Rgba8Image::new(&[0; 4], Size::new(1, -1)).is_none());
        assert!(Rgba8Image::new(&[0; 15], Size::new(2, 2)).is_none());
        // Trailing bytes are fine; only a short buffer is rejected.
        assert!(Rgba8Image::new(&[0; 17], Size::new(2, 2)).is_some());
    }

    #[test]
    fn scales_nearest_neighbour_across_the_destination() {
        let image = image();
        let dst = Rect::new(0, 0, 4, 4);
        let at = |x, y| sample(&image, dst, Point::new(x, y)).unwrap();

        // A 2x2 source over a 4x4 destination replicates each pixel 2x2.
        assert_eq!(at(0, 0), Rgba8::new(255, 0, 0, 255));
        assert_eq!(at(1, 1), Rgba8::new(255, 0, 0, 255));
        assert_eq!(at(2, 0), Rgba8::new(0, 255, 0, 255));
        assert_eq!(at(0, 2), Rgba8::new(0, 0, 255, 255));
        assert_eq!(at(3, 3), Rgba8::new(40, 80, 120, 255));
    }

    #[test]
    fn sampling_ignores_where_the_destination_is_placed() {
        let image = image();
        // The same offset into the destination must sample the same source
        // pixel no matter where the destination sits. This is what stops a
        // caller from clipping the destination and stretching the icon: the
        // canvas clips by intersecting rectangles, never by resampling.
        let placed = Rect::new(-4, -4, 8, 8);
        let origin = Rect::new(0, 0, 8, 8);
        for dx in 0..8 {
            for dy in 0..8 {
                let offset = Point::new(dx, dy);
                assert_eq!(
                    sample(&image, placed, Point::new(dx - 4, dy - 4)),
                    sample(&image, origin, offset),
                    "offset: {offset:?}"
                );
            }
        }
        assert_eq!(
            sample(&image, origin, Point::new(0, 0)),
            Some(Rgba8::new(255, 0, 0, 255))
        );
        assert_eq!(sample(&image, origin, Point::new(-1, 0)), None);
        assert_eq!(sample(&image, origin, Point::new(8, 0)), None);
    }

    #[test]
    fn a_resolved_row_reuses_its_source_offset() {
        let image = image();
        let dst = Rect::new(0, 0, 4, 4);
        // A row resolved once must agree with a freshly resolved one, which is
        // what makes hoisting the vertical scale out of the inner loop safe.
        for y in 0..4 {
            let row = image.scaled_row(dst, y).unwrap();
            for x in 0..4 {
                let again = image.scaled_row(dst, y).unwrap();
                assert_eq!(row.sample_scaled(x), again.sample_scaled(x), "{x},{y}");
            }
        }
        assert!(image.scaled_row(dst, -1).is_none());
        assert!(image.scaled_row(dst, 4).is_none());
    }

    #[test]
    fn extreme_destinations_do_not_overflow() {
        let image = image();
        for dst in [
            Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            Rect::new(i32::MAX, i32::MAX, i32::MAX, i32::MAX),
            Rect::new(0, 0, i32::MAX, 1),
        ] {
            // The only requirement is that this terminates and agrees with
            // clamping; no panic, no wraparound.
            for point in [
                Point::new(i32::MIN, 0),
                Point::new(i32::MAX, i32::MAX),
                Point::new(0, 0),
            ] {
                let _ = sample(&image, dst, point);
            }
        }
    }
}
