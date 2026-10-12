//! DRM frame scheduling and per-scanout lifecycle state.

use std::collections::{HashMap, HashSet};
use std::mem;

use super::super::engine::PresentationScheduler;
use super::layout::{DrmLayoutState, outputs_at_pointer};
use crate::backend::wayland::render::drm::{OutputSurfaceEntry, ScanoutId};
use crate::backend::wayland::render::scene::SceneCache;

pub(super) struct DrmLoopState {
    pub(super) session_active: bool,
    pub(super) topology_dirty: bool,
    pub(super) removed_devices: HashSet<libc::dev_t>,
    pub(super) render_flags: HashMap<ScanoutId, bool>,
    pub(super) taken_render_flags: HashMap<ScanoutId, bool>,
    pub(super) pending_crtcs: HashSet<ScanoutId>,
    pub(super) presentation_scheduler: PresentationScheduler<ScanoutId>,
    pub(super) presentation_seq: HashMap<ScanoutId, u64>,
    pub(super) last_bar_update_seq: u64,
    pub(super) scene_cache: SceneCache,
}

impl DrmLoopState {
    pub(super) fn new(output_surfaces: &[OutputSurfaceEntry]) -> Self {
        let render_flags = output_surfaces
            .iter()
            .map(|entry| (entry.scanout(), true))
            .collect();
        Self {
            session_active: true,
            topology_dirty: false,
            removed_devices: HashSet::new(),
            render_flags,
            taken_render_flags: HashMap::new(),
            pending_crtcs: HashSet::new(),
            presentation_scheduler: PresentationScheduler::default(),
            presentation_seq: output_surfaces
                .iter()
                .map(|entry| (entry.scanout(), 0))
                .collect(),
            last_bar_update_seq: 0,
            scene_cache: SceneCache::default(),
        }
    }

    /// Remember hotplug events while DRM ownership belongs to another session.
    pub(super) fn take_topology_change(&mut self, changed: bool) -> bool {
        self.topology_dirty |= changed;
        self.session_active && mem::take(&mut self.topology_dirty)
    }

    pub(super) fn mark_all_dirty(&mut self) {
        for flag in self.render_flags.values_mut() {
            *flag = true;
        }
    }

    pub(super) fn mark_dirty(&mut self, crtc: ScanoutId) {
        if let Some(flag) = self.render_flags.get_mut(&crtc) {
            *flag = true;
        }
    }

    pub(super) fn mark_pointer_output_dirty(
        &mut self,
        pointer: crate::types::Point,
        layout: &DrmLayoutState,
    ) {
        let mut matched = false;
        // Outputs may overlap in the layout: every matching head must be
        // dirtied or one of them shows a stale frame.
        for crtc in outputs_at_pointer(&layout.output_hit_regions, pointer) {
            self.mark_dirty(crtc);
            matched = true;
        }
        if !matched {
            // This can be observed briefly while a new output layout is being
            // projected. Redrawing all outputs is the safe recovery path.
            self.mark_all_dirty();
        }
    }

    pub(super) fn take_render_flags(&mut self) -> HashMap<ScanoutId, bool> {
        let mut taken = mem::take(&mut self.taken_render_flags);
        taken.clear();
        for (&crtc, flag) in &mut self.render_flags {
            taken.insert(crtc, *flag);
            *flag = false;
        }
        taken
    }

    pub(super) fn has_renderable_dirty_outputs(&self) -> bool {
        self.render_flags
            .iter()
            .any(|(crtc, &dirty)| dirty && !self.pending_crtcs.contains(crtc))
    }

    pub(super) fn remove_output(&mut self, crtc: ScanoutId) {
        self.render_flags.remove(&crtc);
        self.taken_render_flags.remove(&crtc);
        self.pending_crtcs.remove(&crtc);
        self.presentation_seq.remove(&crtc);
        self.presentation_scheduler.remove_output(&crtc);
    }

    pub(super) fn add_output(&mut self, crtc: ScanoutId) {
        self.render_flags.insert(crtc, true);
        self.presentation_seq.entry(crtc).or_insert(0);
    }
}
