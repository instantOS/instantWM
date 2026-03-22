//! `zwlr_screencopy_manager_v1` protocol implementation.
//!
//! This enables screenshot tools like `grim` and screen-recording tools like
//! `wf-recorder` to capture output contents.
//!
//! # Protocol flow
//!
//! 1. Client binds `zwlr_screencopy_manager_v1`.
//! 2. Client calls `capture_output` or `capture_output_region` → compositor
//!    creates a `ZwlrScreencopyFrameV1` object and sends one or more `buffer`
//!    events advertising supported buffer formats, then (v3+) `buffer_done`.
//! 3. Client allocates a matching `wl_shm` buffer and calls `copy` (or
//!    `copy_with_damage`).
//! 4. Compositor queues a `PendingScreencopy`.  On the **next rendered frame**
//!    for that output, `submit_pending_screencopies` copies the framebuffer
//!    contents into the client's SHM buffer and sends `flags` + `ready`.
//!
//! # Y-inversion
//!
//! OpenGL's `glReadPixels` (used by `GlesRenderer::copy_framebuffer`) always
//! returns rows in bottom-to-top order.  Screencopy clients such as `grim` and
//! `wf-recorder` interpret this as Y-inverted content when the `Y_INVERT` flag
//! is set, and flip the image accordingly.  We therefore always set this flag.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

use super::{WaylandRuntime, WaylandState};

/// Protocol version we advertise.
///
/// v3 adds `linux_dmabuf` and `buffer_done` events.
const SCREENCOPY_VERSION: u32 = 3;

// ---------------------------------------------------------------------------
// Per-object state
// ---------------------------------------------------------------------------

/// Per-frame state attached to each `ZwlrScreencopyFrameV1` resource.
pub enum ScreencopyFrameState {
    /// The frame was rejected at creation time (unknown output, zero-size
    /// region, etc.).  Any subsequent `copy` request immediately gets
    /// `failed`.
    Failed,
    Pending {
        output: Output,
        physical_region: Rectangle<i32, Physical>,
        overlay_cursor: bool,
        /// Set to `true` once a `copy` or `copy_with_damage` request has been
        /// received, so that duplicate requests are rejected with
        /// `already_used`.
        copied: Arc<AtomicBool>,
    },
}

/// A pending screencopy request ready to be fulfilled during the next render.
pub struct PendingScreencopy {
    pub output: Output,
    pub physical_region: Rectangle<i32, Physical>,
    /// Whether the client requested cursor overlay (not yet implemented;
    /// stored for completeness).
    pub overlay_cursor: bool,
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    /// `true` when the client sent `copy_with_damage` instead of `copy`.
    /// In that case we send a full-region `damage` event before `ready`.
    pub with_damage: bool,
}

// ---------------------------------------------------------------------------
// `init_screencopy_manager` — register the global
// ---------------------------------------------------------------------------

impl WaylandState {
    /// Register the `zwlr_screencopy_manager_v1` global so that clients can
    /// bind it.
    pub fn init_screencopy_manager(&self) {
        self.display_handle
            .create_global::<WaylandRuntime, ZwlrScreencopyManagerV1, ()>(SCREENCOPY_VERSION, ());
    }
}

