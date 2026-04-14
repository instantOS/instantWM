//! DRM/KMS rendering and GPU output management.

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::{FrameError, FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
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
use smithay::backend::renderer::{Bind, BufferType, Offscreen, Renderer, buffer_type};
use smithay::desktop::utils::{
    OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, take_presentation_feedback_surface_tree,
};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::drm::control::{self, connector, crtc};
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
        let Some(spec) = drm_output_spec(output_manager, &res, conn_handle, &used_crtcs) else {
            continue;
        };

        used_crtcs.push(spec.crtc);
        let entry = initialize_drm_output_surface(
            output_manager,
            renderer,
            state,
            &init_render_elements,
            spec,
            output_x_offset,
        );
        output_x_offset += entry.width;
        output_surfaces.push(entry);
    }

    output_surfaces
}

struct DrmOutputSpec {
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: control::Mode,
    width: i32,
    height: i32,
    physical_size: (i32, i32),
    name: String,
}

fn drm_output_spec(
    output_manager: &ManagedDrmOutputManager,
    resources: &control::ResourceHandles,
    connector: connector::Handle,
    used_crtcs: &[crtc::Handle],
) -> Option<DrmOutputSpec> {
    let conn_info = output_manager
        .device()
        .get_connector(connector, false)
        .ok()?;
    if !is_usable_connector(&conn_info) {
        return None;
    }

    let mode = best_connector_mode(conn_info.modes())?;
    let crtc = unused_connector_crtc(output_manager, resources, &conn_info, used_crtcs)?;
    let (width, height) = mode.size();
    let physical_size = conn_info.size().unwrap_or((0, 0));

    Some(DrmOutputSpec {
        connector,
        crtc,
        mode,
        width: width as i32,
        height: height as i32,
        physical_size: (physical_size.0 as i32, physical_size.1 as i32),
        name: format!(
            "{}-{}",
            connector_type_name(conn_info.interface()),
            conn_info.interface_id()
        ),
    })
}

fn is_usable_connector(conn_info: &connector::Info) -> bool {
    matches!(
        conn_info.state(),
        connector::State::Connected | connector::State::Unknown
    ) && !conn_info.modes().is_empty()
}

fn best_connector_mode(modes: &[control::Mode]) -> Option<control::Mode> {
    modes.iter().copied().max_by(|a, b| {
        let (aw, ah) = a.size();
        let (bw, bh) = b.size();
        (aw as u64 * ah as u64)
            .cmp(&(bw as u64 * bh as u64))
            .then_with(|| a.vrefresh().cmp(&b.vrefresh()))
    })
}

fn unused_connector_crtc(
    output_manager: &ManagedDrmOutputManager,
    resources: &control::ResourceHandles,
    conn_info: &connector::Info,
    used_crtcs: &[crtc::Handle],
) -> Option<crtc::Handle> {
    conn_info
        .encoders()
        .iter()
        .filter_map(|&enc_h| output_manager.device().get_encoder(enc_h).ok())
        .flat_map(|enc| resources.filter_crtcs(enc.possible_crtcs()))
        .find(|crtc| !used_crtcs.contains(crtc))
}

fn initialize_drm_output_surface(
    output_manager: &mut ManagedDrmOutputManager,
    renderer: &mut GlesRenderer,
    state: &mut WaylandState,
    init_render_elements: &DrmOutputRenderElements<GlesRenderer, DrmExtras>,
    spec: DrmOutputSpec,
    x_offset: i32,
) -> OutputSurfaceEntry {
    log::info!(
        "Output {}: {}x{}@{}Hz on CRTC {:?}",
        spec.name,
        spec.width,
        spec.height,
        spec.mode.vrefresh(),
        spec.crtc
    );

    let output = create_drm_wayland_output(state, &spec, x_offset);
    let surface = output_manager
        .lock()
        .initialize_output(
            spec.crtc,
            spec.mode,
            &[spec.connector],
            &output,
            None,
            renderer,
            init_render_elements,
        )
        .expect("initialize_output");
    let (vrr_support, configured_vrr_mode) =
        configure_drm_output_vrr(state, &spec.name, spec.connector, &surface);

    OutputSurfaceEntry {
        crtc: spec.crtc,
        connector: spec.connector,
        surface,
        output: output.clone(),
        x_offset,
        width: spec.width,
        height: spec.height,
        vrr_support,
        configured_vrr_mode,
        vrr_enabled: false,
    }
}

fn create_drm_wayland_output(state: &WaylandState, spec: &DrmOutputSpec, x_offset: i32) -> Output {
    let out_mode = OutputMode {
        size: (spec.width, spec.height).into(),
        refresh: (spec.mode.vrefresh() as i32) * 1000,
    };
    state.create_output_global(
        spec.name.clone(),
        PhysicalProperties {
            size: spec.physical_size.into(),
            subpixel: Subpixel::Unknown,
            make: "instantOS".into(),
            model: "instantWM".into(),
            serial_number: "Unknown".into(),
        },
        out_mode,
        (x_offset, 0),
    )
}

fn configure_drm_output_vrr(
    state: &mut WaylandState,
    output_name: &str,
    connector: connector::Handle,
    surface: &state::ManagedDrmOutput,
) -> (BackendVrrSupport, VrrMode) {
    let vrr_support = drm_surface_vrr_support(surface, connector);
    state.set_output_vrr_support(output_name, vrr_support);
    let configured_vrr_mode = state
        .output_vrr_metadata(output_name)
        .map(|m| m.vrr_mode)
        .unwrap_or(VrrMode::Auto);
    state.set_output_vrr_mode(output_name, configured_vrr_mode);
    state.set_output_vrr_enabled(output_name, false);
    log::info!("Output {output_name}: VRR support = {:?}", vrr_support);
    (vrr_support, configured_vrr_mode)
}

