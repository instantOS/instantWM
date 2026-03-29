//! DRM/KMS rendering and GPU output management.

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::{FrameError, FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::DrmOutputRenderElements;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, VrrSupport};
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Element, Id, RenderElementStates};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, Offscreen, Renderer};
use smithay::desktop::utils::{
    OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, take_presentation_feedback_surface_tree,
};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::drm::control::connector;
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle};

use crate::backend::BackendVrrSupport;
use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::VrrMode;
use crate::wayland::common::{
    CursorPresentation, FixedSceneElements, build_common_scene_elements_from_fixed,
    count_upper_layer_render_elements, get_render_element_counts, resolve_cursor_presentation,
    send_frame_callbacks, update_primary_scanout_output,
};

mod cursor;

// Re-export cursor management
pub use cursor::CursorManager;
pub use state::{
    DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH, ManagedDrmOutputManager, OutputHitRegion,
    OutputSurfaceEntry,
};

pub mod state;

#[derive(Debug)]
pub struct DrmFrameMetadata {
    pub presentation_feedback: OutputPresentationFeedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    Submitted,
    Skipped,
    Failed,
}

render_elements! {
    pub DrmExtras<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Cursor=TextureRenderElement<GlesTexture>,
    Space=smithay::desktop::space::SpaceRenderElements<GlesRenderer, smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>>,
}

