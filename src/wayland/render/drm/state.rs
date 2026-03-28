//! Shared DRM render state types.

use smithay::backend::allocator::gbm::GbmAllocator;
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager};
use smithay::output::Output;
use smithay::reexports::drm::control::{connector, crtc};

use crate::backend::BackendVrrSupport;
use crate::config::config_toml::VrrMode;
use crate::globals::Globals;
use crate::types::Rect;

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
    pub surface: DrmOutput<DrmAllocator, DrmFramebufferExporter, super::DrmFrameMetadata, DrmDeviceFd>,
    pub output: Output,
    pub x_offset: i32,
    pub width: i32,
    pub height: i32,
    pub vrr_support: BackendVrrSupport,
    pub configured_vrr_mode: VrrMode,
    pub vrr_enabled: bool,
}

pub fn sync_monitors_from_outputs_vec(g: &mut Globals, surfaces: &[super::OutputSurfaceEntry]) {
    g.monitors.clear();
    let tag_template = g.cfg.tag_template.clone();

    for (i, surface) in surfaces.iter().enumerate() {
        let x = surface.x_offset;
        let y = 0i32;
        let w = surface.width;
        let h = surface.height;

        let mut mon = crate::types::Monitor::new_with_values(
            g.cfg.mfact,
            g.cfg.nmaster,
            g.cfg.show_bar,
            g.cfg.top_bar,
        );
        mon.num = i as i32;
        mon.monitor_rect = Rect { x, y, w, h };
        mon.work_rect = Rect { x, y, w, h };
        mon.current_tag = Some(1);
        mon.prev_tag = Some(1);
        mon.tag_set = [
            crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY),
            crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY),
        ];
        mon.init_tags(&tag_template);
        mon.update_bar_position(g.cfg.bar_height);
        g.monitors.push(mon);
    }

    g.cfg.screen_width = surfaces
        .iter()
        .map(|s| s.x_offset + s.width)
        .max()
        .unwrap_or(DEFAULT_SCREEN_WIDTH);
    g.cfg.screen_height = surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);

    if g.monitors.is_empty() {
        let mut mon = crate::types::Monitor::new_with_values(
            g.cfg.mfact,
            g.cfg.nmaster,
            g.cfg.show_bar,
            g.cfg.top_bar,
        );
        mon.monitor_rect = Rect {
            x: 0,
            y: 0,
            w: DEFAULT_SCREEN_WIDTH,
            h: DEFAULT_SCREEN_HEIGHT,
        };
        mon.work_rect = Rect {
            x: 0,
            y: 0,
            w: DEFAULT_SCREEN_WIDTH,
            h: DEFAULT_SCREEN_HEIGHT,
        };
        mon.init_tags(&tag_template);
        mon.update_bar_position(g.cfg.bar_height);
        g.monitors.push(mon);
    }

    for (i, mon) in g.monitors.iter_mut() {
        mon.num = i as i32;
    }

    if g.selected_monitor_id() >= g.monitors.count() {
        g.set_selected_monitor(0);
    }
}