fn drm_surface_vrr_support(
    surface: &state::ManagedDrmOutput,
    connector: connector::Handle,
) -> BackendVrrSupport {
    match surface.with_compositor(|compositor| compositor.vrr_supported(connector)) {
        Ok(VrrSupport::Supported) => BackendVrrSupport::Supported,
        Ok(VrrSupport::RequiresModeset) => BackendVrrSupport::RequiresModeset,
        Ok(VrrSupport::NotSupported) | Err(_) => BackendVrrSupport::Unsupported,
    }
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
    let exporter = GbmFramebufferExporter::new(gbm_device.clone(), NodeFilter::None);
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
    let mut cursorless_image_captures =
        crate::backend::wayland::compositor::image_capture::drain_pending_image_captures(
            &mut state.runtime.pending_image_captures,
            &entry.output,
            false,
        );
    let mut cursor_image_captures =
        crate::backend::wayland::compositor::image_capture::drain_pending_image_captures(
            &mut state.runtime.pending_image_captures,
            &entry.output,
            true,
        );
    let has_pending_image_capture =
        !cursorless_image_captures.is_empty() || !cursor_image_captures.is_empty();
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

    if has_pending_screencopy || has_pending_image_capture {
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

        let mut cursorless_dmabuf_captures = Vec::new();
        let mut cursorless_target_captures = Vec::new();
        for capture in cursorless_image_captures.drain(..) {
            if matches!(buffer_type(&capture.frame.buffer()), Some(BufferType::Dma)) {
                cursorless_dmabuf_captures.push(capture);
            } else {
                cursorless_target_captures.push(capture);
            }
        }
        cursorless_image_captures = cursorless_target_captures;

        let mut cursor_dmabuf_captures = Vec::new();
        let mut cursor_target_captures = Vec::new();
        for capture in cursor_image_captures.drain(..) {
            if matches!(buffer_type(&capture.frame.buffer()), Some(BufferType::Dma)) {
                cursor_dmabuf_captures.push(capture);
            } else {
                cursor_target_captures.push(capture);
            }
        }
        cursor_image_captures = cursor_target_captures;

        for capture in cursorless_dmabuf_captures {
            let buffer = capture.frame.buffer();
            let mut dmabuf = match smithay::wayland::dmabuf::get_dmabuf(&buffer) {
                Ok(dmabuf) => dmabuf.clone(),
                Err(err) => {
                    log::warn!("image-capture: failed to access dmabuf: {:?}", err);
                    capture
                        .frame
                        .fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
                    continue;
                }
            };
            let mut target = match renderer.bind(&mut dmabuf) {
                Ok(target) => target,
                Err(err) => {
                    log::warn!("image-capture: failed to bind dmabuf: {:?}", err);
                    capture
                        .frame
                        .fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
                    continue;
                }
            };
            match frame_result.blit_frame_result(
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
                    capture.frame.success(
                        capture.transform,
                        None::<Vec<Rectangle<i32, BufferCoords>>>,
                        crate::backend::wayland::compositor::image_capture::monotonic_timestamp(),
                    );
                }
                Err(err) => {
                    log::warn!("image-capture direct dmabuf blit failed: {:?}", err);
                    capture
                        .frame
                        .fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
                }
            }
        }

        for capture in cursor_dmabuf_captures {
            let buffer = capture.frame.buffer();
            let mut dmabuf = match smithay::wayland::dmabuf::get_dmabuf(&buffer) {
                Ok(dmabuf) => dmabuf.clone(),
                Err(err) => {
                    log::warn!("image-capture: failed to access dmabuf: {:?}", err);
                    capture
                        .frame
                        .fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
                    continue;
                }
            };
            let mut target = match renderer.bind(&mut dmabuf) {
                Ok(target) => target,
                Err(err) => {
                    log::warn!("image-capture: failed to bind dmabuf: {:?}", err);
                    capture
                        .frame
                        .fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
                    continue;
                }
            };
            match frame_result.blit_frame_result(
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
                    capture.frame.success(
                        capture.transform,
                        None::<Vec<Rectangle<i32, BufferCoords>>>,
                        crate::backend::wayland::compositor::image_capture::monotonic_timestamp(),
                    );
                }
                Err(err) => {
                    log::warn!("image-capture direct dmabuf blit failed: {:?}", err);
                    capture
                        .frame
                        .fail(smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown);
                }
            }
        }

        if has_cursorless_screencopy || !cursorless_image_captures.is_empty() {
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
                        crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
                            &mut state.runtime.pending_screencopies,
                            renderer,
                            &target,
                            &entry.output,
                            false,
                        );
                        crate::backend::wayland::compositor::image_capture::submit_image_captures(
                            cursorless_image_captures,
                            renderer,
                            &target,
                        );
                        let _ = sync;
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

        if has_cursor_screencopy || !cursor_image_captures.is_empty() {
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
                        crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
                            &mut state.runtime.pending_screencopies,
                            renderer,
                            &target,
                            &entry.output,
                            true,
                        );
                        crate::backend::wayland::compositor::image_capture::submit_image_captures(
                            cursor_image_captures,
                            renderer,
                            &target,
                        );
                        let _ = sync;
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
