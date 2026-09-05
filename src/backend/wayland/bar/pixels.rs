use crate::types::{Point, Rect, Rgba, Size};

fn clipped_rect(canvas_size: Size, rect: Rect) -> Option<Rect> {
    if !canvas_size.is_positive() || !rect.size().is_positive() {
        return None;
    }
    let x = rect.x.max(0);
    let y = rect.y.max(0);
    let right = (i64::from(rect.x) + i64::from(rect.w)).min(i64::from(canvas_size.w));
    let bottom = (i64::from(rect.y) + i64::from(rect.h)).min(i64::from(canvas_size.h));
    if right <= i64::from(x) || bottom <= i64::from(y) {
        return None;
    }
    Some(Rect::new(x, y, right as i32 - x, bottom as i32 - y))
}

fn pixel_offset(canvas_size: Size, point: Point) -> Option<usize> {
    (point.y as usize)
        .checked_mul(canvas_size.w as usize)?
        .checked_add(point.x as usize)?
        .checked_mul(4)
}

pub(super) fn fill_pixel(pixels: &mut [u8], canvas_size: Size, point: Point, color: [u8; 4]) {
    let [r, g, b, a] = color;
    if point.x < 0 || point.y < 0 || point.x >= canvas_size.w || point.y >= canvas_size.h {
        return;
    }
    let Some(idx) = pixel_offset(canvas_size, point) else {
        return;
    };
    if pixels.len().saturating_sub(idx) < 4 {
        return;
    }
    // ARGB8888: [B, G, R, A] in little-endian.
    if a == 255 {
        pixels[idx] = b;
        pixels[idx + 1] = g;
        pixels[idx + 2] = r;
        pixels[idx + 3] = a;
    } else if a > 0 {
        let source_alpha = a as u32;
        let inverse_alpha = 255 - source_alpha;
        pixels[idx] = ((b as u32 * source_alpha + pixels[idx] as u32 * inverse_alpha) / 255) as u8;
        pixels[idx + 1] =
            ((g as u32 * source_alpha + pixels[idx + 1] as u32 * inverse_alpha) / 255) as u8;
        pixels[idx + 2] =
            ((r as u32 * source_alpha + pixels[idx + 2] as u32 * inverse_alpha) / 255) as u8;
        pixels[idx + 3] = (source_alpha + (pixels[idx + 3] as u32 * inverse_alpha) / 255) as u8;
    }
}

pub(super) fn fill_rect(pixels: &mut [u8], canvas_size: Size, rect: Rect, color: Rgba) {
    let Some(rect) = clipped_rect(canvas_size, rect) else {
        return;
    };
    let [r, g, b, a] = color.to_rgba8();
    let x_end = rect.right();
    let y_end = rect.bottom();
    let x_start = rect.x;
    let y_start = rect.y;

    if a == 255 {
        for py in y_start..y_end {
            let Some(row_start) = pixel_offset(canvas_size, Point::new(x_start, py)) else {
                return;
            };
            let Some(row_end) = pixel_offset(canvas_size, Point::new(x_end, py)) else {
                return;
            };
            let row_end = row_end.min(pixels.len());
            let Some(row) = pixels.get_mut(row_start..row_end) else {
                return;
            };
            row.as_chunks_mut::<4>().0.fill([b, g, r, a]);
        }
    } else {
        for py in y_start..y_end {
            for px in x_start..x_end {
                fill_pixel(pixels, canvas_size, Point::new(px, py), [r, g, b, a]);
            }
        }
    }
}

