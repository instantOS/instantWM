//! Shared pixel-copy helpers for the screencopy and image-capture protocols.
//!
//! Both protocols fulfil their frames by copying the rendered framebuffer
//! region into a client-provided `wl_buffer` (SHM or dmabuf). The copy itself
//! is identical, so it lives here instead of being duplicated per protocol.
//!
//! Behavioral notes:
//! - `copy_into_shm` validates the buffer contents (format + size) itself:
//!   the image-capture protocol is not validated at request time, and for
//!   screencopy it is a redundant second net on top of the request-time
//!   `InvalidBuffer` check.
//! - `copy_into_dmabuf` blits the region *at its location* inside the
//!   framebuffer (mirroring `ExportMem::copy_framebuffer`, which reads at
//!   `region.loc`). Callers whose region is always origin-based pass a
//!   zero location and are unaffected.
//! - `monotonic_timestamp` is the single source of truth for presentation
//!   timestamps across both protocols.

use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTarget};
use smithay::backend::renderer::{Bind, Blit, ExportMem, TextureFilter};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::utils::{Buffer as BufferCoords, Rectangle};
use smithay::wayland::dmabuf::get_dmabuf;

/// Copy `region` of the framebuffer into the client's SHM buffer.
///
/// Stops early (partial frame) if a row would go out of bounds on either the
/// source or the destination. Any failure aborts the whole copy and yields
/// `Err`, so the caller can signal the frame as failed.
pub(crate) fn copy_into_shm(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    region: Rectangle<i32, BufferCoords>,
    buffer: &WlBuffer,
    log_prefix: &str,
) -> Result<(), ()> {
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Xrgb8888)
        .map_err(|err| {
            log::warn!("{log_prefix}: copy_framebuffer failed: {:?}", err);
        })?;

    let pixels = renderer.map_texture(&mapping).map_err(|err| {
        log::warn!("{log_prefix}: map_texture failed: {:?}", err);
    })?;

    smithay::wayland::shm::with_buffer_contents_mut(buffer, |dst_ptr, dst_len, bd| {
        if bd.format != wl_shm::Format::Xrgb8888
            || bd.width < region.size.w
            || bd.height < region.size.h
        {
            return;
        }

        let src_stride = region.size.w as usize * 4;
        let dst_stride = bd.stride.max(0) as usize;
        let copy_w = src_stride.min(dst_stride);
        let height = region.size.h.max(0) as usize;

        // SAFETY: with_buffer_contents_mut guarantees dst_ptr is valid for dst_len.
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
        log::warn!("{log_prefix}: failed to write to SHM buffer");
    })
}

/// Blit `region` of the framebuffer into the client's D-BUF buffer.
pub(crate) fn copy_into_dmabuf(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    region: Rectangle<i32, BufferCoords>,
    buffer: &WlBuffer,
    log_prefix: &str,
) -> Result<(), ()> {
    let dmabuf = get_dmabuf(buffer).map_err(|err| {
        log::warn!("{log_prefix}: failed to access client dmabuf: {:?}", err);
    })?;

    let mut dmabuf = dmabuf.clone();
    let mut target = renderer.bind(&mut dmabuf).map_err(|err| {
        log::warn!("{log_prefix}: failed to bind client dmabuf: {:?}", err);
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
            log::warn!("{log_prefix}: dmabuf blit failed: {:?}", err);
        })?;

    Ok(())
}

/// Monotonic system time for presentation timestamps.
pub(crate) fn monotonic_timestamp() -> Duration {
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
