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

use super::{
    focus::KeyboardFocusTarget,
    state::WaylandState,
};

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
) {
    let mut layer_surface = None;
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
            break;
        }
    }
    if let Some(surface) = layer_surface {
        focus_layer_if_requested(state, &surface);
    }
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

        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let mut map = layer_map_for_output(&output);
            let layers: Vec<_> = map
                .layers()
                .filter(|l| l.wl_surface() == wl_surface)
                .cloned()
                .collect();
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
    }
}
