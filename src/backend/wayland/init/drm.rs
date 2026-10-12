//! DRM/KMS backend initialization.
//!
//! The DRM backend runs as a standalone compositor directly on hardware.
//! This module handles GPU initialization, EGL context setup, and DRM device
//! opening.

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::rustix::fs::OFlags;
use std::path::PathBuf;

use smithay::utils::DeviceFd;

/// Initialize GPU, EGL, and renderer.
///
/// This function is safety-critical: it handles raw file descriptors and
/// unsafe EGL context creation. Do not reorder operations.
pub fn init_gpu(
    session: &mut LibSeatSession,
    seat_name: &str,
) -> (
    PathBuf,
    DrmDevice,
    DrmDeviceNotifier,
    DrmDeviceFd,
    GbmDevice<DrmDeviceFd>,
    EGLDisplay,
    GlesRenderer,
) {
    let (primary_gpu_path, drm_device, drm_notifier, drm_fd) = open_primary_gpu(session, seat_name);

    let gbm_device = GbmDevice::new(drm_fd.clone()).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new(&egl_display).expect("EGLContext::new");
    let renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    (
        primary_gpu_path,
        drm_device,
        drm_notifier,
        drm_fd,
        gbm_device,
        egl_display,
        renderer,
    )
}

fn open_primary_gpu(
    session: &mut LibSeatSession,
    seat_name: &str,
) -> (PathBuf, DrmDevice, DrmDeviceNotifier, DrmDeviceFd) {
    let mut gpus = smithay::backend::udev::all_gpus(seat_name).expect("enumerate rendering GPUs");
    let primary = smithay::backend::udev::primary_gpu(seat_name)
        .ok()
        .flatten();
    gpus.sort_by_key(|path| (Some(path) != primary.as_ref(), path.clone()));

    for gpu_path in gpus {
        // EVDI is scanout-only and may be card0. Never select it as the
        // compositor's renderer merely because it has a connected monitor.
        let render_node = smithay::backend::drm::DrmNode::from_path(&gpu_path)
            .ok()
            .and_then(|node| node.node_with_type(smithay::backend::drm::NodeType::Render))
            .and_then(Result::ok);
        if render_node.is_none() {
            continue;
        }
        let owned = match session.open(
            &gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        ) {
            Ok(fd) => fd,
            Err(error) => {
                log::warn!("Cannot open rendering GPU {}: {error}", gpu_path.display());
                continue;
            }
        };
        let fd = DrmDeviceFd::new(DeviceFd::from(owned));
        match DrmDevice::new(fd.clone(), true) {
            Ok((device, notifier)) => return (gpu_path, device, notifier, fd),
            Err(error) => {
                log::warn!(
                    "Cannot initialize rendering GPU {}: {error}",
                    gpu_path.display()
                );
                let session_fd = fd.device_fd();
                drop(fd);
                if let Ok(fd) = TryInto::<std::os::fd::OwnedFd>::try_into(session_fd) {
                    let _ = session.close(fd);
                }
            }
        }
    }
    panic!("no usable DRM rendering GPU found");
}
