//! DRM/KMS rendering and GPU output management.

use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Buffer as AllocatorBuffer, Fourcc};
use smithay::backend::drm::Framebuffer;
use smithay::backend::drm::compositor::{
    FrameError, FrameFlags, PrimaryPlaneElement, RenderFrameResult,
};
use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
use smithay::backend::drm::output::DrmOutputRenderElements;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, VrrSupport};
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::render_elements;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::utils::select_dmabuf_feedback;
use smithay::backend::renderer::element::{Element, Id, RenderElement, RenderElementStates};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, Blit, BufferType, Offscreen, Renderer, buffer_type};
use smithay::desktop::utils::{
    OutputPresentationFeedback, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, take_presentation_feedback_surface_tree,
};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::drm::control::{self, connector, crtc};
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1;
use std::fmt;
use std::time::Instant;

use smithay::utils::{Buffer as BufferCoords, Physical, Point, Rectangle};
use smithay::wayland::dmabuf::DmabufFeedbackBuilder;

use crate::backend::BackendVrrSupport;
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::image_capture::PendingImageCapture;
use crate::backend::wayland::render::cursor::{ResolvedCursor, resolve_cursor};
use crate::backend::wayland::render::frame::{
    send_frame_callbacks, update_primary_scanout_output, window_overlaps_output,
};
use crate::backend::wayland::render::scene::{
    SharedSceneElements, build_common_scene_elements_from_shared,
    count_upper_layer_render_elements, get_render_element_counts,
    remove_duplicate_overlay_elements,
};
use crate::config::config_toml::VrrMode;
use std::rc::Rc;

mod capture;
mod cursor;
mod output_setup;
use capture::{submit_drm_capture_requests, take_drm_capture_requests};
pub(crate) use output_setup::build_output_dmabuf_feedback;
pub use output_setup::{
    add_new_output_surfaces, build_output_surfaces, create_output_manager, usable_connector_handles,
};

// Re-export cursor management
pub use cursor::CursorManager;
pub use state::{
    DEFAULT_SCREEN_HEIGHT, DEFAULT_SCREEN_WIDTH, ManagedDrmOutput, ManagedDrmOutputManager,
    OutputDmabufFeedback, OutputSurfaceEntry,
};

pub mod state;

