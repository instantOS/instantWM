#![allow(clippy::too_many_arguments)]
//! Wayland bar rendering using cosmic-text for text and MemoryRenderBuffer for output.
//!
//! The bar is rasterized into a single RGBA pixel buffer per monitor,
//! then uploaded as a Smithay MemoryRenderBuffer for compositing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Scale, Transform};

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::bar::paint::{BarPainter, BarScheme};

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
    let cosmic_color = CosmicColor::rgba(
        (color[0] * 255.0) as u8,
        (color[1] * 255.0) as u8,
        (color[2] * 255.0) as u8,
        (color[3] * 255.0) as u8,
    );
    let metrics = Metrics::new(font_size, h as f32);
    let mut buffer = Buffer::new(fs, metrics);
    buffer.set_size(fs, Some(w as f32), Some(h as f32));
    buffer.set_wrap(fs, Wrap::None);
    buffer.set_text(fs, text, Attrs::new(), Shaping::Advanced);
    buffer.shape_until_scroll(fs, false);
    buffer.draw(fs, sc, cosmic_color, |gx, gy, _, _, color| {
        if gx < 0 || gy < 0 || gx >= w || gy >= h {
            return;
        }
        pixel_fill(
            pixels,
            canvas_w,
            canvas_h,
            x + gx,
            y + gy,
            color.r(),
            color.g(),
            color.b(),
            color.a(),
        );
    });
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TextMeasureKey {
    text: String,
    font_size_bits: u32,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct TextRenderKey {
    text: String,
    width: i32,
    height: i32,
    font_size_bits: u32,
}

struct CachedMeasuredText {
    buffer: Buffer,
    width: i32,
}

struct CachedRenderedText {
    buffer: Buffer,
}

pub struct WaylandBarPainter {
    font_system: RefCell<FontSystem>,
    swash_cache: RefCell<SwashCache>,
    text_measure_cache: RefCell<HashMap<TextMeasureKey, CachedMeasuredText>>,
    text_render_cache: RefCell<HashMap<TextRenderKey, CachedRenderedText>>,
    scheme: Option<BarScheme>,
    pixels: Vec<u8>,
    canvas_w: i32,
    canvas_h: i32,
    origin_x: i32,
    origin_y: i32,
    font_size: f32,
    buffers: Vec<BarBuffer>,
    cached_monitors: HashMap<usize, CachedMonitorBar>,
}

pub struct BarBuffer {
    pub buffer: MemoryRenderBuffer,
    pub x: i32,
    pub y: i32,
}

struct CachedMonitorBar {
    key: u64,
    buffer: BarBuffer,
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
            text_measure_cache: RefCell::new(HashMap::new()),
            text_render_cache: RefCell::new(HashMap::new()),
            scheme: None,
            pixels: Vec::new(),
            canvas_w: 0,
            canvas_h: 0,
            origin_x: 0,
            origin_y: 0,
            font_size: DEFAULT_FONT_SIZE,
            buffers: Vec::new(),
            cached_monitors: HashMap::new(),
        }
    }
}

impl WaylandBarPainter {
    pub fn set_font_size(&mut self, font_size: f32) {
        if font_size.is_finite() && font_size > 0.0 {
            self.font_size = font_size;
            self.text_measure_cache.borrow_mut().clear();
            self.text_render_cache.borrow_mut().clear();
        }
    }

    fn text_width_cached(&self, text: &str) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let key = TextMeasureKey {
            text: text.to_string(),
            font_size_bits: self.font_size.to_bits(),
        };

        if let Some(cached) = self.text_measure_cache.borrow().get(&key) {
            return cached.width;
        }

