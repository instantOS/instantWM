//! Keyboard-focus policy for layer-shell surfaces.
//!
//! Layer shell separates surfaces that may be focused by a user action from
//! upper-layer surfaces that automatically take exclusive keyboard focus.
//! Keep that interpretation here so commit, keyboard, pointer, and touch paths
//! cannot drift apart.

use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{SERIAL_COUNTER, Serial};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer, LayerSurfaceCachedState};

use super::super::{KeyboardFocusTarget, WaylandState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayerFocusRequest {
    Automatic,
    UserInteraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayerKeyboardPolicy {
    layer: Layer,
    interactivity: KeyboardInteractivity,
}

impl LayerKeyboardPolicy {
    fn for_root_surface(surface: &WlSurface) -> Self {
        with_states(surface, |states| {
            let mut cached_state = states.cached_state.get::<LayerSurfaceCachedState>();
            let state = cached_state.current();
            Self {
                layer: state.layer,
                interactivity: state.keyboard_interactivity,
            }
        })
    }

    /// Whether a compositor-mediated user action may focus the surface.
    pub(crate) fn accepts_user_focus(self) -> bool {
        self.interactivity != KeyboardInteractivity::None
    }

    /// Whether the surface automatically owns keyboard focus while mapped.
    pub(crate) fn takes_automatic_focus(self) -> bool {
        self.interactivity == KeyboardInteractivity::Exclusive
            && matches!(self.layer, Layer::Overlay | Layer::Top)
    }

    /// Whether WM shortcuts must be withheld while this surface is focused.
    pub(crate) fn suppresses_wm_shortcuts(self) -> bool {
        self.interactivity == KeyboardInteractivity::Exclusive
    }

    fn allows(self, request: LayerFocusRequest) -> bool {
        match request {
            LayerFocusRequest::Automatic => self.takes_automatic_focus(),
            LayerFocusRequest::UserInteraction => self.accepts_user_focus(),
        }
    }
}

impl WaylandState {
    /// Resolve keyboard policy through the mapped layer owner of any surface
    /// in its tree, including popups and subsurfaces.
    pub(crate) fn layer_keyboard_policy(&self, surface: &WlSurface) -> Option<LayerKeyboardPolicy> {
        self.layer_root_surface(surface)
            .map(|root| LayerKeyboardPolicy::for_root_surface(&root))
    }

    /// Apply keyboard focus to a layer surface according to protocol policy.
    pub(crate) fn focus_layer_keyboard(
        &mut self,
        surface: &WlSurface,
        serial: Serial,
        request: LayerFocusRequest,
    ) -> bool {
        let Some(root) = self.layer_root_surface(surface) else {
            return false;
        };
        if !LayerKeyboardPolicy::for_root_surface(&root).allows(request) {
            return false;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        keyboard.set_focus(self, Some(KeyboardFocusTarget::WlSurface(root)), serial);
        true
    }

    /// Whether an upper exclusive layer currently owns the seat keyboard.
    pub(crate) fn exclusive_layer_has_keyboard_focus(&self) -> bool {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focus| {
                let KeyboardFocusTarget::WlSurface(surface) = focus else {
                    return false;
                };
                self.layer_keyboard_policy(&surface)
                    .is_some_and(LayerKeyboardPolicy::takes_automatic_focus)
            })
    }

    /// Preserve a valid layer focus or recover to the highest-priority
    /// remaining exclusive surface.
    ///
    /// Returns `false` when no mapped layer surface should retain focus and
    /// the caller should restore the WM-selected window.
    pub(crate) fn preserve_layer_keyboard_focus(&mut self) -> bool {
        if self.has_mapped_layer_keyboard_focus() {
            return true;
        }

        let Some(surface) = self.topmost_exclusive_layer_surface() else {
            return false;
        };
        self.focus_layer_keyboard(
            &surface,
            SERIAL_COUNTER.next_serial(),
            LayerFocusRequest::Automatic,
        )
    }

    fn has_mapped_layer_keyboard_focus(&self) -> bool {
        let Some(KeyboardFocusTarget::WlSurface(surface)) = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
        else {
            return false;
        };

        self.layer_root_surface(&surface).is_some()
    }

    fn topmost_exclusive_layer_surface(&self) -> Option<WlSurface> {
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        outputs.iter().rev().find_map(|output| {
            let map = layer_map_for_output(output);
            map.layers_on(Layer::Overlay)
                .rev()
                .chain(map.layers_on(Layer::Top).rev())
                .find(|layer| {
                    LayerKeyboardPolicy::for_root_surface(layer.wl_surface())
                        .takes_automatic_focus()
                })
                .map(|layer| layer.wl_surface().clone())
        })
    }

    fn layer_root_surface(&self, surface: &WlSurface) -> Option<WlSurface> {
        self.space.outputs().find_map(|output| {
            layer_map_for_output(output)
                .layer_for_surface(surface, WindowSurfaceType::ALL)
                .map(|layer| layer.wl_surface().clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{LayerFocusRequest, LayerKeyboardPolicy};
    use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};

    fn policy(layer: Layer, interactivity: KeyboardInteractivity) -> LayerKeyboardPolicy {
        LayerKeyboardPolicy {
            layer,
            interactivity,
        }
    }

    #[test]
    fn none_never_accepts_keyboard_focus() {
        for layer in [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background] {
            let policy = policy(layer, KeyboardInteractivity::None);
            assert!(!policy.accepts_user_focus());
            assert!(!policy.takes_automatic_focus());
            assert!(!policy.suppresses_wm_shortcuts());
        }
    }

    #[test]
    fn on_demand_requires_user_interaction() {
        for layer in [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background] {
            let policy = policy(layer, KeyboardInteractivity::OnDemand);
            assert!(policy.accepts_user_focus());
            assert!(!policy.takes_automatic_focus());
            assert!(!policy.suppresses_wm_shortcuts());
            assert!(policy.allows(LayerFocusRequest::UserInteraction));
            assert!(!policy.allows(LayerFocusRequest::Automatic));
        }
    }

    #[test]
    fn only_upper_exclusive_layers_take_automatic_focus() {
        for layer in [Layer::Overlay, Layer::Top] {
            let policy = policy(layer, KeyboardInteractivity::Exclusive);
            assert!(policy.takes_automatic_focus());
            assert!(policy.allows(LayerFocusRequest::Automatic));
        }
        for layer in [Layer::Bottom, Layer::Background] {
            let policy = policy(layer, KeyboardInteractivity::Exclusive);
            assert!(!policy.takes_automatic_focus());
            assert!(!policy.allows(LayerFocusRequest::Automatic));
        }
    }

    #[test]
    fn focused_exclusive_surfaces_suppress_wm_shortcuts() {
        for layer in [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background] {
            assert!(policy(layer, KeyboardInteractivity::Exclusive).suppresses_wm_shortcuts());
        }
    }
}