pub fn build_output_surfaces(
    output_manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    state: &mut WaylandState,
) -> Vec<OutputSurfaceEntry> {
    let mut output_surfaces: Vec<OutputSurfaceEntry> = Vec::new();
    let mut output_x_offset: i32 = 0;

    let res = output_manager
        .device()
        .resource_handles()
        .expect("drm resource_handles");
    let mut used_crtcs: Vec<crtc::Handle> = Vec::new();
    let init_render_elements = DrmOutputRenderElements::<GlesRenderer, DrmExtras>::default();

    for &conn_handle in res.connectors() {
        let Ok(conn_info) = output_manager.device().get_connector(conn_handle, false) else {
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
            .filter_map(|&enc_h| output_manager.device().get_encoder(enc_h).ok())
            .flat_map(|enc| res.filter_crtcs(enc.possible_crtcs()))
            .collect();

        let Some(&picked_crtc) = encoder_crtcs.iter().find(|c| !used_crtcs.contains(c)) else {
            continue;
        };
        used_crtcs.push(picked_crtc);

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
            output_name.clone(),
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

        let surface = output_manager
            .initialize_output(
                picked_crtc,
                mode,
                &[conn_handle],
                &output,
                None,
                renderer,
                &init_render_elements,
            )
            .expect("initialize_output");
        let vrr_support =
            match surface.with_compositor(|compositor| compositor.vrr_supported(conn_handle)) {
                Ok(VrrSupport::Supported) => BackendVrrSupport::Supported,
                Ok(VrrSupport::RequiresModeset) => BackendVrrSupport::RequiresModeset,
                Ok(VrrSupport::NotSupported) | Err(_) => BackendVrrSupport::Unsupported,
            };
        state.set_output_vrr_support(&output_name, vrr_support);
        let configured_vrr_mode = state
            .output_vrr_metadata(&output_name)
            .map(|m| m.vrr_mode)
            .unwrap_or(VrrMode::Auto);
        state.set_output_vrr_mode(&output_name, configured_vrr_mode);
        state.set_output_vrr_enabled(&output_name, false);
        log::info!("Output {output_name}: VRR support = {:?}", vrr_support);

        output_surfaces.push(OutputSurfaceEntry {
            crtc: picked_crtc,
            connector: conn_handle,
            surface,
            output: output.clone(),
            x_offset: output_x_offset,
            width: mode_w,
            height: mode_h,
            vrr_support,
            configured_vrr_mode,
            vrr_enabled: false,
        });
        output_x_offset += mode_w;
    }

    output_surfaces
}

pub fn create_output_manager(
    drm_device: DrmDevice,
    renderer: &GlesRenderer,
    gbm_device: &GbmDevice<DrmDeviceFd>,
) -> ManagedDrmOutputManager {
    let allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm_device.clone(), None);
    let color_formats: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];
    let renderer_formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();

    ManagedDrmOutputManager::new(
        drm_device,
        allocator,
        exporter,
        Some(gbm_device.clone()),
        color_formats,
        renderer_formats,
    )
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
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    cursor_manager: &CursorManager,
    pointer_location: Point<f64, smithay::utils::Logical>,
    start_time: std::time::Instant,
    fixed_scene: Option<FixedSceneElements>,
) -> RenderOutcome {
    let local_pointer = Point::from((
        pointer_location.x - entry.x_offset as f64,
        pointer_location.y,
    ));
    let cursor_presentation = resolve_cursor_presentation(
        &state.cursor_image_status,
        state.cursor_icon_override,
        state.runtime.dnd_icon.as_ref(),
    );

    let cursor_scale = entry.output.current_scale().integer_scale();
    let millis = start_time.elapsed().as_millis() as u32;
    let cursor_elements: Vec<DrmExtras> = build_cursor_elements(
        renderer,
        cursor_manager,
        &cursor_presentation,
        local_pointer,
        cursor_scale,
        millis,
    );
    let cursor_element_ids: Vec<Id> = cursor_elements
        .iter()
        .map(|element| element.id().clone())
        .collect();

    let mut render_elements: Vec<DrmExtras>;

    if state.is_locked() {
        // When locked, only render the lock surface (and cursor) for this output.
        render_elements = Vec::with_capacity(cursor_elements.len() + 4);
        for elem in cursor_elements {
            render_elements.push(elem);
        }
        let output_name = entry.output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            let lock_elements: Vec<
                smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<
                    GlesRenderer,
                >,
            > = smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                renderer,
                lock_surface.wl_surface(),
                smithay::utils::Point::<i32, Physical>::from((0, 0)),
                smithay::utils::Scale::from(1.0),
                1.0,
                smithay::backend::renderer::element::Kind::Unspecified,
            );
            for elem in lock_elements {
                render_elements.push(DrmExtras::Surface(elem));
            }
        }
    } else {
        let scene = build_common_scene_elements_from_fixed(
            state,
            renderer,
            entry.x_offset,
            fixed_scene.expect("fixed scene elements"),
        );
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
        render_elements = Vec::with_capacity(counts.total() + cursor_elements.len());

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
    }

    let has_pending_screencopy = state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output == entry.output);
    let has_cursor_screencopy = state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output == entry.output && copy.overlay_cursor);
    let has_cursorless_screencopy = state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output == entry.output && !copy.overlay_cursor);
    let mut frame_flags = FrameFlags::DEFAULT;
    if entry.vrr_enabled {
        frame_flags |= FrameFlags::SKIP_CURSOR_ONLY_UPDATES;
    }

    let frame_result = match entry.surface.render_frame(
        renderer,
        &render_elements,
        [0.05, 0.05, 0.07, 1.0],
        frame_flags,
    ) {
        Ok(result) => result,
        Err(err) => {
            log::warn!("render_frame: {:?}", err);
            return RenderOutcome::Failed;
        }
    };

    if has_pending_screencopy {
        let output_scale = entry.output.current_scale().fractional_scale();
        let output_transform = entry.output.current_transform().invert();
        let mode_size = entry
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_else(|| (entry.width, entry.height).into());
        let target_size = output_transform.transform_size(mode_size);
        let target_size_buffer: smithay::utils::Size<i32, BufferCoords> =
            (target_size.w, target_size.h).into();
        if has_cursorless_screencopy {
            let mut capture: GlesTexture =
                match renderer.create_buffer(Fourcc::Xrgb8888, target_size_buffer) {
                    Ok(buffer) => buffer,
                    Err(err) => {
                        log::warn!("screencopy offscreen buffer creation failed: {:?}", err);
                        return RenderOutcome::Failed;
                    }
                };
            match renderer.bind(&mut capture) {
                Ok(mut target) => match frame_result.blit_frame_result(
                    target_size,
                    output_transform,
                    output_scale,
                    renderer,
                    &mut target,
                    [Rectangle::from_size(target_size)],
                    cursor_element_ids.iter().cloned(),
                ) {
                    Ok(sync) => {
                        let _ = renderer.wait(&sync);
                        crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
                            &mut state.runtime.pending_screencopies,
                            renderer,
                            &target,
                            &entry.output,
                            false,
                        );
                    }
                    Err(err) => {
                        log::warn!("screencopy blit_frame_result failed: {:?}", err);
                        return RenderOutcome::Failed;
                    }
                },
                Err(err) => {
                    log::warn!("screencopy offscreen bind failed: {:?}", err);
                    return RenderOutcome::Failed;
                }
            }
        }

        if has_cursor_screencopy {
            let mut capture: GlesTexture =
                match renderer.create_buffer(Fourcc::Xrgb8888, target_size_buffer) {
                    Ok(buffer) => buffer,
                    Err(err) => {
                        log::warn!("screencopy offscreen buffer creation failed: {:?}", err);
                        return RenderOutcome::Failed;
                    }
                };
            match renderer.bind(&mut capture) {
                Ok(mut target) => match frame_result.blit_frame_result(
                    target_size,
                    output_transform,
                    output_scale,
                    renderer,
                    &mut target,
                    [Rectangle::from_size(target_size)],
                    std::iter::empty::<Id>(),
                ) {
                    Ok(sync) => {
                        let _ = renderer.wait(&sync);
                        crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
                            &mut state.runtime.pending_screencopies,
                            renderer,
                            &target,
                            &entry.output,
                            true,
                        );
                    }
                    Err(err) => {
                        log::warn!("screencopy blit_frame_result failed: {:?}", err);
                        return RenderOutcome::Failed;
                    }
                },
                Err(err) => {
                    log::warn!("screencopy offscreen bind failed: {:?}", err);
                    return RenderOutcome::Failed;
                }
            }
        }
    }

    if frame_result.needs_sync()
        && let PrimaryPlaneElement::Swapchain(primary_swapchain) = &frame_result.primary_element
    {
        let _ = primary_swapchain.sync.wait();
    }

    update_primary_scanout_output(state, &entry.output, &frame_result.states);

    let frame_metadata = DrmFrameMetadata {
        presentation_feedback: collect_presentation_feedback(state, entry, &frame_result.states),
    };

    match entry.surface.queue_frame(frame_metadata) {
        Ok(()) => {}
        Err(FrameError::EmptyFrame) => {
            // Even when KMS can skip submitting an unchanged frame, visible
            // Wayland clients still need their frame callbacks to keep driving
            // content updates. Without this, callback-paced clients such as
            // Firefox can appear frozen until unrelated input dirties the
            // output and forces a real page flip.
            send_frame_callbacks(state, &entry.output, start_time.elapsed());
            return RenderOutcome::Skipped;
        }
        Err(err) => {
            log::warn!("queue_frame: {:?}", err);
            return RenderOutcome::Failed;
        }
    }

    send_frame_callbacks(state, &entry.output, start_time.elapsed());
    RenderOutcome::Submitted
}

