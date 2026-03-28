#![allow(clippy::too_many_arguments)]
//! Wayland bar rendering using cosmic-text for text and MemoryRenderBuffer for output.
//!
//! The bar is rasterized into a single RGBA pixel buffer per monitor,
//! then uploaded as a Smithay MemoryRenderBuffer for compositing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Scale, Transform};

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::bar::paint::{BarPainter, BarScheme};
use crate::bar::scene;
use crate::contexts::CoreCtx;

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
                            if gx + col >= x + w || gy + row >= y + h {
                                continue;
                            }
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
                            if gx + col >= x + w || gy + row >= y + h {
                                continue;
                            }
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
    async_runtime: Option<AsyncBarRenderRuntime>,
}

pub struct BarBuffer {
    pub buffer: MemoryRenderBuffer,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone)]
struct RawBarBuffer {
    pixels: Vec<u8>,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
}

#[derive(Clone)]
struct AsyncBarRenderRequest {
    key: u64,
    font_size: f32,
    monitors: Vec<scene::MonitorBarSnapshot>,
}

struct AsyncBarRenderResult {
    key: u64,
    buffers: Vec<RawBarBuffer>,
    monitor_updates: Vec<scene::MonitorRenderOutputWithId>,
}

struct AsyncBarRenderShared {
    pending: Mutex<Option<AsyncBarRenderRequest>>,
    wake: Condvar,
    results_tx: Sender<AsyncBarRenderResult>,
    render_ping: Mutex<Option<smithay::reexports::calloop::ping::Ping>>,
}

struct AsyncBarRenderRuntime {
    shared: Arc<AsyncBarRenderShared>,
    results_rx: Receiver<AsyncBarRenderResult>,
    pending_key: u64,
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
        let (results_tx, results_rx) = mpsc::channel();
        let shared = Arc::new(AsyncBarRenderShared {
            pending: Mutex::new(None),
            wake: Condvar::new(),
            results_tx,
            render_ping: Mutex::new(None),
        });

        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("instantwm-wayland-bar".to_string())
            .spawn(move || {
                let mut painter = WaylandBarPainter::new_worker_painter();
                loop {
                    let request = {
                        let mut guard = worker_shared.pending.lock().unwrap();
                        loop {
                            if let Some(request) = guard.take() {
                                break request;
                            }
                            guard = worker_shared.wake.wait(guard).unwrap();
                        }
                    };

                    let result = render_async_snapshot(&mut painter, request);
                    let _ = worker_shared.results_tx.send(result);
                    if let Ok(guard) = worker_shared.render_ping.lock()
                        && let Some(ping) = guard.as_ref()
                    {
                        ping.ping();
                    }
                }
            })
            .expect("failed to spawn Wayland bar worker");

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
            async_runtime: Some(AsyncBarRenderRuntime {
                shared,
                results_rx,
                pending_key: 0,
            }),
        }
    }
}

