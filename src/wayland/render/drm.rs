//! DRM/KMS rendering and GPU output management.

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, GbmBufferedSurface};
use smithay::backend::renderer::Bind;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::drm::control::connector;
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Physical, Point, Rectangle};

use crate::backend::wayland::compositor::WaylandState;
use crate::wayland::common::{
    CursorPresentation, build_common_scene_elements, count_upper_layer_render_elements,
    get_render_element_counts, resolve_cursor_presentation, send_frame_callbacks,
};
use crate::wm::Wm;

mod cursor;

// Re-export cursor management
pub use cursor::CursorManager;
pub use state::{
    DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH, OutputHitRegion, OutputSurfaceEntry,
    SharedDrmState, sync_monitors_from_outputs_vec,
};

pub mod state;

render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Cursor=TextureRenderElement<GlesTexture>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>>,
}

pub fn build_output_surfaces(
    drm_device: &mut DrmDevice,
    renderer: &mut GlesRenderer,
    state: &WaylandState,
    gbm_device: &GbmDevice<DrmDeviceFd>,
) -> Vec<OutputSurfaceEntry> {
    let gbm_allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let color_formats: &[Fourcc] = &[Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;

    let res = drm_device.resource_handles().expect("drm resource_handles");
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();

    for &conn_handle in res.connectors() {
        let Ok(conn_info) = drm_device.get_connector(conn_handle, false) else {
            continue;
        };
        if conn_info.state() != connector::State::Connected
            && conn_info.state() != connector::State::Unknown
        {
            continue;
        }
        let modes = conn_info.modes();
        if modes.is_empty() {
            continue;
        }

        let mut sorted_modes = modes.to_vec();
        sorted_modes.sort_by(|a, b| {
            let (aw, ah) = a.size();
            let (bw, bh) = b.size();
            (bw as u64 * bh as u64)
                .cmp(&(aw as u64 * ah as u64))
                .then_with(|| b.vrefresh().cmp(&a.vrefresh()))
        });
        let mode = sorted_modes[0];

        let encoder_crtcs: Vec<crtc::Handle> = conn_info
            .encoders()
            .iter()
            .filter_map(|&enc_h| drm_device.get_encoder(enc_h).ok())
            .flat_map(|enc| res.filter_crtcs(enc.possible_crtcs()))
            .collect();

        let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) else {
            continue;
        };
        used_crtcs.push(picked_crtc);

        let drm_surface = drm_device
            .create_surface(picked_crtc, mode, &[conn_handle])
            .expect("create_surface");
        let gbm_surface = GbmBufferedSurface::new(
            drm_surface,
            gbm_allocator.clone(),
            color_formats,
            renderer_formats.iter().cloned(),
        )
        .expect("GbmBufferedSurface::new");

        let (mode_w, mode_h) = mode.size();
        let (mode_w, mode_h) = (mode_w as i32, mode_h as i32);
        let output_name = format!(
            "{}-{}",
            connector_type_name(conn_info.interface()),
            conn_info.interface_id()
        );
        log::info!(
            "Output {output_name}: {mode_w}x{mode_h}@{}Hz on CRTC {:?}",
            mode.vrefresh(),
            picked_crtc
        );

        let output = Output::new(
            output_name,
            PhysicalProperties {
                size: {
                    let (mm_w, mm_h) = conn_info.size().unwrap_or((0, 0));
                    (mm_w as i32, mm_h as i32).into()
                },
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
            },
        );
        let out_mode = OutputMode {
            size: (mode_w, mode_h).into(),
            refresh: (mode.vrefresh() as i32) * 1000,
        };
        output.change_current_state(
            Some(out_mode),
            Some(smithay::utils::Transform::Normal),
            Some(Scale::Integer(1)),
            Some((output_x_offset, 0).into()),
        );
        output.set_preferred(out_mode);
        let _global = output.create_global::<WaylandState>(&state.display_handle);

        let damage_tracker = OutputDamageTracker::from_output(&output);

        output_surfaces.push(OutputSurfaceEntry {
            crtc: picked_crtc,
            surface: gbm_surface,
            output: output.clone(),
            damage_tracker,
            x_offset: output_x_offset,
            width: mode_w,
            height: mode_h,
        });
        output_x_offset += mode_w;
    }

    output_surfaces
}

fn connector_type_name(interface: connector::Interface) -> &'static str {
    match interface {
        connector::Interface::DVII => "DVI-I",
        connector::Interface::DVID => "DVI-D",
        connector::Interface::DVIA => "DVI-A",
        connector::Interface::SVideo => "S-Video",
        connector::Interface::DisplayPort => "DP",
        connector::Interface::HDMIA => "HDMI-A",
        connector::Interface::HDMIB => "HDMI-B",
        connector::Interface::EmbeddedDisplayPort => "eDP",
        connector::Interface::VGA => "VGA",
        connector::Interface::LVDS => "LVDS",
        connector::Interface::DSI => "DSI",
        connector::Interface::DPI => "DPI",
        connector::Interface::Composite => "Composite",
        _ => "Unknown",
    }
}

