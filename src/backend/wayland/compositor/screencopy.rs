//! `zwlr_screencopy_manager_v1` protocol implementation.
//!
//! This enables screenshot tools like `grim` and screen-recording tools like
//! `wf-recorder` to capture output contents.
//!
//! # Protocol flow
//!
//! 1. Client binds `zwlr_screencopy_manager_v1`.
//! 2. Client calls `capture_output` or `capture_output_region` -> compositor
//!    creates a `ZwlrScreencopyFrameV1` object and sends one or more `buffer`
//!    events advertising supported buffer formats, then (v3+) `buffer_done`.
//! 3. Client allocates a matching buffer and calls `copy` (or
//!    `copy_with_damage`).
//! 4. Compositor queues a `PendingScreencopy`. On the next matching render for
//!    that output, `submit_pending_screencopies` fulfils it and sends
//!    `flags` + `ready`.
//!
//! # Y-inversion
//!
//! OpenGL framebuffer reads are bottom-to-top. Screencopy clients such as
//! `grim` and `wf-recorder` interpret this correctly when the `Y_INVERT` flag
//! is set, so we always send that flag.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::WaylandState;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::backend::renderer::{Bind, Blit, BufferType, ExportMem, TextureFilter, buffer_type};
use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Buffer as BufferCoords, Logical, Rectangle, Size};
use smithay::wayland::dmabuf::get_dmabuf;

/// Protocol version we advertise.
///
/// v3 adds `linux_dmabuf` and `buffer_done` events.
const SCREENCOPY_VERSION: u32 = 3;

/// Per-frame state attached to each `ZwlrScreencopyFrameV1` resource.
pub enum ScreencopyFrameState {
    /// The frame was rejected at creation time (unknown output, invalid
    /// region, etc.). Any subsequent `copy` request immediately gets `failed`.
    Failed,
    Pending {
        output: Output,
        buffer_region: Rectangle<i32, BufferCoords>,
        overlay_cursor: bool,
        copied: Arc<AtomicBool>,
    },
}

/// A pending screencopy request ready to be fulfilled during render.
pub struct PendingScreencopy {
    pub output: Output,
    pub buffer_region: Rectangle<i32, BufferCoords>,
    pub overlay_cursor: bool,
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    pub with_damage: bool,
}

impl WaylandState {
    /// Register the `zwlr_screencopy_manager_v1` global so that clients can
    /// bind it.
    pub fn init_screencopy_manager(&self) {
        self.display_handle
            .create_global::<WaylandState, ZwlrScreencopyManagerV1, ()>(SCREENCOPY_VERSION, ());
    }

    fn screencopy_dmabuf_supported(&mut self) -> bool {
        self.renderer_mut()
            .and_then(|renderer| Bind::<Dmabuf>::supported_formats(renderer))
            .is_some_and(|formats| formats.iter().any(|format| format.code == Fourcc::Xrgb8888))
    }

    fn screencopy_dmabuf_compatible(
        &mut self,
        dmabuf: &Dmabuf,
        buffer_region: Rectangle<i32, BufferCoords>,
    ) -> bool {
        dmabuf.size().w == buffer_region.size.w
            && dmabuf.size().h == buffer_region.size.h
            && dmabuf.format().code == Fourcc::Xrgb8888
            && self
                .renderer_mut()
                .and_then(|renderer| Bind::<Dmabuf>::supported_formats(renderer))
                .is_some_and(|formats| formats.contains(&dmabuf.format()))
    }
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for WaylandState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for WaylandState {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &ZwlrScreencopyManagerV1,
        request: <ZwlrScreencopyManagerV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                let Some(output) = Output::from_resource(&output) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                let Some(mode) = output.current_mode() else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                let buffer_region =
                    Rectangle::<i32, BufferCoords>::from_size((mode.size.w, mode.size.h).into());
                init_frame(
                    state,
                    data_init,
                    manager,
                    frame,
                    output,
                    buffer_region,
                    overlay_cursor != 0,
                );
            }

            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => {
                if width <= 0 || height <= 0 {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                }

                let Some(output) = Output::from_resource(&output) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                let Some(mode) = output.current_mode() else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };

