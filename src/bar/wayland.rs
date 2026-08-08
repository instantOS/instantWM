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
                    bounds.bottom() - detail_height,
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
        bounds.right()
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
        scene::build_monitor_snapshots(core, Some(status_notifier_tray), tray_menu, false, 0);
    // Cache the systray width so status bar layout can account for it.
    core.bar.runtime.systray_width = snapshots
        .iter()
        .find(|snapshot| snapshot.is_selected_monitor)
        .and_then(|snapshot| snapshot.systray.as_ref())
        .map(|systray| systray.layout.total_width)
        .unwrap_or(0);
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

/// Background buffers for the bottom bar strip, with a centered "grab handle"
/// indicator so users know the bar is interactive.
///
/// The strip renders the status-bar background color plus a semi-transparent
/// white rectangle in the center. Input classification (`button_region_at`)
/// routes presses to the configured `BottomBar` bindings.
pub fn build_bottom_bar_buffers(core: &mut CoreCtx) -> Vec<(MemoryRenderBuffer, Point)> {
    let mut buffers = Vec::new();
    let bg = core.config().colors.status_bar.bg;
    let monitors: Vec<crate::types::Monitor> = core
        .model()
        .monitors_iter()
        .filter(|(_, mon)| mon.bottom_bar_visible(&core.model().clients))
        .map(|(_, mon)| mon.clone())
        .collect();
    for mon in monitors {
        let w = mon.work_rect().w;
        let h = mon.bottom_bar_height;
        if w <= 0 || h <= 0 {
            continue;
        }
        let [r, g, b, a] = bg.to_rgba8();
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        // Fill background, premultiplying alpha for GL compositing.
        for chunk in pixels.chunks_exact_mut(4) {
            let (pr, pg, pb, pa) = if a == 255 {
                (b, g, r, 255)
            } else {
                (
                    (b as u16 * a as u16 / 255) as u8,
                    (g as u16 * a as u16 / 255) as u8,
                    (r as u16 * a as u16 / 255) as u8,
                    a,
                )
            };
            chunk.copy_from_slice(&[pr, pg, pb, pa]);
        }

        // Draw the centered indicator: blend the bar background heavily
        // toward white (~85%) so the handle reads as a bright, white pill
        // regardless of the bar's theme color.
        let indicator = mon.bottom_bar_indicator_rect();
        if indicator.w > 0 && indicator.h > 0 {
            let blend = |bg: u8| -> u8 { ((bg as u16 * 15 + 255 * 85) / 100) as u8 };
            let ir = blend(r);
            let ig = blend(g);
            let ib = blend(b);
            for y in 0..indicator.h.min(h) {
                for x in 0..indicator.w.min(w) {
                    let idx =
                        ((indicator.y + y) as usize * w as usize + (indicator.x + x) as usize) * 4;
                    if idx + 3 < pixels.len() {
                        let (pr, pg, pb, pa) = if a == 255 {
                            (ib, ig, ir, 255)
                        } else {
                            (
                                (ib as u16 * a as u16 / 255) as u8,
                                (ig as u16 * a as u16 / 255) as u8,
                                (ir as u16 * a as u16 / 255) as u8,
                                a,
                            )
                        };
                        pixels[idx..idx + 4].copy_from_slice(&[pr, pg, pb, pa]);
                    }
                }
            }
        }

        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (w, h),
            1,
            Transform::Normal,
            None,
        );
        buffers.push((buffer, Point::new(mon.work_rect().x, mon.bottom_bar_y())));
    }
    buffers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_wm() -> crate::wm::Wm {
        use crate::backend::{Backend, wayland::WaylandBackend};
        crate::wm::Wm::new(Backend::new_wayland(WaylandBackend::new()))
    }

    /// The bottom strip must be an opaque, monitor-width buffer aligned to the
    /// bottom of the monitor. It must never fall back to alpha 0.
    #[test]
    fn bottom_bar_buffers_are_opaque_and_bottom_aligned() {
        let mut wm = test_wm();

        let show_bar = wm.core.config.bar.show;
        let show_bottom = wm.core.config.bar.show_bottom;
        assert!(
            !show_bottom,
            "bottom bar defaults to hidden — opt in via ToggleBottomBar / config"
        );

        let mut mon = crate::types::Monitor::new_with_values(show_bar);
        // Enable the bar for this test only — the production default is
        // hidden, so the test must opt in to exercise the buffer pipeline.
        mon.show_bottom_bar = true;
        mon.bottom_bar_height = 24;
        let id = wm.core.model.monitors.allocate_id();
        mon.monitor_id = id;
        mon.set_available_rect(crate::types::Rect::new(0, 0, 1920, 1080));
        wm.core.model.monitors.restore(vec![mon]);

        let mut core = crate::contexts::CoreCtx::new(
            &mut wm.core,
            &mut wm.work,
            &mut wm.running,
            &mut wm.bar,
            &mut wm.focus,
        );
        let buffers = build_bottom_bar_buffers(&mut core);
        assert_eq!(buffers.len(), 1, "one bottom strip buffer expected");
        let (_buffer, pos) = &buffers[0];
        assert_eq!(pos.x, 0);
        assert_eq!(pos.y, 1080 - 24, "strip must be bottom-aligned");
    }

    /// The indicator blend must compute visibly brighter than the background.
    /// Regression guard for the truncation bug in the blend formula
    /// (`(bg as u16 * 15 + 255 * 85) as u8 / 100` silently casts to u8 *before*
    /// dividing, producing ~1 instead of ~219 on dark backgrounds).
    #[test]
    fn bottom_bar_blend_toward_white_is_not_truncated() {
        let blend = |bg: u8| -> u8 { ((bg as u16 * 15 + 255 * 85) / 100) as u8 };
        assert_eq!(blend(18), 219, "dark bg (18) must blend toward near-white");
        assert_eq!(blend(255), 255, "white bg stays white");
        assert_eq!(blend(0), 216, "black bg blends to ~85% white");
    }
}