#[derive(Debug)]
pub struct DrmFrameMetadata {
    pub presentation_feedback: OutputPresentationFeedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    Submitted,
    EmptyFrame,
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

pub fn render_drm_output(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &mut OutputSurfaceEntry,
    cursor_manager: &CursorManager,
    start_time: Instant,
    shared_scene: Option<Rc<SharedSceneElements>>,
    suppress_upper_layers: bool,
) -> RenderOutcome {
    // Read live rather than taking a parameter: the DRM loop snapshots the
    // same `state.runtime.pointer_location` before rendering and nothing in
    // between mutates it.
    let pointer_location = state.runtime.pointer_location;
    let cursor_elements = build_drm_cursor_elements(
        state,
        renderer,
        entry,
        cursor_manager,
        pointer_location,
        start_time,
    );
    let cursor_element_ids: Vec<Id> = cursor_elements
        .iter()
        .map(|element| element.id().clone())
        .collect();
    let render_elements = build_drm_render_elements(
        state,
        renderer,
        entry,
        cursor_elements,
        shared_scene,
        suppress_upper_layers,
    );
    let capture_requests = take_drm_capture_requests(state, &entry.output);

    let frame_flags = drm_frame_flags(entry);
    let frame_result = match entry
        .surface
        .as_mut()
        .expect("enabled DRM output has a surface")
        .render_frame(
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

    if capture_requests.has_pending()
        && !submit_drm_capture_requests(
            state,
            renderer,
            entry,
            &frame_result,
            &cursor_element_ids,
            capture_requests,
        )
    {
        return RenderOutcome::Failed;
    }

    if frame_result.needs_sync()
        && let PrimaryPlaneElement::Swapchain(primary_swapchain) = &frame_result.primary_element
    {
        let _ = primary_swapchain.sync.wait();
    }

    update_primary_scanout_output(state, &entry.output, &frame_result.states);
    send_output_dmabuf_feedback(state, entry, &frame_result.states);

    let frame_metadata = DrmFrameMetadata {
        presentation_feedback: collect_presentation_feedback(state, entry, &frame_result.states),
    };

    match entry
        .surface
        .as_mut()
        .expect("enabled DRM output has a surface")
        .queue_frame(frame_metadata)
    {
        Ok(()) => {}
        Err(FrameError::EmptyFrame) => {
            return RenderOutcome::EmptyFrame;
        }
        Err(err) => {
            log::warn!("queue_frame: {:?}", err);
            return RenderOutcome::Failed;
        }
    }

    crate::backend::wayland::render::frame::release_fifo_barriers(state, &entry.output);
    send_frame_callbacks(state, &entry.output, start_time.elapsed());
    RenderOutcome::Submitted
}

fn send_output_dmabuf_feedback(
    state: &WaylandState,
    entry: &OutputSurfaceEntry,
    render_states: &RenderElementStates,
) {
    let Some(feedback) = entry.dmabuf_feedback.as_ref() else {
        return;
    };
    if !state.is_locked() {
        for window in state
            .space
            .elements()
            .filter(|window| window_overlaps_output(state, window, &entry.output))
        {
            window.send_dmabuf_feedback(
                &entry.output,
                surface_primary_scanout_output,
                |surface, _| {
                    select_dmabuf_feedback(
                        surface,
                        render_states,
                        &feedback.render,
                        &feedback.scanout,
                    )
                },
            );
        }
    }
    let map = smithay::desktop::layer_map_for_output(&entry.output);
    for layer in map.layers() {
        layer.send_dmabuf_feedback(
            &entry.output,
            surface_primary_scanout_output,
            |surface, _| {
                select_dmabuf_feedback(surface, render_states, &feedback.render, &feedback.scanout)
            },
        );
    }
}

fn build_drm_cursor_elements(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    entry: &OutputSurfaceEntry,
    cursor_manager: &CursorManager,
    pointer_location: Point<f64, smithay::utils::Logical>,
    start_time: Instant,
) -> Vec<DrmExtras> {
    let local_pointer = Point::from((
        pointer_location.x - entry.rect.x as f64,
        pointer_location.y - entry.rect.y as f64,
    ));
    let resolved_cursor = resolve_cursor(
        &state.cursor_image_status,
        state.cursor_icon_override,
        state.runtime.dnd_icon.as_ref(),
        state.runtime.cursor_hidden_by_touch,
    );
    let cursor_scale = entry.output.current_scale().integer_scale();
    let millis = start_time.elapsed().as_millis() as u32;

    build_cursor_elements(
        renderer,
        cursor_manager,
        &resolved_cursor,
        local_pointer,
        cursor_scale,
        millis,
    )
}

fn build_drm_render_elements(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    entry: &OutputSurfaceEntry,
    cursor_elements: Vec<DrmExtras>,
    shared_scene: Option<Rc<SharedSceneElements>>,
    suppress_upper_layers: bool,
) -> Vec<DrmExtras> {
    if state.is_locked() {
        build_locked_drm_render_elements(state, renderer, entry, cursor_elements)
    } else {
        build_unlocked_drm_render_elements(
            state,
            renderer,
            entry,
            cursor_elements,
            shared_scene,
            suppress_upper_layers,
        )
    }
}

fn build_locked_drm_render_elements(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    entry: &OutputSurfaceEntry,
    cursor_elements: Vec<DrmExtras>,
) -> Vec<DrmExtras> {
    let mut render_elements = Vec::with_capacity(cursor_elements.len() + 4);
    render_elements.extend(cursor_elements);

    let output_name = entry.output.name();
    if let Some(lock_surface) = state.lock_surfaces.get(&output_name) {
        let lock_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                renderer,
                lock_surface.wl_surface(),
                smithay::utils::Point::<i32, Physical>::from((0, 0)),
                smithay::utils::Scale::from(1.0),
                1.0,
                smithay::backend::renderer::element::Kind::Unspecified,
            );
        render_elements.extend(lock_elements.into_iter().map(DrmExtras::Surface));
    }

    render_elements
}

fn build_unlocked_drm_render_elements(
    state: &WaylandState,
    renderer: &mut GlesRenderer,
    entry: &OutputSurfaceEntry,
    cursor_elements: Vec<DrmExtras>,
    shared_scene: Option<Rc<SharedSceneElements>>,
    suppress_upper_layers: bool,
) -> Vec<DrmExtras> {
    let scene = build_common_scene_elements_from_shared(
        state,
        renderer,
        &entry.output,
        &shared_scene.expect("shared scene elements"),
    );
    let mut space_render_elements = smithay::desktop::space::space_render_elements(
        renderer,
        [&state.space],
        &entry.output,
        1.0,
    )
    .expect("space render elements");
    remove_duplicate_overlay_elements(&scene, &mut space_render_elements);
    let num_upper = count_upper_layer_render_elements(renderer, &entry.output);
    let counts = get_render_element_counts(&scene, space_render_elements.len(), num_upper);

    let mut render_elements = Vec::with_capacity(counts.total() + cursor_elements.len());
    render_elements.extend(cursor_elements);
    super::assemble_scene_elements!(
        DrmExtras,
        scene,
        space_render_elements,
        num_upper,
        suppress_upper_layers,
        render_elements
    );
    render_elements
}

fn drm_frame_flags(entry: &OutputSurfaceEntry) -> FrameFlags {
    let mut frame_flags = FrameFlags::DEFAULT;
    if entry.vrr_enabled {
        frame_flags |= FrameFlags::SKIP_CURSOR_ONLY_UPDATES;
    }
    frame_flags
}

fn collect_presentation_feedback(
    state: &WaylandState,
    entry: &OutputSurfaceEntry,
    render_states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut output_feedback = OutputPresentationFeedback::new(&entry.output);
    let surface_flags =
        |surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
         _: &smithay::wayland::compositor::SurfaceData| {
            surface_presentation_feedback_flags_from_states(surface, None, render_states)
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

    for window in state
        .space
        .elements()
        .filter(|window| window_overlaps_output(state, window, &entry.output))
    {
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
    resolved_cursor: &ResolvedCursor,
    local_pointer: Point<f64, smithay::utils::Logical>,
    scale: i32,
    millis: u32,
) -> Vec<DrmExtras> {
    let mut custom_elements = Vec::new();

    match resolved_cursor {
        ResolvedCursor::Hidden => {}
        ResolvedCursor::Named(_) => {
            if let Some(cursor_elem) = cursor_manager.render_element(
                local_pointer,
                resolved_cursor,
                scale,
                millis,
                renderer,
            ) {
                custom_elements.push(DrmExtras::Cursor(cursor_elem));
            }
        }
        ResolvedCursor::Surface { surface, hotspot } => {
            let Some(cursor_elements) = super::cursor_surface_render_elements(
                renderer,
                surface,
                local_pointer,
                *hotspot,
                scale as f64,
            ) else {
                return custom_elements;
            };
            custom_elements.extend(cursor_elements.into_iter().map(DrmExtras::Surface));
        }
        ResolvedCursor::DndIcon {
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

            let Some(dnd_elements) = super::cursor_surface_render_elements(
                renderer,
                icon,
                local_pointer,
                *hotspot,
                scale as f64,
            ) else {
                return custom_elements;
            };
            custom_elements.extend(dnd_elements.into_iter().map(DrmExtras::Surface));
        }
    }

    custom_elements
}

#[cfg(test)]
mod tests {
    use super::output_setup::complete_crtc_assignment;

    #[test]
    fn crtc_matching_backtracks_for_constrained_mst_connector() {
        // The first connector is flexible; the second can use only CRTC 1.
        // A first-fit allocator incorrectly consumes 1 for the first output.
        let assignment = complete_crtc_assignment(&[vec![1, 2], vec![1]], &[]).unwrap();
        assert_eq!(assignment, vec![2, 1]);
    }

    #[test]
    fn crtc_matching_respects_retained_outputs() {
        assert_eq!(
            complete_crtc_assignment(&[vec![1, 2], vec![2, 3]], &[1]),
            Some(vec![2, 3])
        );
    }

    #[test]
    fn crtc_matching_rejects_partial_topologies() {
        assert_eq!(complete_crtc_assignment(&[vec![1], vec![1]], &[]), None);
    }
}
