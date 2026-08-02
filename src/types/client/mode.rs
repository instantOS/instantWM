//! Orthogonal client placement and presentation state.

use super::Client;

/// Persistent placement policy, independent of temporary presentation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ClientPlacement {
    #[default]
    Tiling,
    Floating,
}

/// Whether fullscreen changes geometry or only the protocol-visible state.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum FullscreenKind {
    True,
    Fake,
}

/// Presentation to restore when fullscreen ends.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum RestoredPresentation {
    #[default]
    Normal,
    Maximized,
}

/// Temporary presentation, independent of tiled/floating placement.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum ClientPresentation {
    #[default]
    Normal,
    Maximized,
    Fullscreen {
        kind: FullscreenKind,
        restore: RestoredPresentation,
    },
}

/// Complete window mode with orthogonal placement and presentation axes.
///
/// Keeping these values in one immutable value makes snapshots and IPC simple,
/// while preventing a presentation transition from overwriting placement.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    bincode::Encode,
    bincode::Decode,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct ClientMode {
    placement: ClientPlacement,
    presentation: ClientPresentation,
}

impl Default for ClientMode {
    fn default() -> Self {
        Self::tiled()
    }
}

impl ClientMode {
    #[inline]
    pub(crate) const fn tiled() -> Self {
        Self::normal(ClientPlacement::Tiling)
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn floating() -> Self {
        Self::normal(ClientPlacement::Floating)
    }

    #[inline]
    pub(crate) const fn normal(placement: ClientPlacement) -> Self {
        Self {
            placement,
            presentation: ClientPresentation::Normal,
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn maximized(placement: ClientPlacement) -> Self {
        Self {
            placement,
            presentation: ClientPresentation::Maximized,
        }
    }

    #[inline]
    pub const fn presentation(self) -> ClientPresentation {
        self.presentation
    }

    #[inline]
    pub fn is_fullscreen(self) -> bool {
        matches!(self.presentation, ClientPresentation::Fullscreen { .. })
    }

    #[inline]
    pub fn is_true_fullscreen(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Fullscreen {
                kind: FullscreenKind::True,
                ..
            }
        )
    }

    #[inline]
    pub fn is_fake_fullscreen(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Fullscreen {
                kind: FullscreenKind::Fake,
                ..
            }
        )
    }

    /// Whether maximized geometry is the current presentation.
    #[inline]
    pub fn is_maximized(self) -> bool {
        matches!(self.presentation, ClientPresentation::Maximized)
    }

    /// Whether literal client-owned maximization is the current presentation
    /// or the presentation restored after fullscreen.
    ///
    /// This is intentionally not the protocol projection for tiled windows;
    /// that also depends on the monitor's layout presentation.
    #[inline]
    pub fn has_maximized_presentation(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Maximized
                | ClientPresentation::Fullscreen {
                    restore: RestoredPresentation::Maximized,
                    ..
                }
        )
    }

    #[inline]
    pub fn is_normal_floating(self) -> bool {
        self.placement == ClientPlacement::Floating
            && matches!(self.presentation, ClientPresentation::Normal)
    }

    #[inline]
    pub fn is_normal_tiling(self) -> bool {
        self.placement == ClientPlacement::Tiling
            && matches!(self.presentation, ClientPresentation::Normal)
    }

    #[inline]
    pub fn is_free_positioned(self) -> bool {
        self.is_normal_floating() || self.is_maximized()
    }

    #[inline]
    pub const fn placement(self) -> ClientPlacement {
        self.placement
    }

    /// Replace the persistent placement mode without discarding a temporary
    /// presentation mode.
    ///
    /// Rules and policy refreshes may change whether a client should restore
    /// to tiling or floating, but they do not own fullscreen/maximized state.
    #[inline]
    pub(crate) fn with_placement(self, placement: ClientPlacement) -> Self {
        Self { placement, ..self }
    }

    #[inline]
    pub(crate) fn as_fullscreen(self) -> Self {
        self.with_fullscreen_kind(FullscreenKind::True)
    }

    #[inline]
    pub(crate) fn as_fake_fullscreen(self) -> Self {
        self.with_fullscreen_kind(FullscreenKind::Fake)
    }

