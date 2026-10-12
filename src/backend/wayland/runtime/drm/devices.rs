//! DRM device ownership. Rendering remains on the primary GPU; each scanout
//! device owns its KMS manager and notifier, including display-only EVDI nodes.
//! The allocation strategy follows niri's display-only device path:
//! https://github.com/niri-wm/niri/blob/90af637e466687ebce85d5e037c83871a1c0376a/src/backend/tty.rs

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::{Session, libseat::LibSeatSession};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;

use super::{DrmRuntimeEvent, setup_drm_vblank_handler};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::render::drm::{
    DrmDeviceId, ManagedDrmOutputManager, create_output_manager,
};

pub(super) const PRIMARY_DEVICE: DrmDeviceId = DrmDeviceId(0);

pub(super) struct OutputDevice {
    pub path: PathBuf,
    pub manager: ManagedDrmOutputManager,
    pub devnum: libc::dev_t,
    pub token: Option<RegistrationToken>,
}

pub(super) struct DeviceChanges {
    pub removed: Vec<DrmDeviceId>,
    pub added: Vec<PathBuf>,
}

pub(super) struct DrmDevices {
    pub devices: BTreeMap<DrmDeviceId, OutputDevice>,
    primary_gbm: GbmDevice<DrmDeviceFd>,
    session: LibSeatSession,
    seat: String,
    next_id: u64,
}

impl DrmDevices {
    pub fn new(
        path: PathBuf,
        manager: ManagedDrmOutputManager,
        primary_gbm: GbmDevice<DrmDeviceFd>,
        session: LibSeatSession,
        seat: String,
    ) -> Self {
        Self {
            devices: [(
                PRIMARY_DEVICE,
                OutputDevice {
                    devnum: manager.device().device_id(),
                    path,
                    manager,
                    token: None,
                },
            )]
            .into(),
            primary_gbm,
            session,
            seat,
            next_id: 1,
        }
    }

    pub fn manager(&mut self, device: DrmDeviceId) -> &mut ManagedDrmOutputManager {
        &mut self
            .devices
            .get_mut(&device)
            .expect("output outlived its DRM device")
            .manager
    }

    /// Plan from one enumeration; callers retire removed outputs and devices
    /// before opening replacements, including when a card number is reused.
    pub fn discover(
        &self,
        explicitly_removed: &std::collections::HashSet<libc::dev_t>,
    ) -> std::io::Result<DeviceChanges> {
        let paths = smithay::backend::udev::all_gpus(&self.seat)?;
        Ok(plan_device_changes(
            self.devices
                .iter()
                .map(|(&id, device)| (id, device.path.as_path(), device.devnum)),
            paths,
            explicitly_removed,
        ))
    }

    pub fn add(
        &mut self,
        path: PathBuf,
        renderer: &GlesRenderer,
        handle: &LoopHandle<'_, WaylandState>,
        tx: &mpsc::Sender<DrmRuntimeEvent>,
    ) -> anyhow::Result<DrmDeviceId> {
        // Never reuse an identity, even if opening fails or a card number is
        // recycled. Queued events from old lifetimes remain harmless.
        let id = DrmDeviceId(self.next_id);
        self.next_id += 1;
        let owned = self.session.open(
            &path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )?;
        let fd = DrmDeviceFd::new(DeviceFd::from(owned));
        // Keep a DeviceFd for session.close(), after all DRM/GBM references die.
        let session_fd = fd.device_fd();
        let result = (|| {
            let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
            let gbm = GbmDevice::new(fd.clone())?;
            let manager = create_output_manager(drm, renderer, &self.primary_gbm, &gbm);
            let token = setup_drm_vblank_handler(handle, notifier, id, tx.clone())?;
            log::info!(
                "Added scanout device {} ({id:?}), rendering on primary GPU",
                path.display()
            );
            self.devices.insert(
                id,
                OutputDevice {
                    devnum: manager.device().device_id(),
                    path,
                    manager,
                    token: Some(token),
                },
            );
            Ok(id)
        })();
        drop(fd);
        if result.is_err() {
            self.close(session_fd);
        }
        result
    }

    /// Outputs must be dropped first, then the notifier, then the manager,
    /// before asking libseat to close its device. Keep this ordering explicit.
    pub fn retire(&mut self, id: DrmDeviceId, handle: &LoopHandle<'_, WaylandState>) {
        if let Some(device) = self.devices.remove(&id) {
            let fd = device.manager.device().device_fd().device_fd();
            if let Some(token) = device.token {
                handle.remove(token);
            }
            log::info!("Removed scanout device {} ({id:?})", device.path.display());
            drop(device.manager);
            self.close(fd);
        }
    }

    fn close(&mut self, fd: DeviceFd) {
        match TryInto::<OwnedFd>::try_into(fd) {
            Ok(fd) => {
                if let Err(error) = self.session.close(fd) {
                    log::warn!("Failed to close DRM session device: {error}");
                }
            }
            Err(_) => log::error!("DRM session device still referenced after teardown"),
        }
    }
}

/// An explicit udev removal wins over an equal path in the next enumeration:
/// that path may already designate a different device by the time we dispatch.
fn plan_device_changes<'a>(
    known: impl Iterator<Item = (DrmDeviceId, &'a Path, libc::dev_t)>,
    paths: Vec<PathBuf>,
    explicitly_removed: &std::collections::HashSet<libc::dev_t>,
) -> DeviceChanges {
    let known: Vec<_> = known.collect();
    let removed: Vec<_> = known
        .iter()
        .filter(|(id, path, devnum)| {
            *id != PRIMARY_DEVICE
                && (!paths.iter().any(|candidate| candidate == path)
                    || explicitly_removed.contains(devnum))
        })
        .map(|(id, _, _)| *id)
        .collect();
    let added = paths
        .into_iter()
        .filter(|path| {
            !known
                .iter()
                .any(|(id, known_path, _)| !removed.contains(id) && path == known_path)
        })
        .collect();
    DeviceChanges { removed, added }
}

#[cfg(test)]
mod tests;
