use std::collections::HashMap;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::space::render_output;
use smithay::output::Output;
use smithay::utils::Scale;

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::bar::color::rgba_from_hex;
use crate::startup::common_wayland::{build_bar_elements, send_frame_callbacks};
use crate::types::WindowId;
use crate::wm::Wm;

render_elements! {
    pub WaylandExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

pub(super) fn render_frame(
    wm: &mut Wm,
    state: &mut WaylandState,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
    output: &Output,
    damage_tracker: &mut OutputDamageTracker,
    start_time: std::time::Instant,
) {
    apply_cursor_image_status(backend, state);
    state.tick_window_animations();
    let damage = {
        let buffer_age = backend.buffer_age().unwrap_or(0);
        let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");
        let mut custom_elements: Vec<WaylandExtras> = Vec::new();

        for elem in build_bar_elements(wm, renderer) {
            custom_elements.push(WaylandExtras::Memory(elem));
        }

        for elem in wayland_border_elements_shared(&wm.g, state) {
            custom_elements.push(WaylandExtras::Solid(elem));
        }

        let render_result = render_output(
            output,
            renderer,
            &mut framebuffer,
            1.0,
            buffer_age,
            [&state.space],
            &custom_elements,
            damage_tracker,
            [0.05, 0.05, 0.07, 1.0],
        )
        .expect("render output");

        // Fulfil pending screencopy requests while framebuffer is still bound.
        crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
            &mut state.pending_screencopies,
            renderer,
            &framebuffer,
            output,
            start_time,
        );

        render_result.damage.cloned()
    };
    let _ = backend.submit(damage.as_deref());

    send_frame_callbacks(state, output, start_time.elapsed());
}

fn apply_cursor_image_status(backend: &WinitGraphicsBackend<GlesRenderer>, state: &WaylandState) {
    if let Some(icon) = state.cursor_icon_override {
        backend.window().set_cursor_visible(true);
        backend.window().set_cursor(icon);
        return;
    }
    match &state.cursor_image_status {
        smithay::input::pointer::CursorImageStatus::Hidden => {
            backend.window().set_cursor_visible(false);
        }
        smithay::input::pointer::CursorImageStatus::Named(icon) => {
            backend.window().set_cursor_visible(true);
            backend.window().set_cursor(*icon);
        }
        smithay::input::pointer::CursorImageStatus::Surface(_) => {
            backend.window().set_cursor_visible(true);
        }
    }
}

pub(super) fn wayland_border_elements_shared(
    g: &crate::globals::Globals,
    state: &WaylandState,
) -> Vec<SolidColorRenderElement> {
    let scheme = g.cfg.borderscheme.as_ref();
    let bordercolors = &g.cfg.bordercolors;
    let mut out = Vec::new();
    let mut mapped_sizes: HashMap<WindowId, (i32, i32)> = HashMap::new();
    let mut z_order: Vec<WindowId> = Vec::new();
    for window in state.space.elements() {
        if let Some(win) = window.user_data().get::<WindowIdMarker>().map(|m| m.id) {
            let size = window.geometry().size;
            mapped_sizes.insert(win, (size.w.max(1), size.h.max(1)));
            z_order.push(win);
        }
    }

    let sel = g.selected_win();
    for win in &z_order {
        let Some(c) = g.clients.get(win) else {
            continue;
        };
        let bw = c.border_width.max(0);
        let (content_w, content_h) = mapped_sizes.get(win).copied().unwrap_or((c.geo.w, c.geo.h));
        if bw <= 0 || content_w <= 0 || content_h <= 0 {
            continue;
        }
        let is_visible = c
            .monitor_id
            .and_then(|mid| g.monitor(mid))
            .map(|m| c.is_visible_on_tags(m.selected_tags()))
            .unwrap_or(false);
        if !is_visible || c.is_hidden {
            continue;
        }
        let has_tiling = c
            .monitor_id
            .and_then(|mid| g.monitor(mid))
            .map(|m| m.is_tiling_layout())
            .unwrap_or(true);
        let rgba = if Some(*win) == sel {
            if c.isfloating || !has_tiling {
                rgba_from_hex(bordercolors.get(crate::config::SchemeBorder::FloatFocus))
                    .or_else(|| scheme.map(|s| color_to_rgba(&s.float_focus.bg)))
                    .unwrap_or([0.75, 0.40, 0.28, 1.0])
            } else {
                rgba_from_hex(bordercolors.get(crate::config::SchemeBorder::TileFocus))
                    .or_else(|| scheme.map(|s| color_to_rgba(&s.tile_focus.bg)))
                    .unwrap_or([0.28, 0.52, 0.77, 1.0])
            }
        } else {
            rgba_from_hex(bordercolors.get(crate::config::SchemeBorder::Normal))
                .or_else(|| scheme.map(|s| color_to_rgba(&s.normal.bg)))
                .unwrap_or([0.18, 0.18, 0.20, 1.0])
        };

        let x = c.geo.x;
        let y = c.geo.y;
        let ow = content_w + 2 * bw;
        let oh = content_h + 2 * bw;
        push_solid(&mut out, x, y, ow, bw, rgba);
        push_solid(&mut out, x, y + oh - bw, ow, bw, rgba);
        push_solid(&mut out, x, y + bw, bw, (oh - 2 * bw).max(0), rgba);
        push_solid(
            &mut out,
            x + ow - bw,
            y + bw,
            bw,
            (oh - 2 * bw).max(0),
            rgba,
        );
    }
    out
}

fn push_solid(
    out: &mut Vec<SolidColorRenderElement>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: [f32; 4],
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let buffer = SolidColorBuffer::new((w, h), color);
    out.push(SolidColorRenderElement::from_buffer(
        &buffer,
        (x, y),
        Scale::from(1.0),
        1.0,
        Kind::Unspecified,
    ));
}

fn color_to_rgba(color: &crate::drw::Color) -> [f32; 4] {
    [
        color.color.color.red as f32 / 65535.0,
        color.color.color.green as f32 / 65535.0,
        color.color.color.blue as f32 / 65535.0,
        color.color.color.alpha as f32 / 65535.0,
    ]
}