    fn with_fullscreen_kind(self, kind: FullscreenKind) -> Self {
        let restore = match self.presentation {
            ClientPresentation::Maximized => RestoredPresentation::Maximized,
            ClientPresentation::Fullscreen { restore, .. } => restore,
            ClientPresentation::Normal => RestoredPresentation::Normal,
        };
        Self {
            presentation: ClientPresentation::Fullscreen { kind, restore },
            ..self
        }
    }

    #[inline]
    pub(crate) fn with_maximized(self, maximized: bool) -> Self {
        let presentation = match (self.presentation, maximized) {
            (ClientPresentation::Fullscreen { kind, .. }, true) => ClientPresentation::Fullscreen {
                kind,
                restore: RestoredPresentation::Maximized,
            },
            (
                ClientPresentation::Fullscreen {
                    kind,
                    restore: RestoredPresentation::Maximized,
                },
                false,
            ) => ClientPresentation::Fullscreen {
                kind,
                restore: RestoredPresentation::Normal,
            },
            (fullscreen @ ClientPresentation::Fullscreen { .. }, false) => fullscreen,
            (_, true) => ClientPresentation::Maximized,
            (ClientPresentation::Maximized, false) => ClientPresentation::Normal,
            (presentation, false) => presentation,
        };
        Self {
            presentation,
            ..self
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn as_maximized(self) -> Self {
        self.with_maximized(true)
    }

    #[inline]
    pub(crate) fn restored(self) -> Self {
        let presentation = match self.presentation {
            ClientPresentation::Fullscreen {
                restore: RestoredPresentation::Normal,
                ..
            }
            | ClientPresentation::Maximized => ClientPresentation::Normal,
            ClientPresentation::Fullscreen {
                restore: RestoredPresentation::Maximized,
                ..
            } => ClientPresentation::Maximized,
            ClientPresentation::Normal => ClientPresentation::Normal,
        };
        Self {
            presentation,
            ..self
        }
    }
}

impl Client {
    /// Current placement/presentation state.
    #[inline]
    pub fn mode(&self) -> ClientMode {
        self.mode
    }

    /// Persistent placement policy, independent of temporary presentation.
    #[inline]
    pub fn placement(&self) -> ClientPlacement {
        self.mode.placement()
    }

    /// Change the persistent tiled/floating policy while preserving any
    /// temporary fullscreen or maximized presentation.
    #[inline]
    pub(crate) fn set_placement(&mut self, placement: ClientPlacement) {
        self.mode = self.mode.with_placement(placement);
    }

    /// Enter true fullscreen while remembering the current base placement.
    #[inline]
    pub(crate) fn enter_fullscreen(&mut self) {
        self.mode = self.mode.as_fullscreen();
    }

    /// Enter fake fullscreen while remembering the current base placement.
    #[inline]
    pub(crate) fn enter_fake_fullscreen(&mut self) {
        self.mode = self.mode.as_fake_fullscreen();
    }

    /// Set maximized presentation without changing persistent placement.
    #[inline]
    pub(crate) fn set_maximized_presentation(&mut self, maximized: bool) {
        self.mode = self.mode.with_maximized(maximized);
    }

    /// Leave a temporary presentation mode and restore its base placement.
    #[inline]
    pub(crate) fn restore_mode(&mut self) {
        self.mode = self.mode.restored();
    }

    /// Construct otherwise unreachable states in unit tests without exposing a
    /// production escape hatch around the transition API.
    #[cfg(test)]
    pub(crate) fn set_mode_for_test(&mut self, mode: ClientMode) {
        self.mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientMode, ClientPlacement};

    #[test]
    fn changing_placement_preserves_temporary_presentation() {
        for mode in [
            ClientMode::tiled().as_fullscreen(),
            ClientMode::tiled().as_fake_fullscreen(),
            ClientMode::tiled().as_maximized(),
        ] {
            let changed = mode.with_placement(ClientPlacement::Floating);
            assert_eq!(changed.presentation(), mode.presentation());
            assert_eq!(changed.restored(), ClientMode::floating());
        }
    }
}
