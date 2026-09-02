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
use crate::backend::output::{OutputPositionSource, OutputPowerRequestId};
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

#[derive(Debug, Clone, Copy)]
pub struct OutputHitRegion {
    pub crtc: crtc::Handle,
    pub x_offset: i32,
    pub width: i32,
}

pub struct OutputSurfaceEntry {
    pub crtc: crtc::Handle,
    pub surface: Option<
        DrmOutput<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>,
    >,
    pub connector: connector::Handle,
    pub modes: Vec<(OutputMode, control::Mode)>,
    pub output: Output,
    pub dmabuf_feedback: Option<OutputDmabufFeedback>,
    pub rect: crate::types::Rect,
    pub position_source: OutputPositionSource,
    pub vrr_support: BackendVrrSupport,
    pub configured_vrr_mode: VrrMode,
    pub vrr_enabled: bool,
    pub enabled: bool,
    /// Physical DPMS state, independent of logical output enablement.
    pub powered: bool,
    /// A power-on request is acknowledged after its first frame is queued.
    pub pending_power_on: Option<OutputPowerRequestId>,
}
