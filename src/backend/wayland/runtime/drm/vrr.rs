//! Per-output variable refresh rate (adaptive sync) policy for the DRM
//! backend.

use crate::backend::BackendVrrSupport;
use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::render::drm::OutputSurfaceEntry;
use crate::config::config_toml::VrrMode;
use crate::wm::Wm;

fn has_pending_screencopy_for_output(state: &WaylandState, output_name: &str) -> bool {
    state
        .runtime
        .pending_screencopies
        .iter()
        .any(|copy| copy.output.name() == output_name)
}

fn auto_vrr_content_is_suitable(wm: &Wm, output_name: &str) -> bool {
    let Some(mon) = wm
        .core
        .model
        .monitors_iter_all()
        .find(|m| m.name == output_name)
    else {
        return false;
    };
    if matches!(
        wm.core.behavior.current_mode,
        crate::core_state::ActiveWmMode::Overview
    ) && wm.core.model.selected_monitor_id() == mon.id()
    {
        return false;
    }

    let selected = mon.selected_tags();
    let mut visible_clients = mon
        .iter_clients(&wm.core.model.clients)
        .filter(|(_, client)| client.is_visible(selected) && !client.is_scratchpad());

    let Some((_, first_client)) = visible_clients.next() else {
        return false;
    };

    if visible_clients.next().is_some() {
        return false;
    }

    first_client.mode().is_true_fullscreen()
}

fn compute_output_vrr_target(wm: &Wm, state: &WaylandState, entry: &OutputSurfaceEntry) -> bool {
    let output_name = entry.output.name();

    match entry.vrr_support {
        BackendVrrSupport::Unsupported => false,
        BackendVrrSupport::RequiresModeset => matches!(entry.configured_vrr_mode, VrrMode::On),
        BackendVrrSupport::Supported => {
            let hard_blocked = state.is_locked()
                || state.has_window_animations_on_output(&entry.output)
                || state.has_active_layout_preview_animation()
                || has_pending_screencopy_for_output(state, &output_name)
                || !state.overlay_windows_for_render(&entry.output).is_empty()
                || !matches!(
                    state.cursor_image_status,
                    smithay::input::pointer::CursorImageStatus::Named(_)
                        | smithay::input::pointer::CursorImageStatus::Hidden
                )
                || state.runtime.dnd_icon.is_some();

            if hard_blocked {
                return false;
            }

            match entry.configured_vrr_mode {
                VrrMode::Off => false,
                VrrMode::On => true,
                VrrMode::Auto => auto_vrr_content_is_suitable(wm, &output_name),
            }
        }
    }
}

pub(super) fn apply_output_vrr_policy(
    wm: &Wm,
    state: &mut WaylandState,
    entry: &mut OutputSurfaceEntry,
) {
    let target = compute_output_vrr_target(wm, state, entry);
    if entry.vrr_enabled == target {
        state.set_output_vrr_enabled(&entry.output.name(), entry.vrr_enabled);
        return;
    }

    match entry
        .surface
        .as_mut()
        .expect("enabled DRM output has a surface")
        .with_compositor(|compositor| compositor.use_vrr(target))
    {
        Ok(()) => {
            entry.vrr_enabled = target;
            state.set_output_vrr_enabled(&entry.output.name(), target);
            log::info!(
                "Output {}: VRR {} (mode: {:?}, support: {:?})",
                entry.output.name(),
                if target { "enabled" } else { "disabled" },
                entry.configured_vrr_mode,
                entry.vrr_support
            );
        }
        Err(err) => {
            state.set_output_vrr_enabled(&entry.output.name(), entry.vrr_enabled);
            log::warn!(
                "Output {}: failed to set VRR {}: {:?}",
                entry.output.name(),
                if target { "on" } else { "off" },
                err
            );
        }
    }
}
