use smithay::{
    backend::{
        allocator::{Fourcc, dmabuf::Dmabuf},
        drm::DrmNode,
        renderer::{
            Bind, BufferType, buffer_type,
            gles::{GlesRenderer, GlesTarget},
        },
    },
    output::{Output, WeakOutput},
    reexports::wayland_server::protocol::wl_shm,
    utils::{Buffer as BufferCoords, IsAlive, Rectangle, Transform},
    wayland::{
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
            OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, DmabufConstraints, Frame, FrameRef,
            ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef,
        },
    },
};

use super::WaylandState;

// Re-exported so the render backends keep using the shared implementation.
pub(crate) use super::capture_common::monotonic_timestamp;

pub struct PendingImageCapture {
    pub output: Output,
    pub overlay_cursor: bool,
    pub transform: Transform,
    pub size: smithay::utils::Size<i32, BufferCoords>,
    pub frame: Frame,
}

impl ImageCaptureSourceHandler for WaylandState {
    fn source_destroyed(&mut self, _source: ImageCaptureSource) {}
}

impl OutputCaptureSourceHandler for WaylandState {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for WaylandState {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let weak_output = source.user_data().get::<WeakOutput>()?;
        let output = weak_output.upgrade()?;
        let size = capture_size_for_output(&output)?;

        let render_node = self.render_node;
        let dma = {
            let renderer = self.renderer_mut();
            capture_dmabuf_constraints(renderer, render_node, Fourcc::Xrgb8888)
        };

        let shm_formats = vec![wl_shm::Format::Xrgb8888];

        Some(BufferConstraints {
            size,
            shm: shm_formats,
            dma,
        })
    }

    fn new_session(&mut self, session: Session) {
        if let Some(constraints) = self.capture_constraints(&session.source()) {
            session.as_ref().update_constraints(constraints);
        }
        self.runtime
            .image_copy_sessions
            .retain(|session| session.alive());
        self.runtime.image_copy_sessions.push(session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        let source = session.source();
        let Some(weak_output) = source.user_data().get::<WeakOutput>() else {
            frame.fail(CaptureFailureReason::Unknown);
            return;
        };
        let Some(output) = weak_output.upgrade() else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        let Some(size) = capture_size_for_output(&output) else {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        };
        if !self.output_can_render(&output) {
            frame.fail(CaptureFailureReason::Stopped);
            return;
        }

        self.runtime
            .pending_image_captures
            .push(PendingImageCapture {
                transform: output.current_transform(),
                output: output.clone(),
                overlay_cursor: session.draw_cursor(),
                size,
                frame,
            });
        self.request_output_render(&output);
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        self.runtime
            .pending_image_captures
            .retain(|pending| pending.frame != frame);
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.runtime
            .image_copy_sessions
            .retain(|stored| stored.as_ref() != session);
    }
}

fn capture_size_for_output(output: &Output) -> Option<smithay::utils::Size<i32, BufferCoords>> {
    let mode = output.current_mode()?;
    let size = output.current_transform().transform_size(mode.size);
    Some((size.w, size.h).into())
}

fn capture_dmabuf_constraints(
    renderer: Option<&mut GlesRenderer>,
    render_node: Option<DrmNode>,
    code: Fourcc,
) -> Option<DmabufConstraints> {
    let renderer = renderer?;
    let node = render_node?;
    let formats = Bind::<Dmabuf>::supported_formats(renderer)?;
    let modifiers = formats
        .iter()
        .filter(|format| format.code == code)
        .map(|format| format.modifier)
        .collect::<Vec<_>>();
    if modifiers.is_empty() {
        return None;
    }

    Some(DmabufConstraints {
        node,
        formats: vec![(code, modifiers)],
    })
}

pub fn submit_pending_image_captures(
    pending: &mut Vec<PendingImageCapture>,
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    output: &Output,
    overlay_cursor: bool,
) {
    let drained = drain_pending_image_captures(pending, output, overlay_cursor);
    submit_image_captures(drained, renderer, framebuffer);
}

pub fn drain_pending_image_captures(
    pending: &mut Vec<PendingImageCapture>,
    output: &Output,
    overlay_cursor: bool,
) -> Vec<PendingImageCapture> {
    let mut remaining = Vec::new();
    let mut matched = Vec::new();
    for pending_capture in pending.drain(..) {
        if pending_capture.output == *output && pending_capture.overlay_cursor == overlay_cursor {
            matched.push(pending_capture);
        } else {
            remaining.push(pending_capture);
        }
    }
    *pending = remaining;
    matched
}

/// Fail and remove requests for an output that can no longer be rendered.
pub fn fail_pending_image_captures_for_output(
    pending: &mut Vec<PendingImageCapture>,
    output: &Output,
) {
    let mut remaining = Vec::with_capacity(pending.len());
    for capture in pending.drain(..) {
        if capture.output == *output {
            capture.frame.fail(CaptureFailureReason::Stopped);
        } else {
            remaining.push(capture);
        }
    }
    *pending = remaining;
}

pub fn submit_image_captures(
    captures: Vec<PendingImageCapture>,
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
) {
    for capture in captures {
        let buffer = capture.frame.buffer();
        let region = Rectangle::<i32, BufferCoords>::from_size(capture.size);
        let result = match buffer_type(&buffer) {
            Some(BufferType::Shm) => super::capture_common::copy_into_shm(
                renderer,
                framebuffer,
                region,
                &buffer,
                "image-capture",
            ),
            Some(BufferType::Dma) => super::capture_common::copy_into_dmabuf(
                renderer,
                framebuffer,
                region,
                &buffer,
                "image-capture",
            ),
            _ => Err(()),
        };

        if result.is_err() {
            capture.frame.fail(CaptureFailureReason::Unknown);
            continue;
        }

        capture.frame.success(
            capture.transform,
            None::<Vec<Rectangle<i32, BufferCoords>>>,
            monotonic_timestamp(),
        );
    }
}
