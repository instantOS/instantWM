//! Keyboard-focus policy for layer-shell surfaces.
//!
//! Layer shell separates surfaces that may be focused by a user action from
//! upper-layer surfaces that automatically take exclusive keyboard focus.
//! Keep that interpretation here so commit, keyboard, pointer, and touch paths
//! cannot drift apart.

use crate::backend::wayland::stacking::{OutputScenePolicy, SceneBand};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, SERIAL_COUNTER, Serial};
use smithay::wayland::compositor::with_states;
use smithay::wayland::seat::WaylandFocus;
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

    fn for_mapped_root(surface: &WlSurface) -> Option<Self> {
        with_renderer_surface_state(surface, |state| state.buffer().is_some())
            .unwrap_or(false)
            .then(|| Self::for_root_surface(surface))
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
        self.native.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            let owner = map.layer_for_surface(surface, WindowSurfaceType::ALL)?;
            OutputScenePolicy::for_output(&self.wm.core.state, output).layer_band(owner.layer())?;
            LayerKeyboardPolicy::for_mapped_root(owner.wl_surface())
        })
    }

    /// Apply keyboard focus to a layer surface according to protocol policy.
    pub(crate) fn focus_layer_keyboard(
        &mut self,
        surface: &WlSurface,
        serial: Serial,
        request: LayerFocusRequest,
    ) -> bool {
        let Some(root) = self.native.layer_root_surface(surface) else {
            return false;
        };
        let Some(policy) = self.layer_keyboard_policy(&root) else {
            return false;
        };
        if !policy.allows(request) {
            return false;
        }
        // An exclusive surface in a higher layer owns the seat even when a
        // lower surface commits or is clicked on another output.
        if self
            .topmost_exclusive_layer_surface()
            .map(|(_, policy)| policy)
            .is_some_and(|exclusive| {
                SceneBand::for_layer(exclusive.layer) < SceneBand::for_layer(policy.layer)
            })
        {
            return false;
        }
        self.apply_layer_keyboard_focus(root, policy, serial)
    }

    /// Apply a target whose mapping, priority and interactivity were resolved
    /// by the caller during this dispatch; no protocol dispatch occurs between.
    fn apply_layer_keyboard_focus(
        &mut self,
        root: WlSurface,
        policy: LayerKeyboardPolicy,
        serial: Serial,
    ) -> bool {
        if self.native.seat.get_keyboard().is_none() {
            return false;
        }
        // Session-lock aware: while locked this re-anchors to the lock
        // surface instead of the layer surface.
        let target = KeyboardFocusTarget::WlSurface(root);
        if policy.takes_automatic_focus() {
            let keyboard = self.native.keyboard.clone();
            if keyboard
                .grab_start_data()
                .is_some_and(|start| start.focus.as_ref().is_some_and(|focus| focus != &target))
            {
                keyboard.unset_grab(self);
            }
        }
        self.set_keyboard_focus(Some(target), serial);
        true
    }

    /// Whether an upper exclusive layer currently owns the seat keyboard.
    pub(crate) fn exclusive_layer_has_keyboard_focus(&self) -> bool {
        self.focused_layer_keyboard_policy()
            .is_some_and(LayerKeyboardPolicy::takes_automatic_focus)
    }

    fn focused_layer_keyboard_policy(&self) -> Option<LayerKeyboardPolicy> {
        let focus = self.native.seat.get_keyboard()?.current_focus()?;
        self.layer_keyboard_policy(focus.wl_surface()?.as_ref())
    }

    /// Preserve a valid layer focus or recover to the highest-priority
    /// remaining exclusive surface.
    ///
    /// Returns `false` when no mapped layer surface should retain focus and
    /// the caller should restore the WM-selected window.
    pub(crate) fn preserve_layer_keyboard_focus(&mut self) -> bool {
        let current = self
            .focused_layer_keyboard_policy()
            .filter(|policy| policy.accepts_user_focus());
        let candidate = self.topmost_exclusive_layer_surface();
        if let Some((surface, policy)) = candidate {
            if current.is_some_and(|current| {
                current.takes_automatic_focus()
                    && SceneBand::for_layer(current.layer) <= SceneBand::for_layer(policy.layer)
            }) {
                return true;
            }
            return self.apply_layer_keyboard_focus(surface, policy, SERIAL_COUNTER.next_serial());
        }
        current.is_some()
    }

    /// Reconcile mapping, fullscreen/tag visibility and keyboard interactivity.
    /// A live resource may have stopped being a valid keyboard target.
    pub(crate) fn reconcile_layer_keyboard_focus(&mut self) {
        if self.native.is_locked() {
            return;
        }
        let invalid_layer_focus = self
            .native
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .is_some_and(|focus| {
                let Some(surface) = focus.wl_surface() else {
                    return false;
                };
                self.native.layer_root_surface(surface.as_ref()).is_some()
                    && !self
                        .layer_keyboard_policy(surface.as_ref())
                        .is_some_and(|policy| policy.accepts_user_focus())
            });
        if invalid_layer_focus {
            self.clear_seat_focus();
        }
        if !self.preserve_layer_keyboard_focus()
            && self
                .native
                .seat
                .get_keyboard()
                .and_then(|keyboard| keyboard.current_focus())
                .is_none_or(|focus| !focus.alive())
        {
            self.restore_focus_after_overlay();
        }
    }

    fn topmost_exclusive_layer_surface(&self) -> Option<(WlSurface, LayerKeyboardPolicy)> {
        let mut candidate = None;
        // Inspect outputs once. Later outputs win ties, while Overlay always
        // outranks Top regardless of output insertion order.
        for output in self.native.space.outputs() {
            let map = layer_map_for_output(output);
            for layer in [Layer::Overlay, Layer::Top] {
                if candidate
                    .as_ref()
                    .is_some_and(|(_, policy): &(_, LayerKeyboardPolicy)| {
                        SceneBand::for_layer(policy.layer) < SceneBand::for_layer(layer)
                    })
                {
                    continue;
                }
                if let Some(target) = map.layers_on(layer).rev().find_map(|surface| {
                    let policy = LayerKeyboardPolicy::for_mapped_root(surface.wl_surface())?;
                    policy
                        .takes_automatic_focus()
                        .then(|| (surface.wl_surface().clone(), policy))
                }) {
                    // Overlay is always visible; only Top needs fullscreen policy.
                    if layer == Layer::Top
                        && OutputScenePolicy::for_output(&self.wm.core.state, output)
                            .layer_band(layer)
                            .is_none()
                    {
                        continue;
                    }
                    candidate = Some(target);
                }
            }
        }
        candidate
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
