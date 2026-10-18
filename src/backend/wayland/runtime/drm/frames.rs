//! Frame requests, mirror damage propagation, and DRM presentation.

use smithay::reexports::calloop::LoopHandle;
use smithay::wayland::presentation::Refresh;
use std::collections::HashMap;
use std::time::Instant;

use super::{scheduling::DrmLoopState, vrr::apply_output_vrr_policy};
use crate::backend::output::{OutputId, OutputPowerError, OutputPowerMode};
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::input::apply_pending_warp;
use crate::backend::wayland::render::drm::{
    CursorManager, OutputSurfaceEntry, RenderOutcome, ScanoutId, render_drm_output,
};
use crate::backend::wayland::render::scene::build_shared_scene_elements;

pub(super) fn output_refresh(entry: &OutputSurfaceEntry) -> Refresh {
    let period = entry.output.current_mode().and_then(|mode| {
        let refresh = u64::try_from(mode.refresh).ok()?;
        (refresh > 0).then(|| std::time::Duration::from_nanos(1_000_000_000_000u64 / refresh))
    });

    match (entry.vrr_enabled, period) {
        (true, Some(period)) => Refresh::variable(period),
        (false, Some(period)) => Refresh::fixed(period),
        (_, None) => Refresh::Unknown,
    }
}

/// Promote compositor-side redraw requests into DRM output dirties.
pub(super) fn process_commit_redraws(
    state: &mut WaylandState,
    loop_state: &mut DrmLoopState,
    output_surfaces: &[OutputSurfaceEntry],
) {
    use crate::backend::wayland::compositor::PendingRenderTargets;

    match state.native.take_render_targets() {
        PendingRenderTargets::None => {}
        PendingRenderTargets::All => loop_state.mark_all_dirty(),
        PendingRenderTargets::Outputs(outputs) => {
            for entry in output_surfaces {
                if outputs.contains(&entry.output.name()) {
                    loop_state.mark_dirty(entry.scanout());
                }
            }
        }
    }
}

/// Service frame-callback-only commits without forcing a DRM render.
pub(super) fn process_frame_callback_requests(
    state: &mut WaylandState,
    loop_handle: &LoopHandle<'_, WaylandState>,
    loop_state: &DrmLoopState,
    output_surfaces: &[OutputSurfaceEntry],
    start_time: Instant,
) {
    use crate::backend::wayland::compositor::PendingRenderTargets;

    let targets = state.native.take_frame_callback_targets();
    for entry in output_surfaces.iter().filter(|entry| entry.enabled) {
        let targeted = match &targets {
            PendingRenderTargets::None => false,
            PendingRenderTargets::All => true,
            PendingRenderTargets::Outputs(outputs) => outputs.contains(&entry.output.name()),
        };
        if targeted {
            loop_state.presentation_scheduler.arm_callbacks(
                entry.scanout(),
                loop_handle,
                &entry.output,
                start_time,
            );
        }
    }
}

/// Apply compositor-side cursor warp.
pub(super) fn process_cursor_warp(
    state: &mut WaylandState,
    pointer_handle: &smithay::input::pointer::PointerHandle<WaylandState>,
    keyboard_handle: &smithay::input::keyboard::KeyboardHandle<WaylandState>,
    loop_state: &mut DrmLoopState,
) {
    if apply_pending_warp(state, pointer_handle, keyboard_handle) {
        loop_state.mark_all_dirty();
    }
}

/// A realized mirror shows its source's scene, so it must redraw whenever
/// its source does. Every damage path targets the source, which is the output
/// in the space.
fn propagate_mirror_render_flags(
    state: &WaylandState,
    output_surfaces: &[OutputSurfaceEntry],
    render_flags: &mut HashMap<ScanoutId, bool>,
) {
    if state.native.runtime.realized_mirrors.is_empty() {
        return;
    }
    for entry in output_surfaces {
        let Some(source) = state
            .native
            .runtime
            .realized_mirrors
            .get(&entry.output.name())
        else {
            continue;
        };
        let source_dirty = output_surfaces
            .iter()
            .find(|candidate| candidate.output.name() == *source)
            .and_then(|candidate| render_flags.get(&candidate.scanout()).copied())
            .unwrap_or(false);
        if source_dirty {
            render_flags.insert(entry.scanout(), true);
        }
    }
}

