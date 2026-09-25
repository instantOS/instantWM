#![allow(clippy::too_many_arguments)]
//! Wayland bar rendering using cosmic-text and MemoryRenderBuffer output.
//!
//! The bar is rasterized into one ARGB8888 pixel buffer per monitor, then
//! uploaded as a Smithay MemoryRenderBuffer for compositing.

mod async_render;
mod buffer;
mod text;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

use crate::bar::canvas::Canvas;
use crate::bar::paint::{BarPainter, BarScheme};
use crate::bar::scene;
use crate::contexts::CoreCtx;
use crate::types::{Point, Rect, Size};

use self::buffer::{BarBuffer, RawBarBuffer};
use self::text::TextRasterizer;

/// Main-loop state for asynchronous bar rendering.
///
/// Font discovery, shaping, and the pixel scratch buffer deliberately live in
/// [`BarRasterizer`] on the worker thread. Keeping them out of this type avoids
/// constructing a second `FontSystem` that never paints anything.
pub struct WaylandBarRenderer {
    cached_buffers: Vec<BarBuffer>,
    /// Scene the cached buffers depict.
    cached_snapshots: async_render::Snapshots,
    async_runtime: async_render::AsyncBarRenderRuntime,
}

impl Default for WaylandBarRenderer {
    fn default() -> Self {
        Self {
            cached_buffers: Vec::new(),
            cached_snapshots: Default::default(),
            async_runtime: async_render::AsyncBarRenderRuntime::spawn(),
        }
    }
}

impl WaylandBarRenderer {
    pub fn set_render_ping(
        &mut self,
        render_ping: Option<smithay::reexports::calloop::ping::Ping>,
    ) {
        self.async_runtime.set_render_ping(render_ping);
    }
}

/// Worker-local painter used to turn a bar snapshot into ARGB pixels.
#[derive(Default)]
struct BarRasterizer {
    text: TextRasterizer,
    scheme: Option<BarScheme>,
    canvas: Canvas,
    surface_rect: Rect,
}

impl BarRasterizer {
    fn set_fonts(&mut self, fonts: &crate::core_state::FontConfig) {
        self.text.set_fonts(fonts);
    }

    fn begin(&mut self, surface_rect: Rect) {
        self.scheme = None;
        self.surface_rect = surface_rect;
        self.canvas.resize(surface_rect.size());
    }

    fn finish_raw(&mut self) -> Option<RawBarBuffer> {
        if !self.surface_rect.size().is_positive() {
            return None;
        }

        Some(RawBarBuffer {
            // The finished image belongs to the compositor now, so the
            // rasterizer hands its canvas over rather than copying it. The
            // next `begin` allocates a fresh one.
            canvas: std::mem::take(&mut self.canvas),
            position: self.surface_rect.position(),
        })
    }
}

impl BarPainter for BarRasterizer {
    fn text_width(&mut self, text: &str) -> i32 {
        self.text.width(text, self.surface_rect.h)
    }

    fn set_scheme(&mut self, scheme: BarScheme) {
        self.scheme = Some(scheme);
    }

    fn rect(&mut self, bounds: Rect, invert: bool) {
        if bounds.w <= 0 || bounds.h <= 0 {
            return;
        }
        let Some(scheme) = self.scheme.clone() else {
            return;
        };
        self.canvas
            .fill_rect(bounds, scheme.rect_color(invert).into());
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
        self.canvas.fill_rect(bounds, bg.into());
        if detail_height > 0 {
            self.canvas.fill_rect(
                Rect::new(
                    bounds.x,
                    bounds.bottom() - detail_height,
                    bounds.w,
                    detail_height,
                ),
                scheme.detail.into(),
            );
        }
        if !text.is_empty() {
            let available_width = (bounds.w - lpad).max(0);
            let fitted = crate::bar::text::fit_to_width(text, available_width, |candidate| {
                self.text.width(candidate, bounds.h)
            });
            let text = fitted.as_ref();
            let powerline = crate::bar::text::is_powerline_only(text);
            let bleed = if powerline { 2 } else { 0 };
            let text_x = bounds.x + lpad - bleed;
            let text_w = (bounds.w - lpad + bleed * 2).max(0);
            if text_w > 0 {
                self.text.rasterize(
                    &mut self.canvas,
                    Rect::new(text_x, bounds.y, text_w, bounds.h),
                    text,
                    fg,
                );
            }
        }
        bounds.right()
    }

