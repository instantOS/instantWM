//! Wayland bar rendering using cosmic-text for text and MemoryRenderBuffer for output.
//!
//! The bar is rasterized into a single RGBA pixel buffer per monitor,
//! then uploaded as a Smithay MemoryRenderBuffer for compositing.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Scale, Transform};

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::bar::paint::{BarPainter, BarScheme};
use crate::bar::renderer::draw_bar_common;

const DEFAULT_FONT_SIZE: f32 = 14.0;

// Pixel buffer operations (freestanding to avoid borrow conflicts)
fn pixel_fill(
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
    // ARGB8888: [B, G, R, A] in little-endian
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

fn pixel_fill_rect(
    pixels: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: [f32; 4],
) {
    let r = (color[0] * 255.0) as u8;
    let g = (color[1] * 255.0) as u8;
    let b = (color[2] * 255.0) as u8;
    let a = (color[3] * 255.0) as u8;
    let x_end = (x + w).min(canvas_w);
    let y_end = (y + h).min(canvas_h);
    let x_start = x.max(0);
    let y_start = y.max(0);
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
                pixel_fill(pixels, canvas_w, canvas_h, px, py, r, g, b, a);
            }
        }
    }
}

fn measure_width(fs: &mut FontSystem, text: &str, font_size: f32) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let metrics = Metrics::new(font_size, font_size);
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_size(fs, None, None);
    buffer.set_wrap(fs, Wrap::None);
    buffer.set_text(fs, text, Attrs::new(), Shaping::Advanced);
    buffer.shape_until_scroll(fs, false);
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0_f32, f32::max)
        .ceil() as i32
}

fn rasterize_text(
    pixels: &mut [u8],
    canvas_w: i32,
    canvas_h: i32,
    fs: &mut FontSystem,
    sc: &mut SwashCache,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    text: &str,
    color: [f32; 4],
    font_size: f32,
) {
    if text.is_empty() || w <= 0 || h <= 0 {
        return;
    }
    let metrics = Metrics::new(font_size, h as f32);
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_size(fs, Some(w as f32), Some(h as f32));
    buffer.set_wrap(fs, Wrap::None);
    let cosmic_color = CosmicColor::rgba(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    );
    let attrs = Attrs::new().color(cosmic_color);
    buffer.set_text(fs, text, attrs, Shaping::Advanced);
    buffer.shape_until_scroll(fs, false);

    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            let glyph_color = glyph.color_opt.unwrap_or(cosmic_color);
            let image = sc.get_image(fs, physical.cache_key);
            let Some(image) = image else { continue };

            let gx = x + physical.x + image.placement.left;
            let gy = y + run.line_y as i32 + physical.y - image.placement.top;

            match image.content {
                cosmic_text::SwashContent::Mask => {
                    let pw = image.placement.width as i32;
                    for row in 0..image.placement.height as i32 {
                        for col in 0..pw {
                            let mask_idx = (row * pw + col) as usize;
                            if mask_idx >= image.data.len() {
                                continue;
                            }
                            let alpha = image.data[mask_idx];
                            if alpha == 0 {
                                continue;
                            }
                            let a = (alpha as u32 * glyph_color.a() as u32) / 255;
                            pixel_fill(
                                pixels,
                                canvas_w,
                                canvas_h,
                                gx + col,
                                gy + row,
                                glyph_color.r(),
                                glyph_color.g(),
                                glyph_color.b(),
                                a as u8,
                            );
                        }
                    }
                }
                cosmic_text::SwashContent::Color => {
                    let pw = image.placement.width as i32;
                    for row in 0..image.placement.height as i32 {
                        for col in 0..pw {
                            let si = ((row * pw + col) * 4) as usize;
                            if si + 3 < image.data.len() {
                                pixel_fill(
                                    pixels,
                                    canvas_w,
                                    canvas_h,
                                    gx + col,
                                    gy + row,
                                    image.data[si],
                                    image.data[si + 1],
                                    image.data[si + 2],
                                    image.data[si + 3],
                                );
                            }
                        }
                    }
                }
                cosmic_text::SwashContent::SubpixelMask => {}
            }
        }
    }
}

pub struct WaylandBarPainter {
    font_system: RefCell<FontSystem>,
    swash_cache: RefCell<SwashCache>,
    text_width_cache: RefCell<HashMap<String, i32>>,
    scheme: Option<BarScheme>,
    pixels: Vec<u8>,
    canvas_w: i32,
    canvas_h: i32,
    origin_x: i32,
    origin_y: i32,
    font_size: f32,
    buffers: Vec<BarBuffer>,
    cached_buffers: Vec<BarBuffer>,
    cached_key: u64,
    has_cached_buffers: bool,
}

struct BarBuffer {
    buffer: MemoryRenderBuffer,
    x: i32,
    y: i32,
}

impl Clone for BarBuffer {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            x: self.x,
            y: self.y,
        }
    }
}

