//! Render scheduling and target invalidation for WaylandState.

use std::collections::HashSet;
use std::mem;

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::utils::Logical;

use crate::types::Rect;

use super::state::WaylandState;

/// Outputs invalidated by compositor-side work since the last render tick.
///
/// The shared WM continues to identify monitors with its own IDs. This
/// Wayland-local type preserves Smithay's output provenance until the DRM or
/// winit runtime translates it into backend render work.
#[derive(Debug, Default)]
pub enum PendingRenderTargets {
    #[default]
    None,
    Outputs(HashSet<String>),
    All,
}

impl PendingRenderTargets {
    pub(crate) fn invalidate_all(&mut self) -> bool {
        if matches!(self, Self::All) {
            return false;
        }
        *self = Self::All;
        true
    }

    pub(crate) fn invalidate_output(&mut self, output_name: String) -> bool {
        match self {
            Self::None => {
                *self = Self::Outputs(HashSet::from([output_name]));
                true
            }
            Self::Outputs(outputs) => {
                outputs.insert(output_name);
                false
            }
            Self::All => false,
        }
    }
}

impl WaylandState {
    pub fn request_render(&mut self) {
        if !self.runtime.render_targets.invalidate_all() {
            log::debug!("request_render: ping skipped, already dirty");
            return;
        }
        self.ping_render_loop();
    }

    /// Request a redraw for exactly one Smithay output.
    #[inline]
    pub fn request_output_render(&mut self, output: &Output) {
        self.request_output_name_render(output.name());
    }

    pub(super) fn output_can_render(&self, output: &Output) -> bool {
        self.space.outputs().any(|active| active == output)
    }

    /// Drop capture work that can never complete once an output is disabled
    /// or removed. Keeping it would make every later render scan stale work.
    pub fn fail_pending_captures_for_output(&mut self, output: &Output) {
        super::screencopy::fail_pending_screencopies_for_output(
            &mut self.runtime.pending_screencopies,
            output,
        );
        super::image_capture::fail_pending_image_captures_for_output(
            &mut self.runtime.pending_image_captures,
            output,
        );
    }

    /// Request redraws for the outputs Smithay currently associates with a
    /// mapped window.
    ///
    /// Do not use `Space::outputs_for_element` here. Its output membership is
    /// refreshed later in the event-loop tick, while surface commits (notably
    /// short-lived Xwayland override-redirect windows) need to schedule their
    /// redraw immediately.
    pub fn request_window_render(&mut self, window: &Window) {
        let outputs = self.outputs_for_window_geometry(window);
        if outputs.is_empty() {
            self.request_render();
            return;
        }
        self.request_outputs_render(outputs);
    }

    /// Redraw the outputs currently intersected by a mapped window. Unlike
    /// surface-commit scheduling, a fully offscreen lifecycle/animation update
    /// does not need a global fallback.
    pub(crate) fn request_visible_window_render(&mut self, window: &Window) {
        let outputs = self.outputs_for_window_geometry(window);
        self.request_outputs_render(outputs);
    }

    /// Redraw outputs intersected by compositor-owned visual geometry such as
    /// an animated border frame. This geometry may temporarily be larger than
    /// the client surface and therefore cannot be inferred from `Window`.
    pub(crate) fn request_visual_rect_render(&mut self, rect: Rect) {
        let rect = smithay::utils::Rectangle::<i32, Logical>::new(
            (rect.x, rect.y).into(),
            (rect.w.max(1), rect.h.max(1)).into(),
        );
        let outputs = self
            .space
            .outputs()
            .filter(|output| {
                self.space
                    .output_geometry(output)
                    .is_some_and(|output_rect| output_rect.overlaps(rect))
            })
            .cloned()
            .collect();
        self.request_outputs_render(outputs);
    }

    fn request_outputs_render(&mut self, outputs: Vec<Output>) {
        for output in outputs {
            self.request_output_render(&output);
        }
    }

    pub(crate) fn outputs_for_window_geometry(&self, window: &Window) -> Vec<Output> {
        let window_rect = self.space.element_location(window).map(|location| {
            let mut rect = window.bbox_with_popups();
            rect.loc += location - window.geometry().loc;
            rect
        });
        self.space
            .outputs()
            .filter(|output| {
                window_rect.is_some_and(|rect| {
                    self.space
                        .output_geometry(output)
                        .is_some_and(|output_rect| output_rect.overlaps(rect))
                })
            })
            .cloned()
            .collect()
    }

