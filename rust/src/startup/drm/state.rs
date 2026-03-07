use std::collections::{HashMap, HashSet};

use smithay::backend::allocator::gbm::GbmAllocator;
use smithay::backend::drm::{DrmDeviceFd, GbmBufferedSurface};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::output::Output;
use smithay::reexports::drm::control::crtc;
use smithay::utils::Point;

use crate::types::Rect;
use crate::wm::Wm;

/// Default screen dimensions when no DRM outputs are detected.
pub const DEFAULT_SCREEN_WIDTH: i32 = 1280;
pub const DEFAULT_SCREEN_HEIGHT: i32 = 800;

/// Nominal cursor size in pixels to load from the xcursor theme.
pub const CURSOR_SIZE: u32 = 24;

pub struct OutputSurfaceEntry {
    pub crtc: crtc::Handle,
    pub surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    pub x_offset: i32,
    pub width: i32,
    pub height: i32,
}

pub struct SharedDrmState {
    pub session_active: bool,
    pub render_flags: HashMap<crtc::Handle, bool>,
    pub pending_crtcs: HashSet<crtc::Handle>,
    pub pointer_location: Point<f64, smithay::utils::Logical>,
    pub total_width: i32,
    pub total_height: i32,
    pub completed_crtcs: Vec<crtc::Handle>,
}

impl SharedDrmState {
    pub fn new(total_width: i32, total_height: i32) -> Self {
        Self {
            session_active: true,
            render_flags: HashMap::new(),
            pending_crtcs: HashSet::new(),
            pointer_location: Point::from(((total_width / 2) as f64, (total_height / 2) as f64)),
            total_width,
            total_height,
            completed_crtcs: Vec::new(),
        }
    }

    pub fn mark_all_dirty(&mut self) {
        for flag in self.render_flags.values_mut() {
            *flag = true;
        }
    }
}

pub fn sync_monitors_from_outputs_vec(wm: &mut Wm, surfaces: &[OutputSurfaceEntry]) {
    wm.g.monitors.clear();
    let tag_template = wm.g.cfg.tag_template.clone();

    for (i, surface) in surfaces.iter().enumerate() {
        let x = surface.x_offset;
        let y = 0i32;
        let w = surface.width;
        let h = surface.height;

        let mut mon = crate::types::Monitor::new_with_values(
            wm.g.cfg.mfact,
            wm.g.cfg.nmaster,
            wm.g.cfg.showbar,
            wm.g.cfg.topbar,
        );
        mon.num = i as i32;
        mon.monitor_rect = Rect { x, y, w, h };
        mon.work_rect = Rect { x, y, w, h };
        mon.current_tag = 1;
        mon.prev_tag = 1;
        mon.tag_set = [1, 1];
        mon.init_tags(&tag_template);
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    wm.g.cfg.screen_width = surfaces
        .iter()
        .map(|s| s.x_offset + s.width)
        .max()
        .unwrap_or(DEFAULT_SCREEN_WIDTH);
    wm.g.cfg.screen_height = surfaces
        .iter()
        .map(|s| s.height)
        .max()
        .unwrap_or(DEFAULT_SCREEN_HEIGHT);

    if wm.g.monitors.is_empty() {
        let mut mon = crate::types::Monitor::new_with_values(
            wm.g.cfg.mfact,
            wm.g.cfg.nmaster,
            wm.g.cfg.showbar,
            wm.g.cfg.topbar,
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
        mon.update_bar_position(wm.g.cfg.bar_height);
        wm.g.monitors.push(mon);
    }

    for (i, mon) in wm.g.monitors.iter_mut() {
        mon.num = i as i32;
    }

    if wm.g.selected_monitor_id() >= wm.g.monitors.count() {
        wm.g.set_selected_monitor(0);
    }
}