        let cached = {
            let mut fs = self.font_system.borrow_mut();
            let metrics = Metrics::new(self.font_size, self.font_size);
            let mut buffer = Buffer::new(&mut fs, metrics);
            buffer.set_size(&mut fs, None, None);
            buffer.set_wrap(&mut fs, Wrap::None);
            buffer.set_text(&mut fs, text, Attrs::new(), Shaping::Advanced);
            buffer.shape_until_scroll(&mut fs, false);
            let width = buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0_f32, f32::max)
                .ceil() as i32;
            CachedMeasuredText { buffer, width }
        };

        let width = cached.width;
        let mut cache = self.text_measure_cache.borrow_mut();
        if cache.len() > 2048 {
            cache.clear();
        }
        cache.insert(key, cached);
        width
    }

    fn rasterize_text_cached(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        text: &str,
        color: [f32; 4],
    ) {
        if text.is_empty() || w <= 0 || h <= 0 {
            return;
        }

        let cosmic_color = CosmicColor::rgba(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            (color[3] * 255.0) as u8,
        );
        let key = TextRenderKey {
            text: text.to_string(),
            width: w,
            height: h,
            font_size_bits: self.font_size.to_bits(),
        };

        {
            let mut cache = self.text_render_cache.borrow_mut();
            if !cache.contains_key(&key) {
                let mut fs = self.font_system.borrow_mut();
                let metrics = Metrics::new(self.font_size, h as f32);
                let mut buffer = Buffer::new(&mut fs, metrics);
                buffer.set_size(&mut fs, Some(w as f32), Some(h as f32));
                buffer.set_wrap(&mut fs, Wrap::None);
                buffer.set_text(&mut fs, text, Attrs::new(), Shaping::Advanced);
                buffer.shape_until_scroll(&mut fs, false);
                if cache.len() > 2048 {
                    cache.clear();
                }
                cache.insert(key.clone(), CachedRenderedText { buffer });
            }
        }

        let mut fs = self.font_system.borrow_mut();
        let mut sc = self.swash_cache.borrow_mut();
        let cache = self.text_render_cache.borrow();
        let Some(cached) = cache.get(&key) else { return };

        cached.buffer.draw(&mut fs, &mut sc, cosmic_color, |gx, gy, _, _, color| {
            if gx < 0 || gy < 0 || gx >= w || gy >= h {
                return;
            }
            pixel_fill(
                &mut self.pixels,
                self.canvas_w,
                self.canvas_h,
                x + gx,
                y + gy,
                color.r(),
                color.g(),
                color.b(),
                color.a(),
            );
        });
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

    pub fn blit_rgba_bgra(
        &mut self,
        dst_x: i32,
        dst_y: i32,
        dst_w: i32,
        dst_h: i32,
        src_w: i32,
        src_h: i32,
        src_rgba: &[u8],
    ) {
        if dst_w <= 0 || dst_h <= 0 || src_w <= 0 || src_h <= 0 {
            return;
        }
        let needed = (src_w as usize)
            .checked_mul(src_h as usize)
            .and_then(|v| v.checked_mul(4))
            .unwrap_or(0);
        if src_rgba.len() < needed {
            return;
        }

        for y in 0..dst_h {
            let sy = (y as i64 * src_h as i64 / dst_h as i64) as i32;
            for x in 0..dst_w {
                let sx = (x as i64 * src_w as i64 / dst_w as i64) as i32;
                let si = ((sy * src_w + sx) * 4) as usize;
                if si + 3 >= src_rgba.len() {
                    continue;
                }
                let r = src_rgba[si];
                let g = src_rgba[si + 1];
                let b = src_rgba[si + 2];
                let a = src_rgba[si + 3];
                pixel_fill(
                    &mut self.pixels,
                    self.canvas_w,
                    self.canvas_h,
                    dst_x + x,
                    dst_y + y,
                    r,
                    g,
                    b,
                    a,
                );
            }
        }
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
                self.rasterize_text_cached(text_x, y, text_w, h, text, fg);
            }
        }
        x + w
    }
}