pub(crate) fn blit_rgba_scaled(
    pixels: &mut [u8],
    canvas_size: Size,
    dst: Rect,
    source_size: Size,
    src_rgba: &[u8],
) {
    let Some(clipped) = clipped_rect(canvas_size, dst) else {
        return;
    };
    if !source_size.is_positive() {
        return;
    }
    let Some(needed) = (source_size.w as usize)
        .checked_mul(source_size.h as usize)
        .and_then(|v| v.checked_mul(4))
    else {
        return;
    };
    if src_rgba.len() < needed {
        return;
    }

    // Clip the iteration, keeping sampling relative to the original target so
    // clipping cannot stretch the visible part of the icon.
    for y in clipped.y..clipped.bottom() {
        let sy = ((i64::from(y) - i64::from(dst.y)) * i64::from(source_size.h) / i64::from(dst.h))
            as usize;
        for x in clipped.x..clipped.right() {
            let sx = ((i64::from(x) - i64::from(dst.x)) * i64::from(source_size.w)
                / i64::from(dst.w)) as usize;
            let si = (sy * source_size.w as usize + sx) * 4;
            fill_pixel(
                pixels,
                canvas_size,
                Point::new(x, y),
                [
                    src_rgba[si],
                    src_rgba[si + 1],
                    src_rgba[si + 2],
                    src_rgba[si + 3],
                ],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_blits_preserve_source_sampling_and_alpha_blending() {
        let canvas = Size::new(5, 4);
        let source = Size::new(2, 2);
        let rgba = [
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 40, 80, 120, 255,
        ];

        for x in -7..7 {
            for y in -6..6 {
                for w in 1..9 {
                    for h in 1..8 {
                        let dst = Rect::new(x, y, w, h);
                        let mut expected = vec![32; 5 * 4 * 4];
                        // Reference: sample the whole destination, then let
                        // fill_pixel discard pixels outside the canvas.
                        for dy in 0..h {
                            for dx in 0..w {
                                let sx = dx * source.w / w;
                                let sy = dy * source.h / h;
                                let offset = ((sy * source.w + sx) * 4) as usize;
                                fill_pixel(
                                    &mut expected,
                                    canvas,
                                    Point::new(x + dx, y + dy),
                                    rgba[offset..offset + 4].try_into().unwrap(),
                                );
                            }
                        }
                        let mut actual = vec![32; expected.len()];
                        blit_rgba_scaled(&mut actual, canvas, dst, source, &rgba);
                        assert_eq!(actual, expected, "destination: {dst:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn enormous_blit_only_visits_the_visible_canvas() {
        let mut pixels = vec![0; 16];
        blit_rgba_scaled(
            &mut pixels,
            Size::new(2, 2),
            Rect::new(-1_000_000_000, -1_000_000_000, i32::MAX, i32::MAX),
            Size::new(1, 1),
            &[10, 20, 30, 255],
        );
        assert_eq!(pixels, [30, 20, 10, 255].repeat(4));
    }

    #[test]
    fn fills_clip_overflowing_edges_and_partial_pixel_buffers() {
        let mut pixels = vec![0; 19];
        fill_rect(
            &mut pixels,
            Size::new(3, 2),
            Rect::new(1, 0, i32::MAX, i32::MAX),
            Rgba::new(1.0, 0.0, 0.0, 1.0),
        );
        let mut expected = vec![0; 19];
        expected[4..12].copy_from_slice(&[0, 0, 255, 255].repeat(2));
        assert_eq!(pixels, expected);
    }

    #[test]
    fn large_canvas_offsets_and_outside_rectangles_do_not_overflow() {
        let mut pixels = [0; 4];
        fill_pixel(
            &mut pixels,
            Size::new(i32::MAX, i32::MAX),
            Point::new(i32::MAX - 1, i32::MAX - 1),
            [255; 4],
        );
        fill_rect(
            &mut pixels,
            Size::new(1, 1),
            Rect::new(i32::MAX, i32::MAX, 1, 1),
            Rgba::new(1.0, 1.0, 1.0, 1.0),
        );
        blit_rgba_scaled(
            &mut pixels,
            Size::new(1, 1),
            Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            Size::new(1, 1),
            &[255; 4],
        );
        assert_eq!(pixels, [0; 4]);
    }

    #[test]
    fn truncated_source_is_rejected_without_painting() {
        let mut pixels = [0; 4];
        blit_rgba_scaled(
            &mut pixels,
            Size::new(1, 1),
            Rect::new(0, 0, 1, 1),
            Size::new(2, 2),
            &[255; 4],
        );
        assert_eq!(pixels, [0; 4]);
    }
}
