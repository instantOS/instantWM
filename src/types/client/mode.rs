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
///
/// Fake fullscreen is split into two states so the user policy cannot be
/// confused with the protocol state currently advertised to the application:
///
/// * [`FakeArmed`](Self::FakeArmed) keeps the per-window policy while the app
///   is still an ordinary window. Nothing is advertised to the client yet.
/// * [`FakeActive`](Self::FakeActive) advertises fullscreen to the app while
///   the window keeps normal layout/floating geometry and stays manageable.
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
    FakeArmed {
        restore: RestoredPresentation,
    },
    RealFullscreen {
        restore: RestoredPresentation,
    },
    FakeActive {
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
    pub fn is_fullscreen(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::RealFullscreen { .. } | ClientPresentation::FakeActive { .. }
        )
    }

    #[inline]
    pub fn is_true_fullscreen(self) -> bool {
        matches!(self.presentation, ClientPresentation::RealFullscreen { .. })
    }

    #[inline]
    pub fn is_fake_fullscreen(self) -> bool {
        matches!(self.presentation, ClientPresentation::FakeActive { .. })
    }

    #[inline]
    pub fn is_fake_armed(self) -> bool {
        matches!(self.presentation, ClientPresentation::FakeArmed { .. })
    }

    /// Whether maximized geometry is the current presentation.
    ///
    /// An armed fake-fullscreen policy does not change the maximized
    /// presentation; it only decides what happens on the next fullscreen
    /// request.
    #[inline]
    pub fn is_maximized(self) -> bool {
        matches!(
            self.presentation,
            ClientPresentation::Maximized
                | ClientPresentation::FakeArmed {
                    restore: RestoredPresentation::Maximized,
                }
        )
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
                | ClientPresentation::FakeArmed {
                    restore: RestoredPresentation::Maximized,
                }
                | ClientPresentation::RealFullscreen {
                    restore: RestoredPresentation::Maximized,
                }
                | ClientPresentation::FakeActive {
                    restore: RestoredPresentation::Maximized,
                }
        )
    }

    /// Whether the window currently occupies normal tiled/floating geometry.
    ///
    /// `FakeActive` deliberately counts as normal geometry: fake fullscreen
    /// changes what the application believes, not which layout slot the window
    /// occupies. `client_size_is_authoritative` separately excludes it because
    /// the application's fullscreen-sized buffers must not drive model
    /// geometry.
    fn has_normal_window_geometry(self) -> bool {
        match self.presentation {
            ClientPresentation::Normal | ClientPresentation::FakeActive { .. } => true,
            ClientPresentation::FakeArmed {
                restore: RestoredPresentation::Normal,
            } => true,
            ClientPresentation::Maximized
            | ClientPresentation::FakeArmed {
                restore: RestoredPresentation::Maximized,
            }
            | ClientPresentation::RealFullscreen { .. } => false,
        }
    }

    #[inline]
    pub fn is_normal_floating(self) -> bool {
        self.placement == ClientPlacement::Floating && self.has_normal_window_geometry()
    }

    #[inline]
    pub fn is_normal_tiling(self) -> bool {
        self.placement == ClientPlacement::Tiling && self.has_normal_window_geometry()
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

    /// The presentation the client should return to when a fullscreen or fake
    /// fullscreen state ends.
    fn base_restore(self) -> RestoredPresentation {
        match self.presentation {
            ClientPresentation::Normal => RestoredPresentation::Normal,
            ClientPresentation::Maximized => RestoredPresentation::Maximized,
            ClientPresentation::FakeArmed { restore }
            | ClientPresentation::RealFullscreen { restore }
            | ClientPresentation::FakeActive { restore } => restore,
        }
    }

    #[inline]
    pub(crate) fn as_real_fullscreen(self) -> Self {
        Self {
            presentation: ClientPresentation::RealFullscreen {
                restore: self.base_restore(),
            },
            ..self
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn as_fullscreen(self) -> Self {
        self.as_real_fullscreen()
    }

    #[inline]
    pub(crate) fn as_fake_active(self) -> Self {
        Self {
            presentation: ClientPresentation::FakeActive {
                restore: self.base_restore(),
            },
            ..self
        }
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn as_fake_fullscreen(self) -> Self {
        self.as_fake_active()
    }

    #[inline]
    pub(crate) fn as_fake_armed(self) -> Self {
        Self {
            presentation: ClientPresentation::FakeArmed {
                restore: self.base_restore(),
            },
            ..self
        }
    }

    #[inline]
    pub(crate) fn with_maximized(self, maximized: bool) -> Self {
        let presentation = match (self.presentation, maximized) {
            (ClientPresentation::RealFullscreen { .. }, true) => {
                ClientPresentation::RealFullscreen {
                    restore: RestoredPresentation::Maximized,
                }
            }
            (
                ClientPresentation::RealFullscreen {
                    restore: RestoredPresentation::Maximized,
                },
                false,
            ) => ClientPresentation::RealFullscreen {
                restore: RestoredPresentation::Normal,
            },
            (real @ ClientPresentation::RealFullscreen { .. }, false) => real,
            (ClientPresentation::FakeActive { .. }, true) => ClientPresentation::FakeActive {
                restore: RestoredPresentation::Maximized,
            },
            (
                ClientPresentation::FakeActive {
                    restore: RestoredPresentation::Maximized,
                },
                false,
            ) => ClientPresentation::FakeActive {
                restore: RestoredPresentation::Normal,
            },
            (fake @ ClientPresentation::FakeActive { .. }, false) => fake,
            (ClientPresentation::FakeArmed { .. }, true) => ClientPresentation::FakeArmed {
                restore: RestoredPresentation::Maximized,
            },
            (ClientPresentation::FakeArmed { .. }, false) => ClientPresentation::FakeArmed {
                restore: RestoredPresentation::Normal,
            },
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

    /// Leave a temporary presentation mode and restore its base presentation.
    ///
    /// This also clears an armed fake-fullscreen policy. Use the explicit fake
    /// transitions when the policy should survive.
    #[inline]
    pub(crate) fn restored(self) -> Self {
        let presentation = match self.presentation {
            ClientPresentation::Normal => ClientPresentation::Normal,
            // `restore_mode` clears a direct maximized presentation. A
            // maximized *restore* target behind fullscreen/armed fake
            // fullscreen is preserved below because it is not the current
            // presentation.
            ClientPresentation::Maximized => ClientPresentation::Normal,
            ClientPresentation::FakeArmed {
                restore: RestoredPresentation::Normal,
            }
            | ClientPresentation::RealFullscreen {
                restore: RestoredPresentation::Normal,
            }
            | ClientPresentation::FakeActive {
                restore: RestoredPresentation::Normal,
            } => ClientPresentation::Normal,
            ClientPresentation::FakeArmed {
                restore: RestoredPresentation::Maximized,
            }
            | ClientPresentation::RealFullscreen {
                restore: RestoredPresentation::Maximized,
            }
            | ClientPresentation::FakeActive {
                restore: RestoredPresentation::Maximized,
            } => ClientPresentation::Maximized,
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

    /// Whether a size the client commits itself may become authoritative
    /// model geometry.
    ///
    /// Only normal floating windows grant the client that authority. Tiled,
    /// maximized, fullscreen, and scratchpad geometry is owned by the WM, so
    /// their committed sizes must never feed back into the model. Fake active
    /// fullscreen is excluded even though it uses normal layout geometry: the
    /// application believes it is fullscreen and may commit buffers at output
    /// size.
    #[inline]
    pub fn client_size_is_authoritative(&self) -> bool {
        self.mode.is_normal_floating() && !self.mode.is_fake_fullscreen() && !self.is_scratchpad()
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
        self.mode = self.mode.as_real_fullscreen();
    }

    /// Enter fake active fullscreen while remembering the current base
    /// presentation.
    #[inline]
    pub(crate) fn enter_fake_fullscreen(&mut self) {
        self.mode = self.mode.as_fake_active();
    }

    /// Arm fake fullscreen without advertising any fullscreen state yet.
    #[inline]
    pub(crate) fn arm_fake_fullscreen(&mut self) {
        self.mode = self.mode.as_fake_armed();
    }

    /// Set maximized presentation without changing persistent placement.
    #[inline]
    pub(crate) fn set_maximized_presentation(&mut self, maximized: bool) {
        self.mode = self.mode.with_maximized(maximized);
    }

    /// Leave a temporary presentation mode and restore its base presentation.
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
            ClientMode::tiled().as_fake_armed(),
            ClientMode::tiled().as_fake_fullscreen(),
            ClientMode::tiled().as_maximized(),
        ] {
            let changed = mode.with_placement(ClientPlacement::Floating);
            assert_eq!(changed.presentation, mode.presentation);
            assert_eq!(changed.restored(), ClientMode::floating());
        }
    }

    #[test]
    fn fake_armed_keeps_the_base_presentation_without_advertising_fullscreen() {
        let armed = ClientMode::tiled().as_fake_armed();
        assert!(armed.is_fake_armed());
        assert!(!armed.is_fullscreen());
        assert!(armed.is_normal_tiling());
        assert_eq!(armed.restored(), ClientMode::tiled());

        let maximized = ClientMode::tiled().as_maximized().as_fake_armed();
        assert!(maximized.is_fake_armed());
        assert!(maximized.is_maximized());
        assert!(!maximized.is_normal_tiling());
        assert_eq!(maximized.restored(), ClientMode::tiled().as_maximized());
    }

    #[test]
    fn fake_active_keeps_normal_geometry_and_advertises_fullscreen() {
        let active = ClientMode::tiled().as_fake_fullscreen();
        assert!(active.is_fake_fullscreen());
        assert!(active.is_fullscreen());
        assert!(!active.is_true_fullscreen());
        assert!(active.is_normal_tiling());
        assert_eq!(active.restored(), ClientMode::tiled());
    }
}