fn collect_presentation_feedback(
    state: &WaylandState,
    entry: &OutputSurfaceEntry,
    render_states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut output_feedback = OutputPresentationFeedback::new(&entry.output);
    let output_geo = state.space.output_geometry(&entry.output);
    let surface_flags =
        |surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
         _: &smithay::wayland::compositor::SurfaceData| {
            surface_presentation_feedback_flags_from_states(surface, render_states)
        };

    if state.is_locked() {
        let output_name = entry.output.name();
        if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
            take_presentation_feedback_surface_tree(
                lock_surface.wl_surface(),
                &mut output_feedback,
                surface_primary_scanout_output,
                surface_flags,
            );
        }
        return output_feedback;
    }

    for window in state.space.elements() {
        if let Some(out_geo) = output_geo
            && let Some(win_loc) = state.space.element_location(window)
        {
            let win_rect = Rectangle::new(win_loc, window.geometry().size);
            if !out_geo.overlaps(win_rect) {
                continue;
            }
        }

        window.take_presentation_feedback(
            &mut output_feedback,
            surface_primary_scanout_output,
            surface_flags,
        );
    }

    let layer_map = smithay::desktop::layer_map_for_output(&entry.output);
    for layer_surface in layer_map.layers() {
        layer_surface.take_presentation_feedback(
            &mut output_feedback,
            surface_primary_scanout_output,
            surface_flags,
        );
    }

    output_feedback
}

fn build_cursor_elements(
    renderer: &mut GlesRenderer,
    cursor_manager: &CursorManager,
    cursor_presentation: &CursorPresentation,
    local_pointer: Point<f64, smithay::utils::Logical>,
    scale: i32,
    millis: u32,
) -> Vec<DrmExtras> {
    let mut custom_elements = Vec::new();

    match cursor_presentation {
        CursorPresentation::Hidden => {}
        CursorPresentation::Named(_) => {
            if let Some(cursor_elem) = cursor_manager.render_element(
                local_pointer,
                cursor_presentation,
                scale,
                millis,
                renderer,
            ) {
                custom_elements.push(DrmExtras::Cursor(cursor_elem));
            }
        }
        CursorPresentation::Surface { surface, hotspot } => {
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
                    smithay::utils::Scale::from(scale as f64),
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
            custom_elements.extend(build_cursor_elements(
                renderer,
                cursor_manager,
                cursor,
                local_pointer,
                scale,
                millis,
            ));

            if !smithay::utils::IsAlive::alive(icon) {
                return custom_elements;
            }

            let dnd_loc = smithay::utils::Point::<i32, smithay::utils::Physical>::from((
                (local_pointer.x - hotspot.x as f64).round() as i32,
                (local_pointer.y - hotspot.y as f64).round() as i32,
            ));
            let dnd_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    icon,
                    dnd_loc,
                    smithay::utils::Scale::from(scale as f64),
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
