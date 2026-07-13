use smithay::{
    desktop::{LayerSurface as DesktopLayerSurface, WindowSurfaceType, layer_map_for_output},
    output::Output,
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
            WlrLayerShellState,
        },
    },
};

use super::{focus::KeyboardFocusTarget, state::WaylandState};
use crate::backend::wayland::commands::WmCommand;
use crate::types::Rect;
use crate::wm::Wm;
use std::collections::HashMap;

/// Focus a layer surface if it requests keyboard focus.
fn focus_layer_if_requested(
    state: &mut WaylandState,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) {
    use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, LayerSurfaceCachedState};
    let interactivity = with_states(surface, |states| {
        states
            .cached_state
            .get::<LayerSurfaceCachedState>()
            .current()
            .keyboard_interactivity
    });

    if interactivity == KeyboardInteractivity::None {
        return;
    }

    let serial = SERIAL_COUNTER.next_serial();
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(
            state,
            Some(KeyboardFocusTarget::WlSurface(surface.clone())),
            serial,
        );
    }
}

/// Called from `CompositorHandler::commit` when a layer surface commit is detected.
pub(super) fn handle_layer_commit(
    state: &mut WaylandState,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> Option<Output> {
    let mut layer_surface = None;
    let mut layer_output = None;
    for output in state.space.outputs() {
        let mut map = layer_map_for_output(output);
        if let Some(layer) = map
            .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            .cloned()
        {
            map.arrange();
            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<LayerSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            });
            if !initial_configure_sent {
                layer.layer_surface().send_configure();
            }
            layer_surface = Some(surface.clone());
            layer_output = Some(output.clone());
            break;
        }
    }
    if let Some(surface) = layer_surface {
        focus_layer_if_requested(state, &surface);
        // Exclusive zones may have changed on commit, so re-derive each
        // monitor's `available_rect`.
        state.push_command(WmCommand::SyncLayerExclusiveZones);
        if let Some(output) = layer_output.as_ref() {
            state.request_output_render(output);
        }
    }
    layer_output
}

/// Return the output whose Smithay layer map owns `surface`.
pub(super) fn layer_output_for_surface(
    state: &WaylandState,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> Option<Output> {
    state.space.outputs().find_map(|output| {
        let map = layer_map_for_output(output);
        map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            .is_some()
            .then(|| output.clone())
    })
}

/// Build a map from output name to the global (compositor-space) rectangle
/// that is *not* occupied by exclusive layer-shell surfaces on that output.
///
/// Smithay tracks the non-exclusive zone per output via `layer_map_for_output`.
/// The zone is returned in output-local logical coordinates, so we offset it
/// by the output's position in the `Space` to get global coordinates that
/// match `Monitor::monitor_rect`.
pub fn collect_available_rects(state: &WaylandState) -> HashMap<String, Rect> {
    let mut out = HashMap::new();
    for output in state.space.outputs() {
        let output_loc = state
            .space
            .output_geometry(output)
            .map(|geo| geo.loc)
            .unwrap_or_default();
        let zone = layer_map_for_output(output).non_exclusive_zone();
        let rect = Rect {
            x: output_loc.x + zone.loc.x,
            y: output_loc.y + zone.loc.y,
            w: zone.size.w.max(1),
            h: zone.size.h.max(1),
        };
        out.insert(output.name(), rect);
    }
    out
}

/// Apply the latest layer-shell non-exclusive zones to each `Monitor`.
///
/// Returns `true` if any monitor's `available_rect` changed (caller should
/// re-arrange and redraw).
pub fn apply_available_rects(wm: &mut Wm, state: &WaylandState) -> bool {
    let rects = collect_available_rects(state);
    let mut any_changed = false;
    for mon in wm.core.model.monitors.iter_all_mut() {
        let Some(&new_rect) = rects.get(&mon.name) else {
            // No matching output (e.g. monitor was just removed or named
            // differently). Leave it alone; the next monitor refresh will
            // sort it out.
            continue;
        };
        if new_rect == mon.available_rect {
            continue;
        }
        mon.set_available_rect(new_rect);
        let bar_height = mon.bar_height;
        mon.update_bar_position(bar_height);
        any_changed = true;
    }
    any_changed
}

impl WlrLayerShellHandler for WaylandState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.wlr_layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let layer_surface = DesktopLayerSurface::new(surface, namespace);
        let target_output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned());
        let Some(target_output) = target_output else {
            return;
        };
        let mut map = layer_map_for_output(&target_output);
        let _ = map.map_layer(&layer_surface);
        map.arrange();
        drop(map);
        self.push_command(WmCommand::SyncLayerExclusiveZones);
        self.request_output_render(&target_output);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let wl_surface = surface.wl_surface();

        // Check if the keyboard is focused on this layer surface before we destroy it
        let keyboard_focused_on_layer = self
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| {
                if let KeyboardFocusTarget::WlSurface(s) = focus {
                    s == *wl_surface
                } else {
                    false
                }
            });

        let mut affected_outputs = Vec::new();
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let layers: Vec<_> = map
                .layers()
                .filter(|l| l.wl_surface() == wl_surface)
                .cloned()
                .collect();
            if !layers.is_empty() {
                affected_outputs.push(output.clone());
            }
            for layer in layers {
                map.unmap_layer(&layer);
            }
        }

        // If the keyboard was focused on this layer surface, clear seat focus
        // and restore it to the WM's selected window.
        if keyboard_focused_on_layer {
            self.clear_seat_focus();
        }

        // Restore seat focus to mon.sel (the WM's selected window).
        self.restore_focus_after_overlay();
        // Reclaim the space that this layer surface had exclusively reserved.
        self.push_command(WmCommand::SyncLayerExclusiveZones);
        for output in affected_outputs {
            self.request_output_render(&output);
        }
    }
}