// ---------------------------------------------------------------------------
// GlobalDispatch — binding the manager
// ---------------------------------------------------------------------------

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for WaylandRuntime {
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

impl Dispatch<ZwlrScreencopyManagerV1, ()> for WaylandRuntime {
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
                    Rectangle::new((0, 0).into(), (mode.size.w, mode.size.h).into());
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
                let output_rect = Rectangle::new((0, 0).into(), (mode.size.w, mode.size.h).into());
                let request_rect = Rectangle::new((x, y).into(), (width, height).into());
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

// ---------------------------------------------------------------------------
// Frame initialisation helper
// ---------------------------------------------------------------------------

fn init_frame(
    data_init: &mut DataInit<'_, WaylandRuntime>,
    manager: &ZwlrScreencopyManagerV1,
    frame: New<ZwlrScreencopyFrameV1>,
    output: Output,
    physical_region: Rectangle<i32, Physical>,
    overlay_cursor: bool,
) {
    let size: Size<i32, Physical> = physical_region.size;

    let frame = data_init.init(
        frame,
        ScreencopyFrameState::Pending {
            output,
            physical_region,
            overlay_cursor,
            copied: Arc::new(AtomicBool::new(false)),
        },
    );

    // Advertise the SHM (wl_shm) buffer format.
    frame.buffer(
        wl_shm::Format::Xrgb8888,
        size.w as u32,
        size.h as u32,
        size.w as u32 * 4,
    );

    // v3+: signal that all buffer types have been enumerated.
    // Note: we intentionally do NOT advertise linux_dmabuf support here,
    // as the copy path only validates SHM buffers (see request handler).
    if manager.version() >= 3 {
        frame.buffer_done();
    }
}

// ---------------------------------------------------------------------------
// Dispatch — frame requests (copy, copy_with_damage, destroy)
// ---------------------------------------------------------------------------

impl Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrameState> for WaylandRuntime {
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

        // Guard against duplicate copy requests on the same frame object.
        if copied.swap(true, Ordering::SeqCst) {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "copy was already requested on this frame",
            );
            return;
        }

        // Validate that the supplied buffer matches the advertised parameters.
        let buffer_ok = smithay::wayland::shm::with_buffer_contents(&buffer, |_ptr, _len, bd| {
            bd.format == wl_shm::Format::Xrgb8888
                && bd.width == physical_region.size.w
                && bd.height == physical_region.size.h
                && bd.stride == physical_region.size.w * 4
        })
        .unwrap_or(false);

        if !buffer_ok {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "buffer dimensions or format do not match the advertised parameters",
            );
            return;
        }

        state.state.pending_screencopies.push(PendingScreencopy {
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
/// requests for that output.
///
/// The renderer **must** still be bound to the output's framebuffer at the
/// point this function is called so that `copy_framebuffer` can read pixels
/// via `glReadPixels`.
///
/// # Y-inversion
///
/// `glReadPixels` returns rows in bottom-to-top order (OpenGL convention),
/// which is the inverse of what on-screen content looks like.  We therefore
/// always set the `Y_INVERT` flag in the screencopy `flags` event so that
/// clients know to flip the image vertically.
pub fn submit_pending_screencopies(
    pending: &mut Vec<PendingScreencopy>,
    renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    output: &Output,
    start_time: std::time::Instant,
) {
    use smithay::backend::allocator::Fourcc;
    use smithay::backend::renderer::ExportMem;
    use smithay::utils::Buffer as BufferCoord;

    // Drain all pending frames that belong to this output, leaving the others
    // intact for the next render cycle.
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

    if drained.is_empty() {
        return;
    }

    for screencopy in drained {
        let region = screencopy.physical_region;

        // `copy_framebuffer` uses a Buffer-space rectangle.
        let buf_region: Rectangle<i32, BufferCoord> = Rectangle::new(
            (region.loc.x, region.loc.y).into(),
            (region.size.w, region.size.h).into(),
        );

        let mapping = match renderer.copy_framebuffer(framebuffer, buf_region, Fourcc::Xrgb8888) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("screencopy: copy_framebuffer failed: {:?}", e);
                screencopy.frame.failed();
                continue;
            }
        };

        let pixels = match renderer.map_texture(&mapping) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("screencopy: map_texture failed: {:?}", e);
                screencopy.frame.failed();
                continue;
            }
        };

        // Copy pixel data into the client's SHM buffer row-by-row, respecting
        // the client's stride which may be wider than the minimal stride.
        let copy_ok = smithay::wayland::shm::with_buffer_contents_mut(
            &screencopy.buffer,
            |dst_ptr, dst_len, bd| {
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
            },
        );

        if copy_ok.is_err() {
            log::warn!("screencopy: failed to write to client SHM buffer");
            screencopy.frame.failed();
            continue;
        }

        // For copy_with_damage, report the entire captured region as damaged.
        // We don't have per-frame damage tracking exposed here, so reporting
        // the full region is always correct (if conservative).
        if screencopy.with_damage {
            screencopy
                .frame
                .damage(0, 0, region.size.w as u32, region.size.h as u32);
        }

        // YInvert flag — DO NOT remove.
        //
        // OpenGL's glReadPixels (called internally by GlesRenderer::copy_framebuffer)
        // always returns pixel rows in bottom-to-top order, which is the opposite
        // of the top-to-bottom order that screen-capture clients expect.  The
        // zwlr-screencopy-v1 protocol defines the YInvert flag precisely for this
        // case: when set, it tells the client that the buffer it received is
        // vertically flipped relative to the on-screen image and the client must
        // flip it before saving or displaying.
        //
        // Tools like `grim` (screenshots) and `wf-recorder` (screen recording)
        // both honour this flag and flip accordingly.  Removing it causes every
        // captured frame to appear upside-down in those tools.
        screencopy
            .frame
            .flags(zwlr_screencopy_frame_v1::Flags::YInvert);

        let elapsed = start_time.elapsed();
        let tv_sec_hi = (elapsed.as_secs() >> 32) as u32;
        let tv_sec_lo = (elapsed.as_secs() & 0xFFFF_FFFF) as u32;
        screencopy
            .frame
            .ready(tv_sec_hi, tv_sec_lo, elapsed.subsec_nanos());
    }
}