/// Render all outputs that need it.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_outputs(
    state: &mut WaylandState,
    output_surfaces: &mut [OutputSurfaceEntry],
    cursor_manager: &CursorManager,
    loop_handle: &LoopHandle<'_, WaylandState>,
    loop_state: &mut DrmLoopState,
    render_failures: &mut HashMap<ScanoutId, u32>,
    start_time: Instant,
) {
    let mut render_flags = loop_state.take_render_flags();
    propagate_mirror_render_flags(state, output_surfaces, &mut render_flags);
    let session_active = loop_state.session_active;
    let pending_crtcs = loop_state.pending_crtcs.clone();

    if session_active {
        let needs_any_render = output_surfaces.iter().any(|entry| {
            entry.enabled
                && entry.powered
                && !pending_crtcs.contains(&entry.scanout())
                && render_flags.get(&entry.scanout()).copied().unwrap_or(false)
        });
        let shared_scene = if needs_any_render && !state.native.is_locked() {
            Some(build_shared_scene_elements(
                &mut state.wm,
                &mut state.native,
                &mut loop_state.scene_cache,
            ))
        } else {
            None
        };

        for entry in output_surfaces.iter_mut() {
            let needs_render = render_flags.get(&entry.scanout()).copied().unwrap_or(false);
            if !needs_render || !entry.enabled || !entry.powered {
                continue;
            }
            // Don't render if a page flip is already in flight — queue_buffer
            // would fail with EBUSY and leak a swapchain slot.
            if pending_crtcs.contains(&entry.scanout()) {
                // Re-mark as dirty so we render after the VBlank arrives.
                loop_state.mark_dirty(entry.scanout());
                continue;
            }
            apply_output_vrr_policy(state, entry);
            let scene_policy = crate::backend::wayland::stacking::OutputScenePolicy::for_output(
                &state.wm.core.state,
                &state.native.presented_output(&entry.output),
            );
            let graphics = state.graphics.as_mut().expect("DRM graphics initialized");
            let rendered = graphics.with_renderer(|renderer| {
                render_drm_output(
                    &mut state.native,
                    renderer,
                    entry,
                    cursor_manager,
                    start_time,
                    shared_scene.clone(),
                    scene_policy,
                )
            });

            match rendered {
                RenderOutcome::Submitted => {
                    if let Some(id) = entry.pending_power_on.take() {
                        let output = OutputId(entry.output.name());
                        state
                            .native
                            .runtime
                            .output_power_modes
                            .insert(output.0.clone(), OutputPowerMode::On);
                        state
                            .native
                            .runtime
                            .output_power
                            .complete(id, (output, Ok(OutputPowerMode::On)));
                    }
                    loop_state
                        .presentation_scheduler
                        .presentation_submitted(&entry.scanout());
                    loop_state.pending_crtcs.insert(entry.scanout());
                    if let Some(failed_frames) = render_failures.remove(&entry.scanout())
                        && failed_frames >= 3
                    {
                        log::info!(
                            "DRM render recovered on {:?} after {failed_frames} failed frames",
                            entry.scanout()
                        );
                    }
                }
                RenderOutcome::EmptyFrame => {
                    loop_state.presentation_scheduler.arm_callbacks(
                        entry.scanout(),
                        loop_handle,
                        &entry.output,
                        start_time,
                    );
                    render_failures.remove(&entry.scanout());
                    if entry.pending_power_on.is_some() {
                        loop_state.mark_dirty(entry.scanout());
                    }
                }
                RenderOutcome::Failed => {
                    let failed_frames = render_failures.entry(entry.scanout()).or_insert(0);
                    *failed_frames += 1;

                    if *failed_frames == 1 || (*failed_frames).is_multiple_of(60) {
                        log::warn!(
                            "DRM render failed on {:?} (consecutive failures: {})",
                            entry.scanout(),
                            *failed_frames
                        );
                    }

                    if *failed_frames >= 3
                        && let Some(id) = entry.pending_power_on.take()
                    {
                        entry.powered = false;
                        let output = OutputId(entry.output.name());
                        state.native.runtime.output_power.complete(
                            id,
                            (
                                output,
                                Err(OutputPowerError::Backend(
                                    "failed to queue a frame while powering on".to_string(),
                                )),
                            ),
                        );
                        if let Some(surface) = entry.surface.as_ref() {
                            let _ = surface.with_compositor(|compositor| compositor.clear());
                        }
                    } else {
                        loop_state.mark_dirty(entry.scanout());
                    }
                }
            }
        }
    }
    loop_state.taken_render_flags = render_flags;
}
