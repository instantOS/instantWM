//! DRM/KMS backend initialization.
//!
//! The DRM backend runs as a standalone compositor directly on hardware.
//! This module handles GPU initialization, EGL context setup, and DRM device
//! opening.

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::Session;
use smithay::reexports::drm::control::connector;
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;

/// Initialize GPU, EGL, and renderer.
///
/// This function is safety-critical: it handles raw file descriptors and
/// unsafe EGL context creation. Do not reorder operations.
pub fn init_gpu(
    session: &mut LibSeatSession,
    seat_name: &str,
) -> (
    std::path::PathBuf,
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
) -> (
    std::path::PathBuf,
    DrmDevice,
    DrmDeviceNotifier,
    DrmDeviceFd,
) {
    let gpus = smithay::backend::udev::all_gpus(seat_name).unwrap_or_default();
    let mut primary_gpu_path = None;
    let mut drm_device = None;
    let mut drm_notifier = None;
    let mut drm_fd = None;

    for gpu_path in gpus {
        if let Ok(fd) = session.open(
            &gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        ) {
            let fd = DrmDeviceFd::new(DeviceFd::from(fd));
            if let Ok((device, notifier)) = DrmDevice::new(fd.clone(), true) {
                let has_connected = device
                    .resource_handles()
                    .map(|res| {
                        res.connectors().iter().any(|&c| {
                            device
                                .get_connector(c, false)
                                .map(|info| info.state() == connector::State::Connected)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);

                if has_connected || primary_gpu_path.is_none() {
                    primary_gpu_path = Some(gpu_path);
                    drm_device = Some(device);
                    drm_notifier = Some(notifier);
                    drm_fd = Some(fd);
                    if has_connected {
                        break;
                    }
                }
            }
        }
    }

    (
        primary_gpu_path.expect("no GPU found"),
        drm_device.expect("failed to open DRM device"),
        drm_notifier.expect("failed to create DRM notifier"),
        drm_fd.expect("failed to get DRM FD"),
    )
}
