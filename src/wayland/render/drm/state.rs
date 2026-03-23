//! Shared state for the DRM backend.

use std::collections::{HashMap, HashSet};

use smithay::backend::allocator::gbm::GbmAllocator;
use smithay::backend::drm::{DrmDeviceFd, GbmBufferedSurface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::output::Output;
use smithay::reexports::drm::control::crtc;

use crate::globals::Globals;
use crate::types::Rect;

pub const DEFAULT_SCREEN_WIDTH: i32 = 1280;
pub const DEFAULT_SCREEN_HEIGHT: i32 = 800;
pub const CURSOR_SIZE: u32 = 24;

pub struct OutputHitRegion {
    pub crtc: crtc::Handle,
    pub x_offset: i32,
    pub width: i32,
}

pub struct OutputSurfaceEntry {
    pub crtc: crtc::Handle,
    pub surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    pub x_offset: i32,
    pub width: i32,
    pub height: i32,
    pub frame_clock: crate::frame_clock::FrameClock,
    pub last_render_duration: std::time::Duration,
    /// Whether VRR (Variable Refresh Rate) is active on this output.
    pub vrr_active: bool,
}

pub struct SharedDrmState {
    pub session_active: bool,
    pub render_flags: HashMap<crtc::Handle, bool>,
    pub total_width: i32,
    pub total_height: i32,
    pub completed_crtcs: Vec<crtc::Handle>,
    pub pending_crtcs: HashSet<crtc::Handle>,
    pub output_hit_regions: Vec<OutputHitRegion>,
    /// Presentation times from VBlank events, keyed by CRTC
    pub presentation_times: HashMap<crtc::Handle, std::time::Duration>,
    /// CRTCs that have VRR enabled — these should not be re-marked dirty
    /// on VBlank since they only present when a new frame is submitted.
    pub vrr_crtcs: HashSet<crtc::Handle>,
}

impl SharedDrmState {
    pub fn new(total_width: i32, total_height: i32) -> Self {
        Self {
            session_active: true,
            render_flags: HashMap::new(),
            total_width,
            total_height,
            completed_crtcs: Vec::new(),
            pending_crtcs: HashSet::new(),
            output_hit_regions: Vec::new(),
            presentation_times: HashMap::new(),
            vrr_crtcs: HashSet::new(),
        }
    }

    pub fn mark_all_dirty(&mut self) {
        for flag in self.render_flags.values_mut() {
            *flag = true;
        }
    }

    pub fn mark_dirty(&mut self, crtc: crtc::Handle) {
        if let Some(flag) = self.render_flags.get_mut(&crtc) {
            *flag = true;
        }
    }

    pub fn mark_pointer_output_dirty(&mut self, px: i32) {
        for entry in &self.output_hit_regions {
            if px >= entry.x_offset && px < entry.x_offset + entry.width {
                self.mark_dirty(entry.crtc);
                return;
            }
        }
        self.mark_all_dirty();
    }
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
        mon.current_tag = 1;
        mon.prev_tag = 1;
        mon.tag_set = [1, 1];
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
