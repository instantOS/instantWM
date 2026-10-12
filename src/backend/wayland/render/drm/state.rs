//! Shared DRM render state types.

use smithay::backend::allocator::gbm::GbmAllocator;
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager};
use smithay::output::Mode as OutputMode;
use smithay::output::Output;
use smithay::reexports::drm::control::{self, connector, crtc};
use smithay::wayland::dmabuf::DmabufFeedback;

use crate::backend::BackendVrrSupport;
use crate::backend::output::RequestId;
use crate::config::config_toml::VrrMode;
pub const DEFAULT_SCREEN_WIDTH: i32 = 1280;
pub const DEFAULT_SCREEN_HEIGHT: i32 = 800;

pub type DrmAllocator = GbmAllocator<DrmDeviceFd>;
pub type DrmFramebufferExporter = GbmFramebufferExporter<DrmDeviceFd>;
pub type ManagedDrmOutput =
    DrmOutput<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>;
pub type ManagedDrmOutputManager =
    DrmOutputManager<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>;

#[derive(Clone)]
pub struct OutputDmabufFeedback {
    pub render: DmabufFeedback,
    pub scanout: DmabufFeedback,
}

/// A DRM file's lifetime, not its reusable card number or kernel object IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrmDeviceId(pub u64);

/// Kernel object handles are local to a device. The lifetime also rejects
/// completions queued before a device was unplugged and opened again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScanoutId {
    pub device: DrmDeviceId,
    pub crtc: crtc::Handle,
}

pub struct OutputSurfaceEntry {
    pub device: DrmDeviceId,
    pub crtc: crtc::Handle,
    pub surface: Option<
        DrmOutput<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>,
    >,
    pub connector: connector::Handle,
    pub modes: Vec<(OutputMode, control::Mode)>,
    pub output: Output,
    pub dmabuf_feedback: Option<OutputDmabufFeedback>,
    /// Logical rectangle in the space. Stale while the output is a realized
    /// mirror, which is not part of the space.
    pub rect: crate::types::Rect,
    pub vrr_support: BackendVrrSupport,
    pub configured_vrr_mode: VrrMode,
    pub vrr_enabled: bool,
    pub enabled: bool,
    /// Physical DPMS state, independent of logical output enablement.
    pub powered: bool,
    /// A power-on request is acknowledged after its first frame is queued.
    pub pending_power_on: Option<RequestId>,
}

impl OutputSurfaceEntry {
    pub fn scanout(&self) -> ScanoutId {
        ScanoutId {
            device: self.device,
            crtc: self.crtc,
        }
    }
}
