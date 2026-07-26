//! Backend-neutral presentation-mode transactions.
//!
//! Each transaction resolves the client and its monitor exactly once, commits
//! the complete authoritative model change, and returns an owned snapshot for
//! backend I/O, layout scheduling, and animation after the model borrow ends.

use crate::model::WmModel;
use crate::types::{
    Client, ClientMode, ClientPlacement, MaximizedOrigin, MonitorId, Rect, WindowId,
};

/// Commit only the client-local portion of a fullscreen transition.
///
/// Model transactions use this directly, and compound policy transactions may
/// reuse it while already holding the sole mutable client borrow. Keeping the
/// state-machine operation here prevents those transactions from duplicating
/// fullscreen semantics or looking the client up again.
fn set_client_fullscreen(client: &mut Client, fullscreen: bool) -> (ClientMode, bool) {
    let previous_mode = client.mode();
    let changed = if fullscreen {
        !previous_mode.is_true_fullscreen()
    } else {
        previous_mode.is_fullscreen()
    };

    if changed {
        if fullscreen {
            client.enter_fullscreen();
            client.save_border_width();
            client.border_width = 0;
        } else {
            client.restore_mode();
            if previous_mode.is_true_fullscreen() {
                client.restore_border_width();
            }
        }
    }

    (previous_mode, changed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the transition contains required backend and scheduling work"]
pub(crate) enum FullscreenTransition {
    Unchanged {
        monitor_id: MonitorId,
    },
    EnteredFromLayout {
        monitor_id: MonitorId,
        monitor_rect: Rect,
    },
    EnteredFromFloating {
        monitor_id: MonitorId,
        monitor_rect: Rect,
    },
    EnteredFromFakeFullscreen {
        monitor_id: MonitorId,
        monitor_rect: Rect,
    },
    ExitedToTiling {
        monitor_id: MonitorId,
    },
    ExitedToFloating {
        monitor_id: MonitorId,
        restore_rect: Rect,
    },
    ExitedToMaximized {
        monitor_id: MonitorId,
        work_rect: Rect,
    },
}

impl FullscreenTransition {
    #[inline]
    pub(crate) fn changed(self) -> bool {
        !matches!(self, Self::Unchanged { .. })
    }

    #[inline]
    pub(crate) fn monitor_id(self) -> MonitorId {
        match self {
            Self::Unchanged { monitor_id }
            | Self::EnteredFromLayout { monitor_id, .. }
            | Self::EnteredFromFloating { monitor_id, .. }
            | Self::EnteredFromFakeFullscreen { monitor_id, .. }
            | Self::ExitedToTiling { monitor_id }
            | Self::ExitedToFloating { monitor_id, .. }
            | Self::ExitedToMaximized { monitor_id, .. } => monitor_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the transition contains required geometry and scheduling work"]
pub(crate) enum MaximizedTransition {
    Unchanged {
        monitor_id: MonitorId,
    },
    Entered {
        monitor_id: MonitorId,
        work_rect: Rect,
    },
    ExitedToTiling {
        monitor_id: MonitorId,
    },
    ExitedToFloating {
        monitor_id: MonitorId,
        restore_rect: Rect,
    },
    UpdatedFullscreenRestore {
        monitor_id: MonitorId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "leaving maximization requires backend state and geometry projection"]
pub(crate) struct LeaveMaximizedTransition {
    pub(crate) origin: MaximizedOrigin,
    pub(crate) transition: MaximizedTransition,
}

impl MaximizedTransition {
    #[inline]
    pub(crate) fn changed(self) -> bool {
        !matches!(self, Self::Unchanged { .. })
    }

    #[inline]
    pub(crate) fn entered(self) -> bool {
        matches!(self, Self::Entered { .. })
    }

    #[inline]
    pub(crate) fn monitor_id(self) -> MonitorId {
        match self {
            Self::Unchanged { monitor_id }
            | Self::Entered { monitor_id, .. }
            | Self::ExitedToTiling { monitor_id }
            | Self::ExitedToFloating { monitor_id, .. }
            | Self::UpdatedFullscreenRestore { monitor_id } => monitor_id,
        }
    }
}

impl WmModel {
    /// Set real-fullscreen presentation and return the complete backend plan.
    ///
    /// Fake fullscreen is deliberately considered distinct from real
    /// fullscreen: a real fullscreen request promotes it, while an
    /// unfullscreen request leaves either fullscreen variant.
    pub(crate) fn set_fullscreen(
        &mut self,
        win: WindowId,
        fullscreen: bool,
    ) -> Option<FullscreenTransition> {
        let clients = &mut self.clients;
        let monitors = &self.monitors;
        let client = clients.get_mut(&win)?;
        let monitor = monitors.get(client.monitor_id)?;

        let previous_mode = client.mode();
        if fullscreen && previous_mode.is_normal_floating() {
            client.save_floating_placement(client.geo, monitor.work_rect());
        }
        let (_, changed) = set_client_fullscreen(client, fullscreen);
        let monitor_id = client.monitor_id;

        if !changed {
            return Some(FullscreenTransition::Unchanged { monitor_id });
        }

        if fullscreen {
            return Some(if previous_mode.is_normal_floating() {
                FullscreenTransition::EnteredFromFloating {
                    monitor_id,
                    monitor_rect: monitor.monitor_rect,
                }
            } else if previous_mode.is_fake_fullscreen() {
                FullscreenTransition::EnteredFromFakeFullscreen {
                    monitor_id,
                    monitor_rect: monitor.monitor_rect,
                }
            } else {
                FullscreenTransition::EnteredFromLayout {
                    monitor_id,
                    monitor_rect: monitor.monitor_rect,
                }
            });
        }

        Some(if client.mode().is_maximized() {
            FullscreenTransition::ExitedToMaximized {
                monitor_id,
                work_rect: monitor.work_rect(),
            }
        } else if client.placement() == ClientPlacement::Floating {
            let restore_rect = crate::client::geometry::resolve_floating_transition(
                client,
                monitor.work_rect(),
                crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
            );
            client.update_geometry(restore_rect);
            FullscreenTransition::ExitedToFloating {
                monitor_id,
                restore_rect,
            }
        } else {
            FullscreenTransition::ExitedToTiling { monitor_id }
        })
    }

    fn set_maximized_with_origin(
        &mut self,
        win: WindowId,
        maximized: bool,
        origin: MaximizedOrigin,
    ) -> Option<MaximizedTransition> {
        let clients = &mut self.clients;
        let monitors = &self.monitors;
        let client = clients.get_mut(&win)?;
        let monitor = monitors.get(client.monitor_id)?;

        let previous_mode = client.mode();
        if maximized && previous_mode.is_normal_floating() {
            client.save_floating_placement(client.geo, monitor.work_rect());
        }
        client.set_maximized_presentation(maximized, origin);
        let current_mode = client.mode();
        let monitor_id = client.monitor_id;

        if current_mode == previous_mode {
            return Some(MaximizedTransition::Unchanged { monitor_id });
        }
        if current_mode.is_fullscreen() {
            return Some(MaximizedTransition::UpdatedFullscreenRestore { monitor_id });
        }
        if current_mode.is_maximized() {
            return Some(MaximizedTransition::Entered {
                monitor_id,
                work_rect: monitor.work_rect(),
            });
        }
        if current_mode.placement() == ClientPlacement::Floating {
            let restore_rect = crate::client::geometry::resolve_floating_transition(
                client,
                monitor.work_rect(),
                crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
            );
            client.update_geometry(restore_rect);
            return Some(MaximizedTransition::ExitedToFloating {
                monitor_id,
                restore_rect,
            });
        }
        Some(MaximizedTransition::ExitedToTiling { monitor_id })
    }

    /// Apply a client protocol maximize request.
    pub(crate) fn set_client_maximized(
        &mut self,
        win: WindowId,
        maximized: bool,
    ) -> Option<MaximizedTransition> {
        self.set_maximized_with_origin(win, maximized, MaximizedOrigin::Client)
    }

    /// Apply instantWM's protocol-independent per-window zoom.
    pub(crate) fn set_wm_maximized(
        &mut self,
        win: WindowId,
        maximized: bool,
    ) -> Option<MaximizedTransition> {
        self.set_maximized_with_origin(win, maximized, MaximizedOrigin::Wm)
    }

    /// Leave whichever maximized presentation currently owns the window.
    ///
    /// Interactive WM actions must not guess whether maximization came from
    /// the client protocol or instantWM's own zoom command: the origin
    /// determines which backend state must be cleared.
    pub(crate) fn leave_maximized(&mut self, win: WindowId) -> Option<LeaveMaximizedTransition> {
        let origin = self.client(win)?.mode().maximized_origin()?;
        let transition = self.set_maximized_with_origin(win, false, origin)?;
        Some(LeaveMaximizedTransition { origin, transition })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Client, Monitor};

    fn model_with_client(mode: ClientMode) -> (WmModel, WindowId, MonitorId) {
        let mut model = WmModel::default();
        let monitor_id = model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        let win = WindowId(1);
        let mut client = Client {
            win,
            monitor_id,
            border_width: 2,
            old_border_width: 2,
            geo: Rect::new(10, 20, 800, 600),
            old_geo: Rect::new(30, 40, 640, 480),
            ..Client::default()
        };
        client.set_mode_for_test(mode);
        model.insert_client(client);
        (model, win, monitor_id)
    }

    #[test]
    fn fullscreen_transaction_returns_backend_snapshot_and_saves_border() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());

        let transition = model.set_fullscreen(win, true).unwrap();

        assert_eq!(
            transition,
            FullscreenTransition::EnteredFromLayout {
                monitor_id,
                monitor_rect: Rect::new(0, 0, 1920, 1080),
            }
        );
        let client = model.client(win).unwrap();
        assert!(client.mode().is_true_fullscreen());
        assert_eq!(client.border_width, 0);
        assert_eq!(client.old_border_width, 2);
    }

    #[test]
    fn fullscreen_transaction_is_idempotent() {
        let (mut model, win, _) = model_with_client(ClientMode::tiled());
        assert!(model.set_fullscreen(win, true).unwrap().changed());
        assert!(!model.set_fullscreen(win, true).unwrap().changed());
    }

    #[test]
    fn unfullscreen_transaction_restores_placement_and_border() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());
        let _ = model.set_fullscreen(win, true).unwrap();

        let transition = model.set_fullscreen(win, false).unwrap();

        assert_eq!(
            transition,
            FullscreenTransition::ExitedToTiling { monitor_id }
        );
        let client = model.client(win).unwrap();
        assert!(client.mode().is_normal_tiling());
        assert_eq!(client.border_width, 2);
    }

    #[test]
    fn unfullscreen_atomically_restores_saved_floating_geometry() {
        let floating_rect = Rect::new(10, 20, 800, 600);
        let fullscreen_rect = Rect::new(0, 0, 1920, 1080);
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());

        let entered = model.set_fullscreen(win, true).unwrap();
        assert_eq!(
            entered,
            FullscreenTransition::EnteredFromFloating {
                monitor_id,
                monitor_rect: fullscreen_rect,
            }
        );
        model
            .client_mut(win)
            .unwrap()
            .update_geometry(fullscreen_rect);

        let exited = model.set_fullscreen(win, false).unwrap();

        assert_eq!(
            exited,
            FullscreenTransition::ExitedToFloating {
                monitor_id,
                restore_rect: floating_rect,
            }
        );
        let client = model.client(win).unwrap();
        assert!(client.mode().is_normal_floating());
        assert_eq!(client.geo, floating_rect);
        assert_eq!(client.old_geo, fullscreen_rect);
        assert_eq!(client.saved_floating_rect(), Some(floating_rect));
    }

    #[test]
    fn real_fullscreen_request_promotes_fake_fullscreen() {
        let (mut model, win, monitor_id) =
            model_with_client(ClientMode::tiled().as_fake_fullscreen());

        let transition = model.set_fullscreen(win, true).unwrap();

        assert_eq!(
            transition,
            FullscreenTransition::EnteredFromFakeFullscreen {
                monitor_id,
                monitor_rect: Rect::new(0, 0, 1920, 1080),
            }
        );
        assert!(model.client(win).unwrap().mode().is_true_fullscreen());
    }

    #[test]
    fn leaving_fake_fullscreen_does_not_restore_an_unrelated_border_snapshot() {
        let (mut model, win, _) = model_with_client(ClientMode::tiled().as_fake_fullscreen());
        let client = model.client_mut(win).unwrap();
        client.border_width = 3;
        client.old_border_width = 9;

        let _ = model.set_fullscreen(win, false).unwrap();

        assert_eq!(model.client(win).unwrap().border_width, 3);
    }

    #[test]
    fn maximize_transaction_returns_work_and_restore_geometry() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());

        let transition = model.set_client_maximized(win, true).unwrap();

        assert!(transition.entered());
        assert_eq!(transition.monitor_id(), monitor_id);
        assert!(model.client(win).unwrap().mode().is_maximized());
    }

    #[test]
    fn unrelated_presentation_is_not_destroyed_by_unmaximize() {
        let (mut model, win, _) = model_with_client(ClientMode::tiled().as_fullscreen());

        let transition = model.set_client_maximized(win, false).unwrap();

        assert_eq!(
            transition,
            MaximizedTransition::Unchanged {
                monitor_id: model.client(win).unwrap().monitor_id
            }
        );
        assert!(model.client(win).unwrap().mode().is_true_fullscreen());
        assert!(!model.client(win).unwrap().mode().is_protocol_maximized());
    }

    #[test]
    fn fullscreen_restores_client_requested_maximization() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());
        assert!(model.set_client_maximized(win, true).unwrap().changed());

        let entered = model.set_fullscreen(win, true).unwrap();
        assert_eq!(
            entered,
            FullscreenTransition::EnteredFromLayout {
                monitor_id,
                monitor_rect: Rect::new(0, 0, 1920, 1080),
            }
        );
        let fullscreen = model.client(win).unwrap();
        assert!(fullscreen.mode().is_true_fullscreen());
        assert!(fullscreen.mode().is_protocol_maximized());

        let exited = model.set_fullscreen(win, false).unwrap();
        assert_eq!(
            exited,
            FullscreenTransition::ExitedToMaximized {
                monitor_id,
                work_rect: Rect::new(0, 0, 1920, 1080),
            }
        );
        let restored = model.client(win).unwrap();
        assert!(restored.mode().is_maximized());
        assert!(restored.mode().is_protocol_maximized());
    }

    #[test]
    fn maximize_request_during_fullscreen_updates_the_restore_presentation() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        let _ = model.set_fullscreen(win, true).unwrap();

        assert_eq!(
            model.set_client_maximized(win, true).unwrap(),
            MaximizedTransition::UpdatedFullscreenRestore { monitor_id }
        );
        assert!(model.client(win).unwrap().mode().is_true_fullscreen());

        assert_eq!(
            model.set_fullscreen(win, false).unwrap(),
            FullscreenTransition::ExitedToMaximized {
                monitor_id,
                work_rect: Rect::new(0, 0, 1920, 1080),
            }
        );
        let restored = model.client(win).unwrap();
        assert!(restored.mode().is_maximized());
        assert_eq!(restored.placement(), ClientPlacement::Floating);
    }

    #[test]
    fn tiled_unmaximize_requires_layout_instead_of_restoring_maximized_rect() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());
        let _ = model.set_client_maximized(win, true).unwrap();
        model
            .client_mut(win)
            .unwrap()
            .update_geometry(Rect::new(0, 0, 1920, 1080));

        assert_eq!(
            model.set_client_maximized(win, false).unwrap(),
            MaximizedTransition::ExitedToTiling { monitor_id }
        );
        assert!(model.client(win).unwrap().mode().is_normal_tiling());
    }

    #[test]
    fn wm_zoom_is_not_projected_as_client_maximization() {
        let (mut model, win, _) = model_with_client(ClientMode::tiled());

        let _ = model.set_wm_maximized(win, true).unwrap();

        let mode = model.client(win).unwrap().mode();
        assert!(mode.is_wm_maximized());
        assert!(!mode.is_protocol_maximized());
    }

    #[test]
    fn leave_maximized_reports_the_owner_and_restores_placement() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        let _ = model.set_client_maximized(win, true).unwrap();

        let left = model.leave_maximized(win).unwrap();

        assert_eq!(left.origin, MaximizedOrigin::Client);
        assert_eq!(left.transition.monitor_id(), monitor_id);
        assert!(matches!(
            left.transition,
            MaximizedTransition::ExitedToFloating { .. }
        ));
        assert!(model.client(win).unwrap().mode().is_normal_floating());
        assert!(model.leave_maximized(win).is_none());
    }
}