                let logical_size = state
                    .space
                    .output_geometry(&output)
                    .map(|geo| geo.size)
                    .unwrap_or_else(|| (mode.size.w, mode.size.h).into());
                let output_rect = Rectangle::from_size(logical_size);
                let request_rect =
                    Rectangle::<i32, Logical>::new((x, y).into(), (width, height).into());
                let Some(clamped) = request_rect.intersection(output_rect) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };

                let output_scale = output.current_scale().fractional_scale();
                let buffer_region = clamped
                    .to_f64()
                    .to_buffer(
                        output_scale,
                        output.current_transform().invert(),
                        &logical_size.to_f64(),
                    )
                    .to_i32_round();

                init_frame(
                    state,
                    data_init,
                    manager,
                    frame,
                    output,
                    buffer_region,
                    overlay_cursor != 0,
                );
            }

            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

fn init_frame(
    state: &mut WaylandState,
    data_init: &mut DataInit<'_, WaylandState>,
    manager: &ZwlrScreencopyManagerV1,
    frame: New<ZwlrScreencopyFrameV1>,
    output: Output,
    buffer_region: Rectangle<i32, BufferCoords>,
    overlay_cursor: bool,
) {
    let size: Size<i32, BufferCoords> = buffer_region.size;

    let frame = data_init.init(
        frame,
        ScreencopyFrameState::Pending {
            output,
            buffer_region,
            overlay_cursor,
            copied: Arc::new(AtomicBool::new(false)),
        },
    );

    frame.buffer(
        wl_shm::Format::Xrgb8888,
        size.w as u32,
        size.h as u32,
        size.w as u32 * 4,
    );

    if manager.version() >= 3 && state.screencopy_dmabuf_supported() {
        frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, size.w as u32, size.h as u32);
    }

    if manager.version() >= 3 {
        frame.buffer_done();
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState> for WaylandState {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: <ZwlrScreencopyFrameV1 as Resource>::Request,
        data: &ScreencopyFrameState,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            zwlr_screencopy_frame_v1::Request::Destroy => return,
            _ => return,
        };

        let (output, buffer_region, overlay_cursor, copied) = match data {
            ScreencopyFrameState::Failed => {
                frame.failed();
                return;
            }
            ScreencopyFrameState::Pending {
                output,
                buffer_region,
                overlay_cursor,
                copied,
            } => (output, buffer_region, overlay_cursor, copied),
        };

        if copied.swap(true, Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "copy was already requested on this frame",
            );
            return;
        }

        let buffer_ok = match buffer_type(&buffer) {
            Some(BufferType::Shm) => {
                smithay::wayland::shm::with_buffer_contents(&buffer, |_ptr, _len, bd| {
                    bd.format == wl_shm::Format::Xrgb8888
                        && bd.width == buffer_region.size.w
                        && bd.height == buffer_region.size.h
                        && bd.stride == buffer_region.size.w * 4
                })
                .unwrap_or(false)
            }
            Some(BufferType::Dma) => get_dmabuf(&buffer)
                .map(|dmabuf| state.screencopy_dmabuf_compatible(dmabuf, *buffer_region))
                .unwrap_or(false),
            _ => false,
        };

        if !buffer_ok {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "buffer dimensions or format do not match the advertised parameters",
            );
            return;
        }

        state.runtime.pending_screencopies.push(PendingScreencopy {
            output: output.clone(),
            buffer_region: *buffer_region,
            overlay_cursor: *overlay_cursor,
            frame: frame.clone(),
            buffer,
            with_damage,
        });

        // Always schedule one render so live screencasts can deliver an initial
        // frame even when the output is otherwise idle. `copy_with_damage`
        // clients will still wait for real damage after that first frame.
        state.request_render();
    }
}