    pub fn has_window_animations_on_output(&self, output: &Output) -> bool {
        let output_rect = self.space.output_geometry(output);
        self.window_animations.iter().any(|(window_id, animation)| {
            if !animation.is_active() {
                return false;
            }
            let surface_overlaps = self
                .find_window(*window_id)
                .is_some_and(|window| self.outputs_for_window_geometry(window).contains(output));
            let frame_overlaps = output_rect.is_some_and(|output_rect| {
                let border_width = self.presented_border_width(*window_id, 0);
                self.displayed_animation_frame(*window_id)
                    .map(|frame| frame.with_borders(border_width))
                    .is_some_and(|frame| {
                        let frame = smithay::utils::Rectangle::<i32, Logical>::new(
                            (frame.x, frame.y).into(),
                            (frame.w.max(1), frame.h.max(1)).into(),
                        );
                        output_rect.overlaps(frame)
                    })
            });
            surface_overlaps || frame_overlaps
        })
    }

    #[inline]
    pub(crate) fn request_output_name_render(&mut self, output_name: String) {
        if self.runtime.render_targets.invalidate_output(output_name) {
            self.ping_render_loop();
        }
    }

    #[inline]
    fn ping_render_loop(&self) {
        if let Some(render_ping) = &self.runtime.render_ping {
            render_ping.ping();
        }
    }

    #[inline]
    pub fn request_frame_callbacks(&mut self) {
        if self.runtime.frame_callback_targets.invalidate_all() {
            self.ping_render_loop();
        }
    }

    pub fn request_output_frame_callbacks(&mut self, output: &Output) {
        if self
            .runtime
            .frame_callback_targets
            .invalidate_output(output.name())
        {
            self.ping_render_loop();
        }
    }

    pub fn request_window_frame_callbacks(&mut self, window: &Window) {
        let outputs = self.outputs_for_window_geometry(window);
        if outputs.is_empty() {
            self.request_frame_callbacks();
            return;
        }
        for output in outputs {
            self.request_output_frame_callbacks(&output);
        }
    }

    #[inline]
    pub fn request_space_sync(&mut self) {
        if self.runtime.space_sync_pending {
            log::debug!("request_space_sync: already pending");
        }
        self.runtime.space_sync_pending = true;
    }

    #[inline]
    pub fn take_space_sync_pending(&mut self) -> bool {
        mem::take(&mut self.runtime.space_sync_pending)
    }

    #[inline]
    pub fn request_bar_redraw(&mut self) {
        self.push_command(super::super::commands::WmCommand::RequestBarRedraw);
        self.request_render();
    }

    #[inline]
    pub fn take_render_targets(&mut self) -> PendingRenderTargets {
        mem::take(&mut self.runtime.render_targets)
    }

    #[inline]
    pub fn take_frame_callback_targets(&mut self) -> PendingRenderTargets {
        mem::take(&mut self.runtime.frame_callback_targets)
    }
}

#[cfg(test)]
mod render_target_tests {
    use super::PendingRenderTargets;

    #[test]
    fn presentation_constraints_are_compositor_managed() {
        let (_event_loop, state) = crate::backend::wayland::compositor::new_event_loop_and_state();
        assert!(state.fifo_manager_state.is_managed());
        assert!(state.commit_timing_manager_state.is_managed());
    }

    #[test]
    fn output_invalidations_accumulate_without_becoming_global() {
        let mut targets = PendingRenderTargets::None;
        assert!(targets.invalidate_output("DP-1".into()));
        assert!(!targets.invalidate_output("HDMI-A-1".into()));

        let PendingRenderTargets::Outputs(outputs) = targets else {
            panic!("expected targeted output invalidation");
        };
        assert_eq!(outputs.len(), 2);
        assert!(outputs.contains("DP-1"));
        assert!(outputs.contains("HDMI-A-1"));
    }

    #[test]
    fn global_invalidation_supersedes_output_targets() {
        let mut targets = PendingRenderTargets::None;
        targets.invalidate_output("DP-1".into());
        assert!(targets.invalidate_all());
        assert!(!targets.invalidate_output("HDMI-A-1".into()));
        assert!(matches!(targets, PendingRenderTargets::All));
    }
}
