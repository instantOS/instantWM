//! Shared DRM render state types.

use smithay::backend::allocator::gbm::GbmAllocator;
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager};
use smithay::output::Output;
use smithay::reexports::drm::control::{connector, crtc};

use crate::backend::BackendVrrSupport;
use crate::config::config_toml::VrrMode;
pub const DEFAULT_SCREEN_WIDTH: i32 = 1280;
pub const DEFAULT_SCREEN_HEIGHT: i32 = 800;
pub const CURSOR_SIZE: u32 = 24;

pub type DrmAllocator = GbmAllocator<DrmDeviceFd>;
pub type DrmFramebufferExporter = GbmFramebufferExporter<DrmDeviceFd>;
pub type ManagedDrmOutput =
    DrmOutput<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>;
pub type ManagedDrmOutputManager =
    DrmOutputManager<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>;

#[derive(Debug, Clone, Copy)]
pub struct OutputHitRegion {
    pub crtc: crtc::Handle,
    pub x_offset: i32,
    pub width: i32,
}

pub struct OutputSurfaceEntry {
    pub crtc: crtc::Handle,
    pub connector: connector::Handle,
    pub surface:
        DrmOutput<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>,
    pub output: Output,
    pub x_offset: i32,
    pub width: i32,
    pub height: i32,
    pub vrr_support: BackendVrrSupport,
    pub configured_vrr_mode: VrrMode,
    pub vrr_enabled: bool,
}
