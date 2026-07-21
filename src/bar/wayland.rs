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
use crate::types::{Point, Rect, Size};

use self::buffer::{BarBuffer, RawBarBuffer};
use self::text::TextRasterizer;

pub struct WaylandBarPainter {
    text: TextRasterizer,
    scheme: Option<BarScheme>,
    pixels: Vec<u8>,
    surface_rect: Rect,
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
            surface_rect: Rect::default(),
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
            surface_rect: Rect::default(),
            buffers: Vec::new(),
            cached_buffers: Vec::new(),
            cached_key: 0,
            async_runtime: None,
        }
    }

    pub fn set_font_size(&mut self, font_size: f32) {
        self.text.set_font_size(font_size);
    }

    pub fn set_font_families(&mut self, families: &[String]) {
        self.text.set_font_families(families);
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

    pub fn begin(&mut self, _scale: Scale<f64>, surface_rect: Rect) {
        self.scheme = None;
        self.surface_rect = surface_rect;
        let byte_len = if surface_rect.size().is_positive() {
            (surface_rect.w as usize)
                .checked_mul(surface_rect.h as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                .unwrap_or(0)
        } else {
            0
        };
        self.pixels.clear();
        self.pixels.resize(byte_len, 0);
    }

    pub fn finish(&mut self) {
        if !self.surface_rect.size().is_positive() {
            return;
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &self.pixels,
            Fourcc::Argb8888,
            (self.surface_rect.w, self.surface_rect.h),
            1,
            Transform::Normal,
            None,
        );
        self.buffers.push(BarBuffer {
            buffer,
            position: self.surface_rect.position(),
        });
    }

    fn finish_raw(&mut self) -> Option<RawBarBuffer> {
        if !self.surface_rect.size().is_positive() {
            return None;
        }

        Some(RawBarBuffer {
            pixels: std::mem::take(&mut self.pixels),
            rect: self.surface_rect,
        })
    }

    pub fn take_buffers(&mut self) -> Vec<(MemoryRenderBuffer, Point)> {
        self.buffers
            .drain(..)
            .map(|buffer| (buffer.buffer, buffer.position))
            .collect()
    }

    pub fn blit_rgba_bgra(&mut self, destination: Rect, source_size: Size, src_rgba: &[u8]) {
        pixels::blit_rgba_scaled(
            &mut self.pixels,
            self.surface_rect.size(),
            destination,
            source_size,
            src_rgba,
        );
    }
}

impl BarPainter for WaylandBarPainter {
    fn text_width(&mut self, text: &str) -> i32 {
        self.text.width(text, self.surface_rect.h)
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        self.scheme = Some(scheme);
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
            self.surface_rect.size(),
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
        pixels::fill_rect(&mut self.pixels, self.surface_rect.size(), bounds, bg);
        if detail_height > 0 {
            pixels::fill_rect(
                &mut self.pixels,
                self.surface_rect.size(),
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
                    self.surface_rect.size(),
                    Rect::new(text_x, bounds.y, text_w, bounds.h),
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
    status_notifier_tray: &crate::systray::StatusNotifierTray,
    tray_menu: Option<&crate::systray::TrayMenuPresentation>,
) -> Vec<(MemoryRenderBuffer, Point)> {
    let snapshots =
        scene::build_monitor_snapshots(core, Some(status_notifier_tray), tray_menu, false);
    // Cache the systray width so status bar layout can account for it.
    core.bar.runtime.systray_width = if core.config().systray.show {
        crate::systray::layout(
            status_notifier_tray,
            None,
            core.model().expect_selected_monitor().work_rect().w,
            core.model().expect_selected_monitor().bar_height,
            core.config().systray.spacing,
        )
        .total_width
    } else {
        0
    };
    let _ = scale;

    let key = hash::render_key(
        core.config().bar.show,
        core.config().systray.show,
        &snapshots,
    );
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
        .map(|buffer| (buffer.buffer.clone(), buffer.position))
        .collect()
}
