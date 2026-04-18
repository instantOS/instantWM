use std::time::Duration;

use smithay::{
    backend::{
        allocator::{Fourcc, dmabuf::Dmabuf},
        drm::DrmNode,
        renderer::{
            Bind, Blit, BufferType, ExportMem, TextureFilter, buffer_type,
            gles::{GlesRenderer, GlesTarget},
        },
    },
    output::{Output, WeakOutput},
    reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm},
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

        self.runtime
            .pending_image_captures
            .push(PendingImageCapture {
                transform: output.current_transform(),
                output,
                overlay_cursor: session.draw_cursor(),
                size,
                frame,
            });
        self.request_render();
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

pub fn submit_image_captures(
    captures: Vec<PendingImageCapture>,
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
) {
    for capture in captures {
        let buffer = capture.frame.buffer();
        let region = Rectangle::<i32, BufferCoords>::from_size(capture.size);
        let result = match buffer_type(&buffer) {
            Some(BufferType::Shm) => copy_into_shm(renderer, framebuffer, region, &buffer),
            Some(BufferType::Dma) => copy_into_dmabuf(renderer, framebuffer, region, &buffer),
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

fn copy_into_shm(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    region: Rectangle<i32, BufferCoords>,
    buffer: &WlBuffer,
) -> Result<(), ()> {
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Xrgb8888)
        .map_err(|err| {
            log::warn!("image-capture: copy_framebuffer failed: {:?}", err);
        })?;

    let pixels = renderer.map_texture(&mapping).map_err(|err| {
        log::warn!("image-capture: map_texture failed: {:?}", err);
    })?;

    smithay::wayland::shm::with_buffer_contents_mut(buffer, |dst_ptr, dst_len, data| {
        let src_stride = region.size.w as usize * 4;
        let copy_w = src_stride.min(data.stride as usize);
        let height = region.size.h.max(0) as usize;
        let dst_stride = data.stride.max(0) as usize;

        if data.format != wl_shm::Format::Xrgb8888
            || data.width < region.size.w
            || data.height < region.size.h
        {
            return;
        }

        let dst = unsafe { std::slice::from_raw_parts_mut(dst_ptr, dst_len) };
        for row in 0..height {
            let src_offset = row * src_stride;
            let dst_offset = row * dst_stride;
            if src_offset + copy_w > pixels.len() || dst_offset + copy_w > dst.len() {
                break;
            }
            dst[dst_offset..dst_offset + copy_w]
                .copy_from_slice(&pixels[src_offset..src_offset + copy_w]);
        }
    })
    .map_err(|_| {
        log::warn!("image-capture: failed to write SHM buffer");
    })
}

fn copy_into_dmabuf(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    region: Rectangle<i32, BufferCoords>,
    buffer: &WlBuffer,
) -> Result<(), ()> {
    let dmabuf = smithay::wayland::dmabuf::get_dmabuf(buffer).map_err(|err| {
        log::warn!("image-capture: failed to access dmabuf: {:?}", err);
    })?;

    let mut dmabuf = dmabuf.clone();
    let mut target = renderer.bind(&mut dmabuf).map_err(|err| {
        log::warn!("image-capture: failed to bind dmabuf: {:?}", err);
    })?;

    let _ = renderer
        .blit(
            framebuffer,
            &mut target,
            Rectangle::<i32, smithay::utils::Physical>::from_size(
                (region.size.w, region.size.h).into(),
            ),
            Rectangle::<i32, smithay::utils::Physical>::from_size(
                (region.size.w, region.size.h).into(),
            ),
            TextureFilter::Linear,
        )
        .map_err(|err| {
            log::warn!("image-capture: dmabuf blit failed: {:?}", err);
        })?;
    Ok(())
}

pub(crate) fn monotonic_timestamp() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } == 0
        && ts.tv_sec >= 0
        && ts.tv_nsec >= 0
    {
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    } else {
        Duration::ZERO
    }
}