impl Default for WaylandBarPainter {
    fn default() -> Self {
        Self {
            font_system: RefCell::new(FontSystem::new()),
            swash_cache: RefCell::new(SwashCache::new()),
            text_width_cache: RefCell::new(HashMap::new()),
            scheme: None,
            pixels: Vec::new(),
            canvas_w: 0,
            canvas_h: 0,
            origin_x: 0,
            origin_y: 0,
            font_size: DEFAULT_FONT_SIZE,
            buffers: Vec::new(),
            cached_buffers: Vec::new(),
            cached_key: 0,
            has_cached_buffers: false,
        }
    }
}

impl WaylandBarPainter {
    pub fn set_font_size(&mut self, font_size: f32) {
        if font_size.is_finite() && font_size > 0.0 {
            self.font_size = font_size;
            self.text_width_cache.borrow_mut().clear();
        }
    }

    fn text_width_cached(&self, text: &str) -> i32 {
        if text.is_empty() {
            return 0;
        }
        if let Some(width) = self.text_width_cache.borrow().get(text).copied() {
            return width;
        }

        let width = {
            let mut fs = self.font_system.borrow_mut();
            measure_width(&mut fs, text, self.font_size)
        };

        let mut cache = self.text_width_cache.borrow_mut();
        if cache.len() > 2048 {
            cache.clear();
        }
        cache.insert(text.to_string(), width);
        width
    }

    /// Measure text width without requiring `&mut self` — used for hit-testing.
    pub fn measure_text_width(&self, text: &str) -> i32 {
        self.text_width_cached(text)
    }

    pub fn begin(
        &mut self,
        _scale: Scale<f64>,
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
    ) {
        self.scheme = None;
        self.origin_x = origin_x;
        self.origin_y = origin_y;
        self.canvas_w = width;
        self.canvas_h = height;
        let size = (width as usize) * (height as usize) * 4;
        self.pixels.clear();
        self.pixels.resize(size, 0);
    }

    pub fn finish(&mut self) {
        if self.canvas_w <= 0 || self.canvas_h <= 0 {
            return;
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &self.pixels,
            Fourcc::Argb8888,
            (self.canvas_w, self.canvas_h),
            1,
            Transform::Normal,
            None,
        );
        self.buffers.push(BarBuffer {
            buffer,
            x: self.origin_x,
            y: self.origin_y,
        });
    }

    pub fn take_buffers(&mut self) -> Vec<(MemoryRenderBuffer, i32, i32)> {
        self.buffers
            .drain(..)
            .map(|b| (b.buffer, b.x, b.y))
            .collect()
    }
}

impl BarPainter for WaylandBarPainter {
    fn text_width(&mut self, text: &str) -> i32 {
        self.text_width_cached(text)
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        self.scheme = Some(scheme);
    }

    fn scheme(&self) -> Option<&BarScheme> {
        self.scheme.as_ref()
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, filled: bool, invert: bool) {
        if !filled || w <= 0 || h <= 0 {
            return;
        }
        let Some(scheme) = self.scheme.clone() else {
            return;
        };
        let color = scheme.rect_color(invert);
        pixel_fill_rect(
            &mut self.pixels,
            self.canvas_w,
            self.canvas_h,
            x,
            y,
            w,
            h,
            color,
        );
    }

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
    ) -> i32 {
        let Some(scheme) = self.scheme.clone() else {
            return x;
        };
        let (bg, fg) = scheme.text_colors(invert);
        pixel_fill_rect(
            &mut self.pixels,
            self.canvas_w,
            self.canvas_h,
            x,
            y,
            w,
            h,
            bg,
        );
        if detail_height > 0 {
            pixel_fill_rect(
                &mut self.pixels,
                self.canvas_w,
                self.canvas_h,
                x,
                y + h - detail_height,
                w,
                detail_height,
                scheme.detail,
            );
        }
        if !text.is_empty() {
            let text_x = x + lpad;
            let text_w = (w - lpad).max(0);
            if text_w > 0 {
                let mut fs = self.font_system.borrow_mut();
                let mut sc = self.swash_cache.borrow_mut();
                rasterize_text(
                    &mut self.pixels,
                    self.canvas_w,
                    self.canvas_h,
                    &mut fs,
                    &mut sc,
                    text_x,
                    y,
                    text_w,
                    h,
                    text,
                    fg,
                    self.font_size,
                );
            }
        }
        x + w
    }
}

pub fn draw_bar_wayland(ctx: &mut crate::contexts::WmCtx, mon_idx: usize) {
    draw_bar_common_with_painter(ctx, mon_idx);
}

pub fn draw_bars_wayland(ctx: &mut crate::contexts::WmCtx) {
    ctx.g.status_text_width =
        crate::bar::renderer::compute_status_hit_width(ctx.bar_painter, &ctx.g.status_text);
}