pub fn render_bar_buffers(
    core: &mut crate::contexts::CoreCtx,
    painter: &mut WaylandBarPainter,
    scale: Scale<f64>,
    wayland_systray: &crate::types::WaylandSystray,
    wayland_systray_menu: Option<&crate::types::WaylandSystrayMenu>,
) -> Vec<(MemoryRenderBuffer, i32, i32)> {
    // Cache the systray width so status bar layout can account for it.
    core.globals_mut().bar_runtime.systray_width =
        crate::systray::wayland::get_wayland_systray_width_with_state(core, wayland_systray);

    let mon_indices: Vec<(usize, i32, i32, i32, i32, usize)> = core
        .globals()
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
                core.globals().cfg.bar_height,
                m.id(),
            ))
        })
        .collect();

    painter
        .cached_monitors
        .retain(|monitor_id, _| mon_indices.iter().any(|entry| entry.5 == *monitor_id));

    let mut rendered = Vec::with_capacity(mon_indices.len());

    for (mon_idx, origin_x, origin_y, width, height, monitor_id) in mon_indices {
        let key = monitor_render_key(core, mon_idx, monitor_id, wayland_systray);
        if let Some(cached) = painter.cached_monitors.get(&monitor_id)
            && cached.key == key
        {
            rendered.push((cached.buffer.buffer.clone(), cached.buffer.x, cached.buffer.y));
            continue;
        }

        painter.begin(scale, origin_x, origin_y, width, height);
        crate::bar::renderer::draw_bar(core, mon_idx, painter);
        if core.globals().cfg.show_systray
            && let Some(mon) = core.globals().monitor(mon_idx).cloned()
        {
            crate::systray::wayland::draw_wayland_systray(
                core,
                wayland_systray,
                wayland_systray_menu,
                &mon,
                painter,
            );
        }
        painter.finish();

        let mut fresh = painter.take_buffers();
        if let Some((buffer, x, y)) = fresh.pop() {
            painter.cached_monitors.insert(
                monitor_id,
                CachedMonitorBar {
                    key,
                    buffer: BarBuffer {
                        buffer: buffer.clone(),
                        x,
                        y,
                    },
                },
            );
            rendered.push((buffer, x, y));
        }
    }

    rendered
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

fn monitor_render_key(
    core: &crate::contexts::CoreCtx,
    mon_idx: usize,
    monitor_id: usize,
    wayland_systray: &crate::types::WaylandSystray,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    core.bar.update_seq().hash(&mut hasher);
    core.globals().cfg.show_bar.hash(&mut hasher);
    core.globals().cfg.bar_height.hash(&mut hasher);
    core.globals().cfg.horizontal_padding.hash(&mut hasher);
    core.globals().cfg.startmenusize.hash(&mut hasher);
    core.globals().drag.bar_active.hash(&mut hasher);
    core.globals().selected_monitor_id().hash(&mut hasher);

    let Some(m) = core.globals().monitor(mon_idx) else {
        return hasher.finish();
    };

    monitor_id.hash(&mut hasher);
    m.num.hash(&mut hasher);
    m.work_rect.x.hash(&mut hasher);
    m.work_rect.y.hash(&mut hasher);
    m.work_rect.w.hash(&mut hasher);
    m.work_rect.h.hash(&mut hasher);
    m.bar_y.hash(&mut hasher);
    m.showbar.hash(&mut hasher);
    m.current_tag.hash(&mut hasher);
    m.selected_tags().hash(&mut hasher);
    m.sel.hash(&mut hasher);
    hash_gesture(&mut hasher, m.gesture);
    if let Some(tag) = m.current_tag() {
        tag.showbar.hash(&mut hasher);
        tag.name.hash(&mut hasher);
        tag.alt_name.hash(&mut hasher);
        tag.layouts.symbol().hash(&mut hasher);
    }

    let selected = m.selected_tags();
    for (win, c) in m.iter_clients(core.globals().clients.map()) {
        if !c.is_visible(selected) {
            continue;
        }
        win.hash(&mut hasher);
        c.name.hash(&mut hasher);
        c.tags.hash(&mut hasher);
        c.is_urgent.hash(&mut hasher);
        c.is_locked.hash(&mut hasher);
        c.is_fullscreen.hash(&mut hasher);
        c.is_hidden.hash(&mut hasher);
    }

    if core.globals().selected_monitor_id() == monitor_id {
        core.globals().bar_runtime.status_text.hash(&mut hasher);
    }

    if core.globals().cfg.show_systray && core.globals().selected_monitor_id() == monitor_id {
        for item in &wayland_systray.items {
            item.service.hash(&mut hasher);
            item.path.hash(&mut hasher);
            item.icon_w.hash(&mut hasher);
            item.icon_h.hash(&mut hasher);
            item.icon_rgba.hash(&mut hasher);
        }
    }

    hasher.finish()
}
