#![allow(clippy::too_many_arguments)]
//! Wayland bar rendering using cosmic-text and MemoryRenderBuffer output.
//!
//! The bar is rasterized into one ARGB8888 pixel buffer per monitor, then
//! uploaded as a Smithay MemoryRenderBuffer for compositing.

mod async_render;
mod buffer;
mod hash;
mod pixels;
mod systray;
mod text;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::{Scale, Transform};

use crate::bar::paint::{BarPainter, BarScheme};
use crate::bar::scene;
use crate::contexts::CoreCtx;
use crate::types::geometry::Rect;

use self::buffer::{BarBuffer, RawBarBuffer};
use self::text::TextRasterizer;

pub struct WaylandBarPainter {
    text: TextRasterizer,
    scheme: Option<BarScheme>,
    pixels: Vec<u8>,
    canvas_w: i32,
    canvas_h: i32,
    origin_x: i32,
    origin_y: i32,
    buffers: Vec<BarBuffer>,
    cached_buffers: Vec<BarBuffer>,
    cached_key: u64,
    async_runtime: Option<async_render::AsyncBarRenderRuntime>,
}

impl Default for WaylandBarPainter {
    fn default() -> Self {
        Self {
            text: TextRasterizer::default(),
            scheme: None,
            pixels: Vec::new(),
            canvas_w: 0,
            canvas_h: 0,
            origin_x: 0,
            origin_y: 0,
            buffers: Vec::new(),
            cached_buffers: Vec::new(),
            cached_key: 0,
            async_runtime: Some(async_render::AsyncBarRenderRuntime::spawn()),
        }
    }
}

impl WaylandBarPainter {
    fn new_worker_painter() -> Self {
        Self {
            text: TextRasterizer::default(),
            scheme: None,
            pixels: Vec::new(),
            canvas_w: 0,
            canvas_h: 0,
            origin_x: 0,
            origin_y: 0,
            buffers: Vec::new(),
            cached_buffers: Vec::new(),
            cached_key: 0,
            async_runtime: None,
        }
    }

    pub fn set_font_size(&mut self, font_size: f32) {
        self.text.set_font_size(font_size);
    }

    pub fn set_render_ping(
        &mut self,
        render_ping: Option<smithay::reexports::calloop::ping::Ping>,
    ) {
        let Some(runtime) = self.async_runtime.as_mut() else {
            return;
        };
        runtime.set_render_ping(render_ping);
    }

    /// Measure text width without requiring `&mut self`; used for hit-testing.
    pub fn measure_text_width(&self, text: &str) -> i32 {
        self.text.width(text, 0)
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
        pixels::blit_rgba_scaled(
            &mut self.pixels,
            self.canvas_w,
            self.canvas_h,
            Rect::new(dst_x, dst_y, dst_w, dst_h),
            src_w,
            src_h,
            src_rgba,
        );
    }
}

impl BarPainter for WaylandBarPainter {
    fn text_width(&mut self, text: &str) -> i32 {
        self.text.width(text, self.canvas_h)
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        self.scheme = Some(scheme);
    }

    fn scheme(&self) -> Option<&BarScheme> {
        self.scheme.as_ref()
    }

    fn rect(&mut self, bounds: Rect, filled: bool, invert: bool) {
        if !filled || bounds.w <= 0 || bounds.h <= 0 {
            return;
        }
        let Some(scheme) = self.scheme.clone() else {
            return;
        };
        pixels::fill_rect(
            &mut self.pixels,
            self.canvas_w,
            self.canvas_h,
            bounds,
            scheme.rect_color(invert),
        );
    }

    fn text(
        &mut self,
        bounds: Rect,
        lpad: i32,
        text: &str,
        invert: bool,
        detail_height: i32,
    ) -> i32 {
        let Some(scheme) = self.scheme.clone() else {
            return bounds.x;
        };
        let (bg, fg) = scheme.text_colors(invert);
        pixels::fill_rect(&mut self.pixels, self.canvas_w, self.canvas_h, bounds, bg);
        if detail_height > 0 {
            pixels::fill_rect(
                &mut self.pixels,
                self.canvas_w,
                self.canvas_h,
                Rect::new(
                    bounds.x,
                    bounds.y + bounds.h - detail_height,
                    bounds.w,
                    detail_height,
                ),
                scheme.detail,
            );
        }
        if !text.is_empty() {
            let powerline = TextRasterizer::is_powerline_text(text);
            let bleed = if powerline { 2 } else { 0 };
            let text_x = bounds.x + lpad - bleed;
            let text_w = (bounds.w - lpad + bleed * 2).max(0);
            if text_w > 0 {
                self.text.rasterize(
                    &mut self.pixels,
                    self.canvas_w,
                    self.canvas_h,
                    text_x,
                    bounds.y,
                    text_w,
                    bounds.h,
                    text,
                    fg,
                );
            }
        }
        bounds.x + bounds.w
    }
}

pub fn render_bar_buffers(
    core: &mut CoreCtx,
    painter: &mut WaylandBarPainter,
    scale: Scale<f64>,
    wayland_systray: &crate::types::WaylandSystray,
    wayland_systray_menu: Option<&crate::types::WaylandSystrayMenu>,
) -> Vec<(MemoryRenderBuffer, i32, i32)> {
    let snapshots =
        scene::build_monitor_snapshots(core, Some((wayland_systray, wayland_systray_menu)), false);
    // Cache the systray width so status bar layout can account for it.
    core.globals_mut().bar_runtime.systray_width =
        crate::systray::wayland::get_wayland_systray_width_with_state(
            core,
            wayland_systray,
            core.globals().selected_monitor().bar_height,
        );
    let _ = scale;

    let key = hash::render_key(core, &snapshots, wayland_systray_menu);
    async_render::poll_result(core, painter);

    if painter.cached_key != key {
        async_render::request_render(painter, key, snapshots);
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
