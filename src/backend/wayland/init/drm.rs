//! DRM/KMS backend initialization.
//!
//! The DRM backend runs as a standalone compositor directly on hardware.
//! This module handles GPU initialization, EGL context setup, and DRM device
//! opening.

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier};
use smithay::backend::egl::context::ContextPriority;
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
///
/// The compositor may run on a "split" GPU setup where the device that owns
/// the display (KMS) differs from the device that provides rendering (EGL).
/// This is common on ARM SoCs (e.g. MediaTek display + Mali/Panfrost render).
/// We therefore select the scanout (KMS) device and the render (EGL) device
/// independently, and use the render device's GBM for allocation while letting
/// each scanout device import those buffers (cross-device).
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
    // Pick a GPU that can actually render (has a render node) for EGL/GBM.
    let gpus = smithay::backend::udev::all_gpus(seat_name).expect("enumerate rendering GPUs");
    let primary = smithay::backend::udev::primary_gpu(seat_name)
        .ok()
        .flatten();
    let render_card = primary
        .clone()
        .filter(has_render_node)
        .or_else(|| gpus.iter().find(|path| has_render_node(path)).cloned())
        .unwrap_or_else(|| {
            panic!(
                "no GPU with a render node found among {:?}; \
                 instantWM's DRM backend needs a render-capable GPU",
                gpus
            )
        });

    // Pick a GPU that can do KMS mode-setting for scanout.
    let (scanout_path, drm_device, drm_notifier, drm_fd) = open_scanout_gpu(session, seat_name);

    // Open the render device.  If it is the same device as the scanout device,
    // reuse the already-open scanout fd (a single GPU case).  Otherwise open
    // its render node directly.
    let render_fd = if render_card == scanout_path {
        drm_fd.clone()
    } else {
        let render_node = smithay::backend::drm::DrmNode::from_path(&render_card)
            .ok()
            .and_then(|node| node.node_with_type(smithay::backend::drm::NodeType::Render))
            .and_then(Result::ok)
            .and_then(|node| node.dev_path())
            .unwrap_or_else(|| render_card.clone());
        let owned = match session.open(
            &render_node,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        ) {
            Ok(fd) => fd,
            Err(_) => {
                // Render nodes are not always managed by the seat; fall back
                // to a direct open (they are typically group-readable).
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&render_node)
                    .unwrap_or_else(|error| {
                        panic!(
                            "Cannot open render GPU {}: {error}. \
                             Make sure you have access to the render node.",
                            render_node.display()
                        )
                    });
                file.into()
            }
        };
        DrmDeviceFd::new(DeviceFd::from(owned))
    };

    let gbm_device = GbmDevice::new(render_fd.clone()).expect("GbmDevice::new");
    let egl_display = unsafe { EGLDisplay::new(gbm_device.clone()) }.expect("EGLDisplay::new");
    let egl_context = EGLContext::new_with_priority(&egl_display, ContextPriority::High)
        .or_else(|err| {
            log::warn!("High-priority EGL context unavailable ({err}); using default priority");
            EGLContext::new(&egl_display)
        })
        .expect("EGLContext::new");
    let renderer = unsafe { GlesRenderer::new(egl_context) }.expect("GlesRenderer::new");

    (
        scanout_path,
        drm_device,
        drm_notifier,
        drm_fd,
        gbm_device,
        egl_display,
        renderer,
    )
}

fn has_render_node(path: &PathBuf) -> bool {
    smithay::backend::drm::DrmNode::from_path(path)
        .ok()
        .and_then(|node| node.node_with_type(smithay::backend::drm::NodeType::Render))
        .and_then(Result::ok)
        .is_some()
}

/// Open a GPU that supports KMS mode-setting (has resource handles).
///
/// Unlike the old `open_primary_gpu`, this does not require a render node:
/// a display-only device (e.g. a MediaTek KMS controller) is a valid scanout
/// target.  Rendering happens on a separate render GPU.
fn open_scanout_gpu(
    session: &mut LibSeatSession,
    seat_name: &str,
) -> (PathBuf, DrmDevice, DrmDeviceNotifier, DrmDeviceFd) {
    // Try devices in ascending card order.  all_gpus() is already sorted, and
    // preferring the render-primary (which may be a render-only GPU) only adds
    // a failing first attempt on split-GPU machines.
    let gpus = smithay::backend::udev::all_gpus(seat_name).expect("enumerate scanout GPUs");

    for gpu_path in gpus {
        let owned = match session.open(
            &gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        ) {
            Ok(fd) => fd,
            Err(error) => {
                log::warn!("Cannot open scanout GPU {}: {error}", gpu_path.display());
                continue;
            }
        };
        let fd = DrmDeviceFd::new(DeviceFd::from(owned));
        // Prefer disabling connectors for a clean slate, but fall back to
        // keeping the current state when we lack DRM master (common on an
        // unprivileged tty session where the disable commit is rejected).
        let mut disable_connectors = true;
        loop {
            match DrmDevice::new(fd.clone(), disable_connectors) {
                Ok((device, notifier)) => return (gpu_path, device, notifier, fd),
                Err(error) => {
                    if disable_connectors {
                        log::warn!(
                            "Cannot reset scanout GPU {} with connectors disabled ({error}); \
                             retrying without disabling connectors",
                            gpu_path.display()
                        );
                        disable_connectors = false;
                        continue;
                    }
                    log::warn!(
                        "Cannot initialize scanout GPU {}: {error}",
                        gpu_path.display()
                    );
                    let session_fd = fd.device_fd();
                    drop(fd);
                    if let Ok(fd) = TryInto::<std::os::fd::OwnedFd>::try_into(session_fd) {
                        let _ = session.close(fd);
                    }
                    break;
                }
            }
        }
    }
    panic!(
        "no usable DRM scanout (KMS) GPU found among the available GPUs. \
         instantWM's DRM backend requires a device that supports mode-setting. \
         Try `instantwm --backend nested` or `--backend x11` instead."
    );
}