/// Fulfil pending screencopy requests for one output and one cursor mode.
pub fn submit_pending_screencopies(
    pending: &mut Vec<PendingScreencopy>,
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    output: &Output,
    overlay_cursor: bool,
) {
    let drained: Vec<PendingScreencopy> = {
        let mut remaining = Vec::new();
        let mut matched = Vec::new();
        for pending_copy in pending.drain(..) {
            if pending_copy.output == *output && pending_copy.overlay_cursor == overlay_cursor {
                matched.push(pending_copy);
            } else {
                remaining.push(pending_copy);
            }
        }
        *pending = remaining;
        matched
    };

    if drained.is_empty() {
        return;
    }

    for screencopy in drained {
        let region = screencopy.buffer_region;

        let copy_result = match buffer_type(&screencopy.buffer) {
            Some(BufferType::Shm) => {
                copy_into_shm(renderer, framebuffer, region, &screencopy.buffer)
            }
            Some(BufferType::Dma) => {
                copy_into_dmabuf(renderer, framebuffer, region, &screencopy.buffer)
            }
            _ => {
                log::warn!("screencopy: unsupported client buffer type");
                Err(())
            }
        };

        if copy_result.is_err() {
            screencopy.frame.failed();
            continue;
        }

        if screencopy.with_damage {
            screencopy
                .frame
                .damage(0, 0, region.size.w as u32, region.size.h as u32);
        }

        screencopy
            .frame
            .flags(zwlr_screencopy_frame_v1::Flags::YInvert);

        let presented = monotonic_timestamp();
        let tv_sec_hi = (presented.as_secs() >> 32) as u32;
        let tv_sec_lo = (presented.as_secs() & 0xFFFF_FFFF) as u32;
        screencopy
            .frame
            .ready(tv_sec_hi, tv_sec_lo, presented.subsec_nanos());
    }
}

fn copy_into_shm(
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    region: Rectangle<i32, BufferCoords>,
    buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
) -> Result<(), ()> {
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Xrgb8888)
        .map_err(|err| {
            log::warn!("screencopy: copy_framebuffer failed: {:?}", err);
        })?;

    let pixels = renderer.map_texture(&mapping).map_err(|err| {
        log::warn!("screencopy: map_texture failed: {:?}", err);
    })?;

    smithay::wayland::shm::with_buffer_contents_mut(buffer, |dst_ptr, dst_len, bd| {
        let src_stride = region.size.w as usize * 4;
        let dst_stride = bd.stride as usize;
        let height = region.size.h as usize;
        let copy_w = (region.size.w as usize * 4).min(dst_stride);

        // SAFETY: with_buffer_contents_mut guarantees dst_ptr is valid for dst_len.
        let dst_slice = unsafe { std::slice::from_raw_parts_mut(dst_ptr, dst_len) };

        for row in 0..height {
            let src_offset = row * src_stride;
            let dst_offset = row * dst_stride;
            if src_offset + copy_w > pixels.len() || dst_offset + copy_w > dst_len {
                break;
            }
            dst_slice[dst_offset..dst_offset + copy_w]
                .copy_from_slice(&pixels[src_offset..src_offset + copy_w]);
        }
    })
    .map_err(|_| {
        log::warn!("screencopy: failed to write to client SHM buffer");
    })
}

fn copy_into_dmabuf(
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    region: Rectangle<i32, BufferCoords>,
    buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
) -> Result<(), ()> {
    let dmabuf = get_dmabuf(buffer).map_err(|err| {
        log::warn!("screencopy: failed to access client dmabuf: {:?}", err);
    })?;

    let mut dmabuf = dmabuf.clone();
    let mut target = renderer.bind(&mut dmabuf).map_err(|err| {
        log::warn!("screencopy: failed to bind client dmabuf: {:?}", err);
    })?;

    let _ = renderer
        .blit(
            framebuffer,
            &mut target,
            Rectangle::<i32, smithay::utils::Physical>::new(
                (region.loc.x, region.loc.y).into(),
                (region.size.w, region.size.h).into(),
            ),
            Rectangle::<i32, smithay::utils::Physical>::from_size(
                (region.size.w, region.size.h).into(),
            ),
            TextureFilter::Linear,
        )
        .map_err(|err| {
            log::warn!("screencopy: dmabuf blit failed: {:?}", err);
        })?;

    Ok(())
}

fn monotonic_timestamp() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` points to valid writable storage owned by this function.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } == 0
        && ts.tv_sec >= 0
        && ts.tv_nsec >= 0
    {
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    } else {
        Duration::ZERO
    }
}