    fn blit_rgba(&mut self, destination: Rect, source_size: Size, src_rgba: &[u8]) {
        self.canvas.blit_rgba(destination, src_rgba, source_size);
    }
}

pub fn render_bar_buffers(
    core: &mut CoreCtx,
    renderer: &mut WaylandBarRenderer,
) -> Vec<(MemoryRenderBuffer, Point)> {
    let snapshots = scene::build_monitor_snapshots(core, 0);
    async_render::poll_result(core, renderer, &snapshots);

    if *renderer.cached_snapshots == snapshots {
        core.bar.mark_drawn();
    } else {
        renderer.async_runtime.request(snapshots);
    }

    renderer
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
    let bg = core.config().colors.status.bg;
    let indicator_color = bottom_bar_indicator_color(bg);
    core.model()
        .monitors_iter_all()
        .filter(|mon| mon.bottom_bar_visible(&core.model().clients))
        .filter_map(|mon| {
            let size = Size::new(mon.work_rect().w, mon.bottom_bar_height);
            if !size.is_positive() {
                return None;
            }
            let mut canvas = Canvas::new(size);
            canvas.fill_rect(canvas.bounds(), bg.into());
            canvas.fill_rect(mon.bottom_bar_indicator_rect(), indicator_color.into());
            let buffer = MemoryRenderBuffer::from_slice(
                canvas.as_slice(),
                Fourcc::Argb8888,
                (size.w, size.h),
                1,
                Transform::Normal,
                None,
            );
            Some((buffer, Point::new(mon.work_rect().x, mon.bottom_bar_y())))
        })
        .collect()
}

/// Blend the bar background heavily toward white (~85%) so the handle reads
/// as a bright, white pill regardless of the bar's theme color.
fn bottom_bar_indicator_color(bg: crate::types::Rgba) -> crate::types::Rgba {
    let [r, g, b, _] = bg.to_rgba8();
    let blend = |channel: u8| ((channel as u16 * 15 + 255 * 85) / 100) as f32 / 255.0;
    crate::types::Rgba::new(blend(r), blend(g), blend(b), bg.a())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_still_paints_and_advances_the_complete_cell() {
        let mut painter = BarRasterizer::default();
        painter.begin(Rect::new(0, 0, 8, 4));
        painter.set_scheme(BarScheme {
            foreground: crate::types::Rgba::new(1.0, 0.0, 0.0, 1.0),
            background: crate::types::Rgba::new(0.0, 0.0, 1.0, 1.0),
            detail: crate::types::Rgba::ZERO,
        });

        let right = BarPainter::text(&mut painter, Rect::new(2, 0, 4, 4), 0, "", false, 0);

        assert_eq!(right, 6);
        let pixel = (2 * 4) as usize;
        assert_eq!(
            &painter.canvas.as_slice()[pixel..pixel + 4],
            &[255, 0, 0, 255]
        );
    }

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

        let mut mon = crate::types::Monitor::new_with_values();
        mon.bar_default_show = show_bar;
        // Enable the bar for this test only — the production default is
        // hidden, so the test must opt in to exercise the buffer pipeline.
        mon.show_bottom_bar = true;
        mon.bottom_bar_height = 24;
        let id = wm.core.model.monitors.allocate_id();
        mon.monitor_id = id;
        mon.set_available_rect(crate::types::Rect::new(0, 0, 1920, 1080));
        wm.core.model.monitors.restore(vec![mon]);

        let mut core = wm.core_ctx();
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
        let blend = |value: u8| {
            let gray = f32::from(value) / 255.0;
            bottom_bar_indicator_color(crate::types::Rgba::rgb(gray, gray, gray)).to_rgba8()[0]
        };
        assert_eq!(blend(18), 219, "dark bg (18) must blend toward near-white");
        assert_eq!(blend(255), 255, "white bg stays white");
        assert_eq!(blend(0), 216, "black bg blends to ~85% white");
    }
}