pub fn reset_bar_wayland(ctx: &mut crate::contexts::WmCtx) {
    crate::bar::renderer::reset_bar_common(ctx);
}

pub fn should_draw_bar_wayland(ctx: &crate::contexts::WmCtx) -> bool {
    ctx.g.cfg.showbar
}

pub fn render_bar_buffers(
    ctx: &mut crate::contexts::WmCtx,
    scale: Scale<f64>,
) -> Vec<(MemoryRenderBuffer, i32, i32)> {
    let key = bar_render_key(ctx);
    if ctx.bar_painter.has_cached_buffers && ctx.bar_painter.cached_key == key {
        return ctx
            .bar_painter
            .cached_buffers
            .iter()
            .map(|b| (b.buffer.clone(), b.x, b.y))
            .collect();
    }

    let mon_indices: Vec<(usize, i32, i32, i32, i32)> = ctx
        .g
        .monitors_iter()
        .filter_map(|(i, m)| {
            if !m.shows_bar() {
                return None;
            }
            Some((
                i,
                m.work_rect.x,
                m.bar_y,
                m.work_rect.w,
                ctx.g.cfg.bar_height,
            ))
        })
        .collect();

    for (mon_idx, origin_x, origin_y, width, height) in mon_indices {
        ctx.bar_painter
            .begin(scale, origin_x, origin_y, width, height);
        draw_bar_common_with_painter(ctx, mon_idx);
        ctx.bar_painter.finish();
    }

    let rendered = ctx.bar_painter.take_buffers();
    ctx.bar_painter.cached_buffers = rendered
        .iter()
        .map(|(buffer, x, y)| BarBuffer {
            buffer: buffer.clone(),
            x: *x,
            y: *y,
        })
        .collect();
    ctx.bar_painter.cached_key = key;
    ctx.bar_painter.has_cached_buffers = true;

    rendered
}

fn draw_bar_common_with_painter(ctx: &mut crate::contexts::WmCtx, mon_idx: usize) {
    let painter_ptr = ctx.bar_painter as *mut WaylandBarPainter;
    let ctx_ptr = ctx as *mut crate::contexts::WmCtx;
    unsafe {
        draw_bar_common(&mut *ctx_ptr, mon_idx, &mut *painter_ptr);
    }
}

fn hash_gesture(hasher: &mut DefaultHasher, gesture: crate::types::Gesture) {
    match gesture {
        crate::types::Gesture::None => 0u8.hash(hasher),
        crate::types::Gesture::WinTitle(win) => {
            1u8.hash(hasher);
            win.hash(hasher);
        }
        crate::types::Gesture::Tag(tag) => {
            2u8.hash(hasher);
            tag.hash(hasher);
        }
        crate::types::Gesture::Overlay => 3u8.hash(hasher),
        crate::types::Gesture::CloseButton => 4u8.hash(hasher),
        crate::types::Gesture::StartMenu => 5u8.hash(hasher),
    }
}

//TODO: document what this does
fn bar_render_key(ctx: &crate::contexts::WmCtx) -> u64 {
    let mut hasher = DefaultHasher::new();
    ctx.g.cfg.showbar.hash(&mut hasher);
    ctx.g.cfg.bar_height.hash(&mut hasher);
    ctx.g.cfg.horizontal_padding.hash(&mut hasher);
    ctx.g.cfg.startmenusize.hash(&mut hasher);
    ctx.g.drag.bar_active.hash(&mut hasher);
    ctx.g.status_text.hash(&mut hasher);
    ctx.g.selected_monitor_id().hash(&mut hasher);

    for m in ctx.g.monitors_iter_all() {
        m.num.hash(&mut hasher);
        m.work_rect.x.hash(&mut hasher);
        m.work_rect.y.hash(&mut hasher);
        m.work_rect.w.hash(&mut hasher);
        m.work_rect.h.hash(&mut hasher);
        m.bar_y.hash(&mut hasher);
        m.showbar.hash(&mut hasher);
        m.current_tag.hash(&mut hasher);
        m.sel.hash(&mut hasher);
        hash_gesture(&mut hasher, m.gesture);
        if let Some(tag) = m.current_tag() {
            tag.showbar.hash(&mut hasher);
            tag.name.hash(&mut hasher);
            tag.alt_name.hash(&mut hasher);
            tag.layouts.symbol().hash(&mut hasher);
        }

        let selected = m.selected_tags();
        for (win, c) in m.iter_clients(ctx.g.clients.map()) {
            if !c.is_visible_on_tags(selected) {
                continue;
            }
            win.hash(&mut hasher);
            c.name.hash(&mut hasher);
            c.tags.hash(&mut hasher);
            c.isurgent.hash(&mut hasher);
            c.islocked.hash(&mut hasher);
            c.is_fullscreen.hash(&mut hasher);
            c.is_hidden.hash(&mut hasher);
        }
    }

    hasher.finish()
}
