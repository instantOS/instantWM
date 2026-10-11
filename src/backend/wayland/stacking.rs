//! Scene order shared by rendering, pointer/touch targeting, and layer focus.

use smithay::output::Output;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::core_state::CoreState;

/// Front-to-back scene bands. Session lock replaces the entire client scene;
/// cursors and drag icons are composed separately above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SceneBand {
    SafetyUi,
    OverlayLayer,
    WindowOverlay,
    TopLayer,
    Bar,
    Borders,
    Windows,
    LowerLayers,
}

impl SceneBand {
    pub(crate) const FRONT_TO_BACK: [Self; 8] = [
        Self::SafetyUi,
        Self::OverlayLayer,
        Self::WindowOverlay,
        Self::TopLayer,
        Self::Bar,
        Self::Borders,
        Self::Windows,
        Self::LowerLayers,
    ];

    pub(crate) fn for_layer(layer: Layer) -> Self {
        match layer {
            Layer::Overlay => Self::OverlayLayer,
            Layer::Top => Self::TopLayer,
            Layer::Bottom | Layer::Background => Self::LowerLayers,
        }
    }

    pub(crate) fn above_bar(self) -> bool {
        self < Self::Bar
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputScenePolicy {
    fullscreen: bool,
}

impl OutputScenePolicy {
    pub(crate) fn for_output(core: &CoreState, output: &Output) -> Self {
        let name = output.name();
        let fullscreen = core
            .model
            .monitors
            .iter_all()
            .find(|m| m.name == name)
            .is_some_and(|monitor| {
                !core.model.is_overview_active_on(monitor)
                    && monitor.iter_clients().any(|(_, client)| {
                        client.mode().is_true_fullscreen()
                            && client.is_visible(monitor.visible_tags())
                    })
            });
        Self { fullscreen }
    }

    /// Overlay is the protocol's escape hatch above fullscreen. Top is for
    /// ordinary panels and is concealed during real fullscreen presentation.
    pub(crate) fn layer_band(self, layer: Layer) -> Option<SceneBand> {
        if layer == Layer::Top && self.fullscreen {
            None
        } else {
            Some(SceneBand::for_layer(layer))
        }
    }

    pub(crate) fn layers(self, band: SceneBand) -> impl Iterator<Item = Layer> {
        [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background]
            .into_iter()
            .filter(move |&layer| self.layer_band(layer) == Some(band))
    }
}