pub fn render_drm_output(
    wm: &mut Wm,
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    cursor_manager: &CursorManager,
    pointer_location: Point<f64, smithay::utils::Logical>,
    start_time: std::time::Instant,
) -> bool {
    let (dmabuf, age) = match entry.surface.next_buffer() {
        Ok(buf) => buf,
        Err(e) => {
            log::trace!("next_buffer: {e}");
            return false;
        }
    };

    let mut dmabuf_clone = dmabuf.clone();
    let Ok(mut target) = renderer.bind(&mut dmabuf_clone) else {
        log::warn!("renderer bind failed");
        return false;
    };

    let local_pointer = Point::from((
        pointer_location.x - entry.x_offset as f64,
        pointer_location.y,
    ));
    let cursor_presentation = resolve_cursor_presentation(
        &state.cursor_image_status,
        state.cursor_icon_override,
        state.dnd_icon.as_ref(),
    );

    let cursor_elements: Vec<DrmExtras> = build_cursor_elements(
        renderer,
        cursor_manager,
        &cursor_presentation,
        local_pointer,
    );

    let scene = build_common_scene_elements(wm, state, renderer, entry.x_offset);
    let space_render_elements = smithay::desktop::space::space_render_elements(
        renderer,
        [&state.space],
        &entry.output,
        1.0,
    )
    .expect("space render elements");

    // Shared: count upper layer elements
    let num_upper = count_upper_layer_render_elements(renderer, &entry.output);

    // Shared: get element counts for pre-allocation (include cursor elements)
    let counts = get_render_element_counts(&scene, space_render_elements.len(), num_upper);
    let mut render_elements = Vec::with_capacity(counts.total() + cursor_elements.len());

    // Backend-specific: cursor elements come first in DRM (winit handles cursor differently)
    for elem in cursor_elements {
        render_elements.push(elem);
    }

    // Shared: assemble remaining elements in z-order
    super::assemble_scene_elements!(
        DrmExtras,
        scene,
        space_render_elements,
        num_upper,
        render_elements
    );

    let render_result = entry.damage_tracker.render_output(
        renderer,
        &mut target,
        age as usize,
        &render_elements,
        [0.05, 0.05, 0.07, 1.0],
    );

    crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
        &mut state.pending_screencopies,
        renderer,
        &target,
        &entry.output,
        start_time,
    );
    drop(target);

    match render_result {
        Ok(result) => {
            let damage: Option<Vec<Rectangle<i32, Physical>>> = result.damage.cloned();
            if let Err(e) = entry.surface.queue_buffer(None, damage, ()) {
                log::warn!("queue_buffer: {e}");
                return false;
            }
        }
        Err(e) => {
            log::warn!("render_output: {:?}", e);
            return false;
        }
    }

    send_frame_callbacks(state, &entry.output, start_time.elapsed());
    true
}

fn build_cursor_elements(
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    cursor_presentation: &CursorPresentation,
    local_pointer: Point<f64, smithay::utils::Logical>,
) -> Vec<DrmExtras> {
    let mut custom_elements = Vec::new();

    match cursor_presentation {
        CursorPresentation::Hidden => {}
        CursorPresentation::Named(_) => {
            if let Some(cursor_elem) =
                cursor_manager.render_element(local_pointer, cursor_presentation)
            {
                custom_elements.push(DrmExtras::Cursor(cursor_elem));
            }
        }
        CursorPresentation::Surface { surface, hotspot } => {
            // Double-check that the surface is still alive before rendering.
            if !smithay::utils::IsAlive::alive(surface) {
                return custom_elements;
            }
            let cursor_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (local_pointer.x - hotspot.x as f64).round() as i32,
                (local_pointer.y - hotspot.y as f64).round() as i32,
            ));
            let cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    surface,
                    cursor_loc,
                    smithay::utils::Scale::from(1.0),
                    1.0,
                    smithay::backend::renderer::element::Kind::Cursor,
                );
            for elem in cursor_elements {
                custom_elements.push(DrmExtras::Surface(elem));
            }
        }
        CursorPresentation::DndIcon {
            icon,
            hotspot,
            cursor,
        } => {
            // Render the base cursor first
            custom_elements.extend(build_cursor_elements(
                renderer,
                cursor_manager,
                cursor,
                local_pointer,
            ));

            // Double-check that the drag icon surface is still alive before rendering.
            if !smithay::utils::IsAlive::alive(icon) {
                return custom_elements;
            }

            // Then render the drag icon
            let dnd_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (local_pointer.x - hotspot.x as f64).round() as i32,
                (local_pointer.y - hotspot.y as f64).round() as i32,
            ));
            let dnd_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    icon,
                    dnd_loc,
                    smithay::utils::Scale::from(1.0),
                    1.0,
                    smithay::backend::renderer::element::Kind::Cursor,
                );
            for elem in dnd_elements {
                custom_elements.push(DrmExtras::Surface(elem));
            }
        }
    }

    custom_elements
}
