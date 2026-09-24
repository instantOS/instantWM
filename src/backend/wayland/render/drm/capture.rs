use super::*;

pub(super) struct DrmCaptureRequests {
    has_cursor_screencopy: bool,
    has_cursorless_screencopy: bool,
    cursor_image_captures: Vec<PendingImageCapture>,
    cursorless_image_captures: Vec<PendingImageCapture>,
}

impl DrmCaptureRequests {
    pub(super) fn has_pending(&self) -> bool {
        self.has_cursor_screencopy
            || self.has_cursorless_screencopy
            || !self.cursor_image_captures.is_empty()
            || !self.cursorless_image_captures.is_empty()
    }
}

pub(super) fn take_drm_capture_requests(
    state: &mut WaylandState,
    output: &Output,
) -> DrmCaptureRequests {
    let has_cursor_screencopy = state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output == *output && copy.overlay_cursor);
    let has_cursorless_screencopy = state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output == *output && !copy.overlay_cursor);
    let cursorless_image_captures =
        crate::backend::wayland::compositor::image_capture::drain_pending_image_captures(
            &mut state.runtime.pending_image_captures,
            output,
            false,
        );
    let cursor_image_captures =
        crate::backend::wayland::compositor::image_capture::drain_pending_image_captures(
            &mut state.runtime.pending_image_captures,
            output,
            true,
        );

    DrmCaptureRequests {
        has_cursor_screencopy,
        has_cursorless_screencopy,
        cursor_image_captures,
        cursorless_image_captures,
    }
}

pub(super) fn submit_drm_capture_requests<B, F>(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &OutputSurfaceEntry,
    frame_result: &RenderFrameResult<'_, B, F, DrmExtras>,
    cursor_element_ids: &[Id],
    requests: DrmCaptureRequests,
) -> bool
where
    B: AllocatorBuffer + AsDmabuf,
    <B as AsDmabuf>::Error: fmt::Debug,
    F: Framebuffer,
    DrmExtras: RenderElement<GlesRenderer>,
    GlesRenderer: Blit,
{
    let target = DrmCaptureTarget::for_output(entry);
    let (cursorless_dmabuf_captures, cursorless_target_captures) =
        split_dmabuf_captures(requests.cursorless_image_captures);
    let (cursor_dmabuf_captures, cursor_target_captures) =
        split_dmabuf_captures(requests.cursor_image_captures);

    submit_dmabuf_image_captures(
        renderer,
        frame_result,
        &target,
        cursorless_dmabuf_captures,
        cursor_element_ids,
    );
    submit_dmabuf_image_captures(renderer, frame_result, &target, cursor_dmabuf_captures, &[]);

    let cursorless_ok = submit_offscreen_capture(
        state,
        renderer,
        entry,
        frame_result,
        &target,
        requests.has_cursorless_screencopy,
        cursorless_target_captures,
        cursor_element_ids,
        false,
    );
    let cursor_ok = submit_offscreen_capture(
        state,
        renderer,
        entry,
        frame_result,
        &target,
        requests.has_cursor_screencopy,
        cursor_target_captures,
        &[],
        true,
    );

    cursorless_ok && cursor_ok
}

struct DrmCaptureTarget {
    size: smithay::utils::Size<i32, Physical>,
    buffer_size: smithay::utils::Size<i32, BufferCoords>,
    transform: smithay::utils::Transform,
    scale: f64,
}

impl DrmCaptureTarget {
    fn for_output(entry: &OutputSurfaceEntry) -> Self {
        let scale = entry.output.current_scale().fractional_scale();
        let transform = entry.output.current_transform().invert();
        let mode_size = entry
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_else(|| (entry.rect.w, entry.rect.h).into());
        let size = transform.transform_size(mode_size);
        let buffer_size = (size.w, size.h).into();

        Self {
            size,
            buffer_size,
            transform,
            scale,
        }
    }
}

fn split_dmabuf_captures(
    captures: Vec<PendingImageCapture>,
) -> (Vec<PendingImageCapture>, Vec<PendingImageCapture>) {
    let mut dmabuf_captures = Vec::new();
    let mut target_captures = Vec::new();

    for capture in captures {
        if matches!(buffer_type(&capture.frame.buffer()), Some(BufferType::Dma)) {
            dmabuf_captures.push(capture);
        } else {
            target_captures.push(capture);
        }
    }

    (dmabuf_captures, target_captures)
}

fn submit_dmabuf_image_captures<B, F>(
    renderer: &mut GlesRenderer,
    frame_result: &RenderFrameResult<'_, B, F, DrmExtras>,
    target_info: &DrmCaptureTarget,
    captures: Vec<PendingImageCapture>,
    filter_ids: &[Id],
) where
    B: AllocatorBuffer + AsDmabuf,
    <B as AsDmabuf>::Error: fmt::Debug,
    F: Framebuffer,
    DrmExtras: RenderElement<GlesRenderer>,
    GlesRenderer: Blit,
{
    for capture in captures {
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
            target_info.size,
            target_info.transform,
            target_info.scale,
            renderer,
            &mut target,
            [Rectangle::from_size(target_info.size)],
            filter_ids.iter().cloned(),
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
}

#[allow(clippy::too_many_arguments)]
fn submit_offscreen_capture<B, F>(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    entry: &OutputSurfaceEntry,
    frame_result: &RenderFrameResult<'_, B, F, DrmExtras>,
    target_info: &DrmCaptureTarget,
    has_screencopy: bool,
    image_captures: Vec<PendingImageCapture>,
    filter_ids: &[Id],
    overlay_cursor: bool,
) -> bool
where
    B: AllocatorBuffer + AsDmabuf,
    <B as AsDmabuf>::Error: fmt::Debug,
    F: Framebuffer,
    DrmExtras: RenderElement<GlesRenderer>,
    GlesRenderer: Blit,
{
    if !has_screencopy && image_captures.is_empty() {
        return true;
    }

    let mut capture: GlesTexture =
        match renderer.create_buffer(Fourcc::Xrgb8888, target_info.buffer_size) {
            Ok(buffer) => buffer,
            Err(err) => {
                log::warn!("screencopy offscreen buffer creation failed: {:?}", err);
                return false;
            }
        };
    match renderer.bind(&mut capture) {
        Ok(mut target) => match frame_result.blit_frame_result(
            target_info.size,
            target_info.transform,
            target_info.scale,
            renderer,
            &mut target,
            [Rectangle::from_size(target_info.size)],
            filter_ids.iter().cloned(),
        ) {
            Ok(sync) => {
                crate::backend::wayland::compositor::screencopy::submit_pending_screencopies(
                    &mut state.runtime.pending_screencopies,
                    renderer,
                    &target,
                    &entry.output,
                    overlay_cursor,
                );
                crate::backend::wayland::compositor::image_capture::submit_image_captures(
                    image_captures,
                    renderer,
                    &target,
                );
                let _ = sync;
                true
            }
            Err(err) => {
                log::warn!("screencopy blit_frame_result failed: {:?}", err);
                false
            }
        },
        Err(err) => {
            log::warn!("screencopy offscreen bind failed: {:?}", err);
            false
        }
    }
}
