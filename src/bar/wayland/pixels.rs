use crate::types::geometry::Rect;

pub(super) fn fill_pixel(
    pixels: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    x: i32,
    y: i32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    if x < 0 || y < 0 || x >= canvas_w || y >= canvas_h {
        return;
    }
    let idx = ((y * canvas_w + x) * 4) as usize;
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

pub(super) fn fill_rect(
    pixels: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    rect: Rect,
    color: [f32; 4],
) {
    let r = (color[0] * 255.0) as u8;
    let g = (color[1] * 255.0) as u8;
    let b = (color[2] * 255.0) as u8;
    let a = (color[3] * 255.0) as u8;
    let x_end = (rect.x + rect.w).min(canvas_w);
    let y_end = (rect.y + rect.h).min(canvas_h);
    let x_start = rect.x.max(0);
    let y_start = rect.y.max(0);

    if a == 255 {
        for py in y_start..y_end {
            let row_start = ((py * canvas_w + x_start) * 4) as usize;
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
                fill_pixel(pixels, canvas_w, canvas_h, px, py, r, g, b, a);
            }
        }
    }
}

pub(super) fn blit_rgba_scaled(
    pixels: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    dst: Rect,
    src_w: i32,
    src_h: i32,
    src_rgba: &[u8],
) {
    if dst.w <= 0 || dst.h <= 0 || src_w <= 0 || src_h <= 0 {
        return;
    }
    let needed = (src_w as usize)
        .checked_mul(src_h as usize)
        .and_then(|v| v.checked_mul(4))
        .unwrap_or(0);
    if src_rgba.len() < needed {
        return;
    }

    for y in 0..dst.h {
        let sy = (y as i64 * src_h as i64 / dst.h as i64) as i32;
        for x in 0..dst.w {
            let sx = (x as i64 * src_w as i64 / dst.w as i64) as i32;
            let si = ((sy * src_w + sx) * 4) as usize;
            if si + 3 >= src_rgba.len() {
                continue;
            }
            fill_pixel(
                pixels,
                canvas_w,
                canvas_h,
                dst.x + x,
                dst.y + y,
                src_rgba[si],
                src_rgba[si + 1],
                src_rgba[si + 2],
                src_rgba[si + 3],
            );
        }
    }
}
