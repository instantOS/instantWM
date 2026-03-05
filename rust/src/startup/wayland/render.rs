use std::collections::HashMap;
use std::time::Duration;

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::WinitGraphicsBackend;
use smithay::desktop::space::render_output;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::desktop::PopupManager;
use smithay::output::Output;
use smithay::utils::Scale;
use smithay::wayland::seat::WaylandFocus;

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::bar::color::rgba_from_hex;
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
    let damage = {
        let buffer_age = backend.buffer_age().unwrap_or(0);
        let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");
        let mut custom_elements: Vec<WaylandExtras> = Vec::new();

        if wm.g.cfg.showbar {
            let mut ctx = wm.ctx();
            let bar_buffers = crate::bar::wayland::render_bar_buffers(
                &mut ctx.core,
                &mut wm.bar_painter,
                Scale::from(1.0),
            );
            for (buffer, x, y) in bar_buffers {
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    (x as f64, y as f64),
                    &buffer,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    Ok(elem) => custom_elements.push(WaylandExtras::Memory(elem)),
                    Err(e) => {
                        log::warn!("bar buffer upload failed: {:?}", e);
                    }
                }
            }
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

        render_result.damage.cloned()
    };
    let _ = backend.submit(damage.as_deref());

    let time = start_time.elapsed();
    for window in state.space.elements() {
        if let Some(surface) = window.wl_surface() {
            send_frames_surface_tree(
                &surface,
                output,
                time,
                Some(Duration::from_millis(16)),
                surface_primary_scanout_output,
            );
            if let Some(toplevel) = window.toplevel() {
                for (popup, _) in PopupManager::popups_for_surface(toplevel.wl_surface()) {
                    send_frames_surface_tree(
                        popup.wl_surface(),
                        output,
                        time,
                        Some(Duration::from_millis(16)),
                        surface_primary_scanout_output,
                    );
                }
            }
        }
    }
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
    let mut occluders: HashMap<WindowId, IntRect> = HashMap::new();
    for win in &z_order {
        let Some(c) = g.clients.get(win) else {
            continue;
        };
        let is_visible = c
            .monitor_id
            .and_then(|mid| g.monitor(mid))
            .map(|m| c.is_visible_on_tags(m.selected_tags()))
            .unwrap_or(false);
        if !is_visible || c.is_hidden {
            continue;
        }
        let bw = c.border_width.max(0);
        let (content_w, content_h) = mapped_sizes.get(win).copied().unwrap_or((c.geo.w, c.geo.h));
        if content_w <= 0 || content_h <= 0 {
            continue;
        }
        occluders.insert(
            *win,
            IntRect {
                x: c.geo.x,
                y: c.geo.y,
                w: content_w + 2 * bw,
                h: content_h + 2 * bw,
            },
        );
    }

    let sel = g.selected_win();
    for (idx, win) in z_order.iter().enumerate() {
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
        let mut border_parts = vec![
            IntRect { x, y, w: ow, h: bw },
            IntRect {
                x,
                y: y + oh - bw,
                w: ow,
                h: bw,
            },
            IntRect {
                x,
                y: y + bw,
                w: bw,
                h: (oh - 2 * bw).max(0),
            },
            IntRect {
                x: x + ow - bw,
                y: y + bw,
                w: bw,
                h: (oh - 2 * bw).max(0),
            },
        ];

        for higher in z_order.iter().skip(idx + 1) {
            let Some(occ) = occluders.get(higher).copied() else {
                continue;
            };
            border_parts = border_parts
                .into_iter()
                .flat_map(|part| subtract_rect(part, occ))
                .collect();
            if border_parts.is_empty() {
                break;
            }
        }

        for part in border_parts {
            push_solid(&mut out, part.x, part.y, part.w, part.h, rgba);
        }
    }
    out
}

#[derive(Clone, Copy)]
struct IntRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn intersect_rect(a: IntRect, b: IntRect) -> Option<IntRect> {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(IntRect {
        x: x1,
        y: y1,
        w: x2 - x1,
        h: y2 - y1,
    })
}

fn subtract_rect(base: IntRect, cut: IntRect) -> Vec<IntRect> {
    if base.w <= 0 || base.h <= 0 {
        return Vec::new();
    }
    let Some(i) = intersect_rect(base, cut) else {
        return vec![base];
    };

    let mut out = Vec::new();
    if i.y > base.y {
        out.push(IntRect {
            x: base.x,
            y: base.y,
            w: base.w,
            h: i.y - base.y,
        });
    }
    let base_bottom = base.y + base.h;
    let inter_bottom = i.y + i.h;
    if inter_bottom < base_bottom {
        out.push(IntRect {
            x: base.x,
            y: inter_bottom,
            w: base.w,
            h: base_bottom - inter_bottom,
        });
    }
    if i.x > base.x {
        out.push(IntRect {
            x: base.x,
            y: i.y,
            w: i.x - base.x,
            h: i.h,
        });
    }
    let base_right = base.x + base.w;
    let inter_right = i.x + i.w;
    if inter_right < base_right {
        out.push(IntRect {
            x: inter_right,
            y: i.y,
            w: base_right - inter_right,
            h: i.h,
        });
    }
    out.into_iter().filter(|r| r.w > 0 && r.h > 0).collect()
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
