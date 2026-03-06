//! `zwlr_screencopy_manager_v1` protocol implementation.
//!
//! This enables screenshot tools like `grim` and screen-recording tools like
//! `wf-recorder` to capture output contents.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Physical, Rectangle, Size};

use super::WaylandState;

/// Protocol version we advertise. v3 adds linux_dmabuf + buffer_done events.
const SCREENCOPY_VERSION: u32 = 3;

/// Per-frame state attached to each `ZwlrScreencopyFrameV1` resource.
pub enum ScreencopyFrameState {
    Failed,
    Pending {
        output: Output,
        physical_region: Rectangle<i32, Physical>,
        overlay_cursor: bool,
        copied: Arc<AtomicBool>,
    },
}

/// A pending screencopy request ready to be fulfilled during the next render.
pub struct PendingScreencopy {
    pub output: Output,
    pub physical_region: Rectangle<i32, Physical>,
    pub overlay_cursor: bool,
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    pub with_damage: bool,
}

impl WaylandState {
    /// Register the `zwlr_screencopy_manager_v1` global.
    pub fn init_screencopy_manager(&self) {
        self.display_handle
            .create_global::<WaylandState, ZwlrScreencopyManagerV1, ()>(SCREENCOPY_VERSION, ());
    }
}

// ---------------------------------------------------------------------------
// GlobalDispatch — binding the manager
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Dispatch — manager requests (capture_output, capture_output_region)
// ---------------------------------------------------------------------------

impl Dispatch<ZwlrScreencopyManagerV1, ()> for WaylandState {
    fn request(
        _state: &mut Self,
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
                let physical_region =
                    Rectangle::from_loc_and_size((0, 0), (mode.size.w, mode.size.h));
                init_frame(
                    data_init,
                    manager,
                    frame,
                    output,
                    physical_region,
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
                let output_rect =
                    Rectangle::from_loc_and_size((0, 0), (mode.size.w, mode.size.h));
                let request_rect = Rectangle::from_loc_and_size((x, y), (width, height));
                let Some(clamped) = request_rect.intersection(output_rect) else {
                    let frame = data_init.init(frame, ScreencopyFrameState::Failed);
                    frame.failed();
                    return;
                };
                init_frame(
                    data_init,
                    manager,
                    frame,
                    output,
                    clamped,
                    overlay_cursor != 0,
                );
            }

            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

fn init_frame(
    data_init: &mut DataInit<'_, WaylandState>,
    manager: &ZwlrScreencopyManagerV1,
    frame: New<ZwlrScreencopyFrameV1>,
    output: Output,
    physical_region: Rectangle<i32, Physical>,
    overlay_cursor: bool,
) {
    let size = physical_region.size;
    let frame = data_init.init(
        frame,
        ScreencopyFrameState::Pending {
            output,
            physical_region,
            overlay_cursor,
            copied: Arc::new(AtomicBool::new(false)),
        },
    );

    // Advertise SHM buffer format.
    frame.buffer(
        wl_shm::Format::Xrgb8888,
        size.w as u32,
        size.h as u32,
        size.w as u32 * 4,
    );

    if manager.version() >= 3 {
        frame.linux_dmabuf(
            smithay::backend::allocator::Fourcc::Xrgb8888 as u32,
            size.w as u32,
            size.h as u32,
        );
        frame.buffer_done();
    }
}

// ---------------------------------------------------------------------------
// Dispatch — frame requests (copy, copy_with_damage, destroy)
// ---------------------------------------------------------------------------

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

        let (output, physical_region, overlay_cursor, copied) = match data {
            ScreencopyFrameState::Failed => {
                frame.failed();
                return;
            }
            ScreencopyFrameState::Pending {
                output,
                physical_region,
                overlay_cursor,
                copied,
            } => (output, physical_region, overlay_cursor, copied),
        };

        if copied.swap(true, Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "copy was already requested",
            );
            return;
        }

        // Validate buffer dimensions via SHM metadata.
        let buffer_ok =
            smithay::wayland::shm::with_buffer_contents(&buffer, |_ptr, _len, bd| {
                bd.format == wl_shm::Format::Xrgb8888
                    && bd.width == physical_region.size.w
                    && bd.height == physical_region.size.h
                    && bd.stride == physical_region.size.w * 4
            })
            .unwrap_or(false);

        if !buffer_ok {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "buffer does not match advertised parameters",
            );
            return;
        }

        state.pending_screencopies.push(PendingScreencopy {
            output: output.clone(),
            physical_region: *physical_region,
            overlay_cursor: *overlay_cursor,
            frame: frame.clone(),
            buffer,
            with_damage,
        });
    }
}

// ---------------------------------------------------------------------------
// Render-time screencopy fulfilment
// ---------------------------------------------------------------------------

/// After rendering an output, call this to fulfil any pending screencopy
/// requests for that output.  `renderer` must still be bound to the
/// output's framebuffer.
pub fn submit_pending_screencopies(
    pending: &mut Vec<PendingScreencopy>,
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    output: &Output,
    start_time: std::time::Instant,
) {
    use smithay::backend::renderer::ExportMem;

    let drained: Vec<PendingScreencopy> = {
        let mut remaining = Vec::new();
        let mut matched = Vec::new();
        for p in pending.drain(..) {
            if p.output == *output {
                matched.push(p);
            } else {
                remaining.push(p);
            }
        }
        *pending = remaining;
        matched
    };

    for screencopy in drained {
        let region = screencopy.physical_region;
        let buf_region = Rectangle::from_loc_and_size(
            (region.loc.x, region.loc.y),
            (region.size.w, region.size.h),
        );

        let mapping = match renderer.copy_framebuffer(
            buf_region,
            smithay::backend::allocator::Fourcc::Xrgb8888,
        ) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("screencopy copy_framebuffer failed: {:?}", e);
                screencopy.frame.failed();
                continue;
            }
        };

        let pixels = match renderer.map_texture(&mapping) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("screencopy map_texture failed: {:?}", e);
                screencopy.frame.failed();
                continue;
            }
        };

        let copy_ok = smithay::wayland::shm::with_buffer_contents_mut(
            &screencopy.buffer,
            |dst_ptr, dst_len, bd| {
                let src_stride = region.size.w as usize * 4;
                let dst_stride = bd.stride as usize;
                let height = region.size.h as usize;
                let copy_w = (region.size.w as usize * 4).min(dst_stride);
                for row in 0..height {
                    let src_offset = row * src_stride;
                    let dst_offset = row * dst_stride;
                    if src_offset + copy_w > pixels.len() || dst_offset + copy_w > dst_len {
                        break;
                    }
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            pixels.as_ptr().add(src_offset),
                            dst_ptr.add(dst_offset),
                            copy_w,
                        );
                    }
                }
            },
        );

        if copy_ok.is_err() {
            log::warn!("screencopy: failed to write to client SHM buffer");
            screencopy.frame.failed();
            continue;
        }

        // Send flags (no Y-invert for standard rendering) then ready.
        screencopy
            .frame
            .flags(zwlr_screencopy_frame_v1::Flags::empty());
        let elapsed = start_time.elapsed();
        let tv_sec_hi = (elapsed.as_secs() >> 32) as u32;
        let tv_sec_lo = (elapsed.as_secs() & 0xFFFF_FFFF) as u32;
        screencopy
            .frame
            .ready(tv_sec_hi, tv_sec_lo, elapsed.subsec_nanos());
    }
}
