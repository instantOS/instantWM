use crate::bar::color::Rgba;
use crate::types::{Point, Rect, Size};

pub(super) fn fill_pixel(pixels: &mut [u8], canvas_size: Size, point: Point, color: [u8; 4]) {
    let [r, g, b, a] = color;
    if point.x < 0 || point.y < 0 || point.x >= canvas_size.w || point.y >= canvas_size.h {
        return;
    }
    let idx = ((point.y * canvas_size.w + point.x) * 4) as usize;
    if idx + 3 >= pixels.len() {
        return;
    }
    // ARGB8888: [B, G, R, A] in little-endian.
    if a == 255 {
        pixels[idx] = b;
        pixels[idx + 1] = g;
        pixels[idx + 2] = r;
        pixels[idx + 3] = a;
    } else if a > 0 {
        let sa = a as u32;
        let ia = 255 - sa;
        pixels[idx] = ((b as u32 * sa + pixels[idx] as u32 * ia) / 255) as u8;
        pixels[idx + 1] = ((g as u32 * sa + pixels[idx + 1] as u32 * ia) / 255) as u8;
        pixels[idx + 2] = ((r as u32 * sa + pixels[idx + 2] as u32 * ia) / 255) as u8;
        pixels[idx + 3] = (sa + (pixels[idx + 3] as u32 * ia) / 255) as u8;
    }
}

pub(super) fn fill_rect(pixels: &mut [u8], canvas_size: Size, rect: Rect, color: Rgba) {
    let [r, g, b, a] = color.to_rgba8();
    let x_end = (rect.x + rect.w).min(canvas_size.w);
    let y_end = (rect.y + rect.h).min(canvas_size.h);
    let x_start = rect.x.max(0);
    let y_start = rect.y.max(0);

    if a == 255 {
        for py in y_start..y_end {
            let row_start = ((py * canvas_size.w + x_start) * 4) as usize;
            for px in 0..(x_end - x_start) {
                let idx = row_start + (px * 4) as usize;
                if idx + 3 < pixels.len() {
                    pixels[idx] = b;
                    pixels[idx + 1] = g;
                    pixels[idx + 2] = r;
                    pixels[idx + 3] = a;
                }
            }
        }
    } else {
        for py in y_start..y_end {
            for px in x_start..x_end {
                fill_pixel(pixels, canvas_size, Point::new(px, py), [r, g, b, a]);
            }
        }
    }
}

pub(super) fn blit_rgba_scaled(
    pixels: &mut [u8],
    canvas_size: Size,
    dst: Rect,
    source_size: Size,
    src_rgba: &[u8],
) {
    if !dst.size().is_positive() || !source_size.is_positive() {
        return;
    }
    let needed = (source_size.w as usize)
        .checked_mul(source_size.h as usize)
        .and_then(|v| v.checked_mul(4))
        .unwrap_or(0);
    if src_rgba.len() < needed {
        return;
    }

    for y in 0..dst.h {
        let sy = (y as i64 * source_size.h as i64 / dst.h as i64) as i32;
        for x in 0..dst.w {
            let sx = (x as i64 * source_size.w as i64 / dst.w as i64) as i32;
            let si = ((sy * source_size.w + sx) * 4) as usize;
            if si + 3 >= src_rgba.len() {
                continue;
            }
            fill_pixel(
                pixels,
                canvas_size,
                Point::new(dst.x + x, dst.y + y),
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