impl WaylandBarPainter {
    fn new_worker_painter() -> Self {
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
            async_runtime: None,
        }
    }

    pub fn set_font_size(&mut self, font_size: f32) {
        if font_size.is_finite() && font_size > 0.0 {
            self.font_size = font_size;
            self.text_width_cache.borrow_mut().clear();
        }
    }

    pub fn set_render_ping(
        &mut self,
        render_ping: Option<smithay::reexports::calloop::ping::Ping>,
    ) {
        let Some(runtime) = self.async_runtime.as_mut() else {
            return;
        };
        if let Ok(mut guard) = runtime.shared.render_ping.lock() {
            *guard = render_ping;
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

    fn finish_raw(&mut self) -> Option<RawBarBuffer> {
        if self.canvas_w <= 0 || self.canvas_h <= 0 {
            return None;
        }

        Some(RawBarBuffer {
            pixels: std::mem::take(&mut self.pixels),
            width: self.canvas_w,
            height: self.canvas_h,
            x: self.origin_x,
            y: self.origin_y,
        })
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

fn draw_wayland_systray_snapshot(
    painter: &mut WaylandBarPainter,
    snapshot: &scene::SystraySnapshot,
    layout: &scene::WorkerTrayLayout,
    bar_height: i32,
) {
    painter.set_scheme(snapshot.base_scheme.clone());
    if layout.tray_total_w > 0 {
        painter.rect(
            layout.tray_start_x,
            0,
            layout.tray_total_w,
            bar_height,
            true,
            true,
        );
    }
    if layout.menu_total_w > 0 {
        painter.rect(
            layout.menu_start_x,
            0,
            layout.menu_total_w,
            bar_height,
            true,
            true,
        );
    }

    let icon_h = bar_height.max(1);
    for slot in &layout.tray_slots {
        let Some(item) = snapshot.items.items.get(slot.idx) else {
            continue;
        };
        painter.blit_rgba_bgra(
            slot.start,
            0,
            slot.end - slot.start,
            icon_h,
            item.icon_w,
            item.icon_h,
            &item.icon_rgba,
        );
    }

    if let Some(menu) = &snapshot.menu {
        let mut scheme = snapshot.base_scheme.clone();
        painter.set_scheme(scheme.clone());
        for (row, item) in menu.items.iter().enumerate() {
            let Some(slot) = layout.menu_slots.get(row) else {
                continue;
            };
            let x = slot.start;
            let w = slot.end - slot.start;
            if item.separator {
                painter.rect(x + 3, bar_height / 2, w - 6, 1, true, false);
                continue;
            }
            if !item.enabled {
                scheme.fg[3] = 0.6;
                painter.set_scheme(scheme.clone());
            }
            painter.text(x, 0, w, bar_height, 8, &item.label, false, 0);
            if !item.enabled {
                scheme.fg[3] = 1.0;
                painter.set_scheme(scheme.clone());
            }
        }
    }
}

fn raw_to_bar_buffer(raw: &RawBarBuffer) -> BarBuffer {
    let buffer = MemoryRenderBuffer::from_slice(
        &raw.pixels,
        Fourcc::Argb8888,
        (raw.width, raw.height),
        1,
        Transform::Normal,
        None,
    );
    BarBuffer {
        buffer,
        x: raw.x,
        y: raw.y,
    }
}

fn render_async_snapshot(
    painter: &mut WaylandBarPainter,
    request: AsyncBarRenderRequest,
) -> AsyncBarRenderResult {
    let mut buffers = Vec::new();
    let mut monitor_updates = Vec::new();
    painter.set_font_size(request.font_size);

    for mon in request.monitors {
        painter.begin(
            Scale::from(1.0),
            mon.origin_x,
            mon.origin_y,
            mon.width,
            mon.height,
        );
        let output = scene::render_monitor_snapshot(&mon, painter);
        let bar_height = mon.height;
        let tray_layout = mon
            .systray
            .as_ref()
            .map(|s| scene::worker_systray_layout(s, mon.width, bar_height.max(1)));
        if let (Some(systray), Some(layout)) = (&mon.systray, &tray_layout) {
            draw_wayland_systray_snapshot(painter, systray, layout, bar_height);
        }

        if let Some(raw) = painter.finish_raw() {
            buffers.push(raw);
        }
        monitor_updates.push(scene::MonitorRenderOutputWithId {
            monitor_id: mon.monitor_id,
            output,
        });
    }

    AsyncBarRenderResult {
        key: request.key,
        buffers,
        monitor_updates,
    }
}

fn request_async_render(
    painter: &mut WaylandBarPainter,
    key: u64,
    core: &mut CoreCtx,
    wayland_systray: &crate::types::WaylandSystray,
    wayland_systray_menu: Option<&crate::types::WaylandSystrayMenu>,
) {
    let Some(runtime) = painter.async_runtime.as_mut() else {
        return;
    };
    if runtime.pending_key == key {
        return;
    }
    let monitors =
        scene::build_monitor_snapshots(core, Some((wayland_systray, wayland_systray_menu)));

    let mut pending = runtime.shared.pending.lock().unwrap();
    *pending = Some(AsyncBarRenderRequest {
        key,
        font_size: painter.font_size,
        monitors,
    });
    runtime.pending_key = key;
    runtime.shared.wake.notify_one();
}

fn poll_async_render_result(core: &mut CoreCtx, painter: &mut WaylandBarPainter) {
    let Some(runtime) = painter.async_runtime.as_mut() else {
        return;
    };

    let mut latest = None;
    loop {
        match runtime.results_rx.try_recv() {
            Ok(result) => {
                if result.key < runtime.pending_key {
                    continue;
                }
                latest = Some(result);
            }
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }

    let Some(result) = latest else {
        return;
    };

    painter.cached_buffers = result.buffers.iter().map(raw_to_bar_buffer).collect();
    painter.cached_key = result.key;
    painter.has_cached_buffers = true;

    for update in result.monitor_updates {
        core.bar
            .replace_hit_cache(update.monitor_id, update.output.hit_cache);
        if let Some(mon) = core.globals_mut().monitor_mut(update.monitor_id) {
            mon.bar_clients_width = update.output.bar_clients_width;
            mon.activeoffset = update.output.activeoffset;
        }
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
    let _ = scale;

    let key = bar_render_key(core, wayland_systray);
    poll_async_render_result(core, painter);

    if painter.cached_key != key {
        request_async_render(painter, key, core, wayland_systray, wayland_systray_menu);
    }

    if painter.cached_key == key {
        core.bar.mark_drawn();
    }

    painter
        .cached_buffers
        .iter()
        .map(|b| (b.buffer.clone(), b.x, b.y))
        .collect()
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
fn bar_render_key(
    core: &crate::contexts::CoreCtx,
    wayland_systray: &crate::types::WaylandSystray,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    core.bar.update_seq().hash(&mut hasher);
    core.globals().cfg.show_bar.hash(&mut hasher);
    core.globals().cfg.bar_height.hash(&mut hasher);
    core.globals().cfg.horizontal_padding.hash(&mut hasher);
    core.globals().cfg.startmenusize.hash(&mut hasher);
    core.globals().drag.bar_active.hash(&mut hasher);
    core.globals().bar_runtime.status_text.hash(&mut hasher);
    core.globals().selected_monitor_id().hash(&mut hasher);

    for m in core.globals().monitors_iter_all() {
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
    }

    if core.globals().cfg.show_systray {
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
