//! Backend-neutral presentation-mode transactions.
//!
//! Each transaction resolves the client and its monitor exactly once, commits
//! the complete authoritative model change, and returns an owned snapshot for
//! backend I/O, layout scheduling, and animation after the model borrow ends.

use crate::layouts::PresentationMode;
use crate::model::WmModel;
use crate::types::{Client, ClientMode, ClientPlacement, Monitor, MonitorId, Rect, WindowId};

/// Presentation state requested by a client before its backend has completed
/// the authoritative window-management transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InitialPresentationIntent {
    pub fullscreen: bool,
    pub maximized: bool,
}

/// Commit only the client-local portion of a fullscreen transition.
///
/// Monitor-owned placement bookkeeping stays with the caller; this helper owns
/// the mode state machine plus the border-width save/restore. It returns the
/// mode the client held before the transition together with whether anything
/// actually changed, so the caller classifies the outcome without re-reading
/// the client's mode.
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
pub(crate) struct FullscreenTransition {
    pub monitor_id: MonitorId,
    pub change: FullscreenChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullscreenChange {
    Unchanged,
    Entered {
        monitor_rect: Rect,
        projection: FullscreenEntryProjection,
    },
    Exited {
        restore_rect: Option<Rect>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullscreenEntryProjection {
    Animated,
    BackendOnly,
}

impl FullscreenTransition {
    pub(crate) fn changed(self) -> bool {
        self.change != FullscreenChange::Unchanged
    }

    pub(crate) fn presentation_rect(self) -> Option<Rect> {
        match self.change {
            FullscreenChange::Entered { monitor_rect, .. } => Some(monitor_rect),
            FullscreenChange::Exited { restore_rect } => restore_rect,
            FullscreenChange::Unchanged => None,
        }
    }

    pub(crate) fn entered(self) -> bool {
        matches!(self.change, FullscreenChange::Entered { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the transition contains required geometry and scheduling work"]
pub(crate) struct MaximizedTransition {
    pub monitor_id: MonitorId,
    pub change: MaximizedChange,
}

/// Visible effect of a maximize-state change. A fullscreen client only has
/// its restore mode updated, which is `Unchanged` from the outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaximizedChange {
    Unchanged,
    Entered { work_rect: Rect },
    Exited { restore_rect: Option<Rect> },
}

impl MaximizedChange {
    pub(crate) fn changed(self) -> bool {
        self != Self::Unchanged
    }

    pub(crate) fn presentation_rect(self) -> Option<Rect> {
        match self {
            Self::Entered { work_rect } => Some(work_rect),
            Self::Exited { restore_rect } => restore_rect,
            Self::Unchanged => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "client maximize intent requires geometry, layout, and protocol projection"]
pub(crate) struct ClientMaximizeIntentTransition {
    pub monitor_id: MonitorId,
    pub outcome: ClientMaximizeIntentOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientMaximizeIntentOutcome {
    Placement {
        placement: ClientPlacement,
        changed: bool,
        visible_restore_rect: Option<Rect>,
    },
    FloatingPresentation(MaximizedChange),
    Rejected,
}

impl ClientMaximizeIntentTransition {
    pub(crate) fn changed(self) -> bool {
        match self.outcome {
            ClientMaximizeIntentOutcome::Placement { changed, .. } => changed,
            ClientMaximizeIntentOutcome::FloatingPresentation(change) => change.changed(),
            ClientMaximizeIntentOutcome::Rejected => false,
        }
    }

    pub(crate) fn entered_floating_presentation(self) -> bool {
        matches!(
            self.outcome,
            ClientMaximizeIntentOutcome::FloatingPresentation(MaximizedChange::Entered { .. })
        )
    }

    pub(crate) fn presentation_rect(self) -> Option<Rect> {
        match self.outcome {
            ClientMaximizeIntentOutcome::Placement {
                visible_restore_rect,
                ..
            } => visible_restore_rect,
            ClientMaximizeIntentOutcome::FloatingPresentation(change) => change.presentation_rect(),
            ClientMaximizeIntentOutcome::Rejected => None,
        }
    }
}

/// Geometry a client leaving fullscreen or maximization returns to: the work
/// area when still maximized, its restored floating rectangle when floating,
/// and nothing when the layout owns it.
fn restore_rect_after_leaving(client: &mut Client, work_rect: Rect) -> Option<Rect> {
    if client.mode().is_maximized() {
        Some(work_rect)
    } else if client.placement() == ClientPlacement::Floating {
        let restore_rect = crate::client::geometry::resolve_floating_transition(
            client,
            work_rect,
            crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
            client.border_width,
        );
        client.update_geometry(restore_rect);
        Some(restore_rect)
    } else {
        None
    }
}

/// Whether the application should perceive its window as maximized.
///
/// Tiling presentations deliberately expose tiled placement through the
/// application's maximize/restore control. In a global floating
/// presentation, maximization retains its literal full-work-area meaning.
pub(crate) fn protocol_maximized(monitor: &Monitor, client: &Client) -> bool {
    if monitor.current_layout() == PresentationMode::Floating {
        client.mode().has_maximized_presentation()
    } else {
        client.placement() == ClientPlacement::Tiling
    }
}

/// Reach a client through the monitor already resolved as its owner.
fn owned_client_mut(monitor: &mut Monitor, win: WindowId) -> &mut Client {
    monitor
        .client_mut(win)
        .expect("monitor was resolved as the client's owner")
}

/// Commit the client-local portion of a maximize-state change.
fn set_client_maximized(client: &mut Client, work_rect: Rect, maximized: bool) -> MaximizedChange {
    let previous_mode = client.mode();
    if maximized && previous_mode.is_normal_floating() {
        client.save_floating_placement(client.geo, work_rect);
    }
    client.set_maximized_presentation(maximized);
    let current_mode = client.mode();

    if current_mode == previous_mode || current_mode.is_fullscreen() {
        MaximizedChange::Unchanged
    } else if current_mode.is_maximized() {
        MaximizedChange::Entered { work_rect }
    } else {
        MaximizedChange::Exited {
            restore_rect: restore_rect_after_leaving(client, work_rect),
        }
    }
}

fn set_fullscreen_on(
    monitor: &mut Monitor,
    win: WindowId,
    fullscreen: bool,
) -> FullscreenTransition {
    let monitor_id = monitor.id();
    let monitor_rect = monitor.monitor_rect;
    let work_rect = monitor.work_rect();
    let client = owned_client_mut(monitor, win);

    if fullscreen && client.mode().is_normal_floating() {
        client.save_floating_placement(client.geo, work_rect);
    }
    let (previous_mode, changed) = set_client_fullscreen(client, fullscreen);

    let change = if !changed {
        FullscreenChange::Unchanged
    } else if fullscreen {
        FullscreenChange::Entered {
            monitor_rect,
            projection: if previous_mode.is_normal_floating() {
                FullscreenEntryProjection::BackendOnly
            } else {
                FullscreenEntryProjection::Animated
            },
        }
    } else {
        FullscreenChange::Exited {
            restore_rect: restore_rect_after_leaving(client, work_rect),
        }
    };
    FullscreenTransition { monitor_id, change }
}

fn set_maximized_on(monitor: &mut Monitor, win: WindowId, maximized: bool) -> MaximizedTransition {
    let monitor_id = monitor.id();
    let work_rect = monitor.work_rect();
    let change = set_client_maximized(owned_client_mut(monitor, win), work_rect, maximized);
    MaximizedTransition { monitor_id, change }
}

fn apply_maximize_intent_on(
    monitor: &mut Monitor,
    win: WindowId,
    maximized: bool,
) -> ClientMaximizeIntentTransition {
    let monitor_id = monitor.id();
    let floating_presentation = monitor.current_layout() == PresentationMode::Floating;
    let work_rect = monitor.work_rect();
    let client = owned_client_mut(monitor, win);

    let rejected = client.is_fixed_size || client.transient_for.is_some() || client.is_scratchpad();
    if maximized && rejected {
        return ClientMaximizeIntentTransition {
            monitor_id,
            outcome: ClientMaximizeIntentOutcome::Rejected,
        };
    }

    if floating_presentation {
        let was_client_maximized = client.mode().has_maximized_presentation();
        if maximized && !client.mode().is_fullscreen() {
            // Global floating presentation makes even persistently tiled
            // clients free-positioned, so literal maximization must save
            // their current visible rectangle too.
            client.save_floating_placement(client.geo, work_rect);
        }
        let mut change = set_client_maximized(client, work_rect, maximized);
        if !maximized
            && was_client_maximized
            && !client.mode().is_fullscreen()
            && change == (MaximizedChange::Exited { restore_rect: None })
        {
            let restore_rect = crate::client::geometry::resolve_floating_transition(
                client,
                work_rect,
                crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
                client.border_width,
            );
            client.update_geometry(restore_rect);
            change = MaximizedChange::Exited {
                restore_rect: Some(restore_rect),
            };
        }
        if matches!(change, MaximizedChange::Entered { .. }) {
            monitor.raise_client(win);
        }
        return ClientMaximizeIntentTransition {
            monitor_id,
            outcome: ClientMaximizeIntentOutcome::FloatingPresentation(change),
        };
    }

    let target = if maximized {
        ClientPlacement::Tiling
    } else {
        ClientPlacement::Floating
    };
    let before_mode = client.mode();
    let before_geo = client.geo;
    let before_border = client.border_width;

    // A client-maximized presentation can survive a layout-presentation
    // switch. Once tiling is available, collapse it into placement before
    // applying the new intent.
    if before_mode.has_maximized_presentation() {
        client.set_maximized_presentation(false);
    }

    let mut visible_restore_rect = None;
    match target {
        ClientPlacement::Tiling => {
            if client.placement() == ClientPlacement::Floating {
                if client.mode().is_normal_floating() {
                    client.save_floating_placement(client.geo, work_rect);
                }
                client.set_placement(ClientPlacement::Tiling);
            }
        }
        ClientPlacement::Floating => {
            if client.placement() != ClientPlacement::Floating {
                let restored = crate::client::geometry::resolve_floating_transition(
                    client,
                    work_rect,
                    crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
                    client.restored_border_width(),
                );
                client.set_placement(ClientPlacement::Floating);
                client.save_floating_placement(restored, work_rect);
                if client.mode().is_normal_floating() {
                    client.restore_border_width();
                    client.update_geometry(restored);
                    visible_restore_rect = Some(restored);
                }
            } else if client.mode().is_normal_floating() {
                visible_restore_rect = Some(client.geo);
            }
        }
    }

    ClientMaximizeIntentTransition {
        monitor_id,
        outcome: ClientMaximizeIntentOutcome::Placement {
            placement: target,
            changed: client.mode() != before_mode
                || client.geo != before_geo
                || client.border_width != before_border,
            visible_restore_rect,
        },
    }
}

impl WmModel {
    /// Apply coalesced initial presentation intent after rules and backend
    /// placement policy have established the client's restore mode.
    pub(crate) fn apply_initial_presentation_intent(
        &mut self,
        win: WindowId,
        intent: InitialPresentationIntent,
    ) {
        let Some(monitor) = self.client_owner_mut(win) else {
            return;
        };
        // Maximization establishes the mode restored after fullscreen, so it
        // must be applied first regardless of protocol request ordering.
        if intent.maximized {
            let _ = apply_maximize_intent_on(monitor, win, true);
        }
        if intent.fullscreen {
            let _ = set_fullscreen_on(monitor, win, true);
        }
    }

    /// Whether the application should perceive `win` as maximized.
    ///
    /// Callers already holding a [`crate::model::ClientView`] use
    /// [`protocol_maximized`] directly.
    pub(crate) fn client_protocol_maximized(&self, win: WindowId) -> Option<bool> {
        let view = self.client_view(win)?;
        Some(protocol_maximized(view.monitor, view.client))
    }

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
        Some(set_fullscreen_on(
            self.client_owner_mut(win)?,
            win,
            fullscreen,
        ))
    }

    /// Interpret an application's maximize/restore control.
    ///
    /// In tiling presentations it controls tiled/floating placement. In the
    /// global floating presentation it controls a literal work-area maximize.
    pub(crate) fn apply_client_maximize_intent(
        &mut self,
        win: WindowId,
        maximized: bool,
    ) -> Option<ClientMaximizeIntentTransition> {
        Some(apply_maximize_intent_on(
            self.client_owner_mut(win)?,
            win,
            maximized,
        ))
    }

    /// Collapse literal client maximization into tiled placement when a
    /// monitor leaves the global floating presentation.
    pub(crate) fn reconcile_client_maximization_for_tiling(
        &mut self,
        monitor_id: MonitorId,
    ) -> Vec<WindowId> {
        let Some(monitor) = self.monitors.get_mut(monitor_id) else {
            return Vec::new();
        };
        let work_rect = monitor.work_rect();
        let changed: Vec<WindowId> = monitor
            .clients()
            .values()
            .filter(|client| client.mode().has_maximized_presentation())
            .map(|client| client.win)
            .collect();

        for &win in &changed {
            let client = owned_client_mut(monitor, win);
            let _ = set_client_maximized(client, work_rect, false);
            client.set_placement(ClientPlacement::Tiling);
        }
        changed
    }

    /// Leave the client's maximized presentation.
    pub(crate) fn leave_maximized(&mut self, win: WindowId) -> Option<MaximizedTransition> {
        let monitor = self.client_owner_mut(win)?;
        if !owned_client_mut(monitor, win).mode().is_maximized() {
            return None;
        }
        if monitor.current_layout() != PresentationMode::Floating {
            return Some(set_maximized_on(monitor, win, false));
        }
        let intent = apply_maximize_intent_on(monitor, win, false);
        match intent.outcome {
            ClientMaximizeIntentOutcome::FloatingPresentation(change) => {
                Some(MaximizedTransition {
                    monitor_id: intent.monitor_id,
                    change,
                })
            }
            _ => unreachable!("floating presentation must use literal client maximization"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MonitorBuilder;
    use crate::types::Client;

    fn model_with_client(mode: ClientMode) -> (WmModel, WindowId, MonitorId) {
        let mut model = WmModel::default();
        let monitor_id = model.monitors.push(
            MonitorBuilder::new()
                .rect(Rect::new(0, 0, 1920, 1080), Rect::new(0, 0, 1920, 1080))
                .build(),
        );
        let win = WindowId(1);
        let mut client = Client {
            win,
            border_width: 2,
            old_border_width: 2,
            geo: Rect::new(10, 20, 800, 600),
            old_geo: Rect::new(30, 40, 640, 480),
            ..Client::default()
        };
        client.set_mode_for_test(mode);
        assert!(model.add_client(monitor_id, client));
        (model, win, monitor_id)
    }

    #[test]
    fn fullscreen_transaction_returns_backend_snapshot_and_saves_border() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());

        let transition = model.set_fullscreen(win, true).unwrap();

        assert_eq!(
            transition,
            FullscreenTransition {
                monitor_id,
                change: FullscreenChange::Entered {
                    monitor_rect: Rect::new(0, 0, 1920, 1080),
                    projection: FullscreenEntryProjection::Animated,
                },
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
            FullscreenTransition {
                monitor_id,
                change: FullscreenChange::Exited { restore_rect: None },
            }
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
            FullscreenTransition {
                monitor_id,
                change: FullscreenChange::Entered {
                    monitor_rect: fullscreen_rect,
                    projection: FullscreenEntryProjection::BackendOnly,
                },
            }
        );
        model
            .client_mut(win)
            .unwrap()
            .update_geometry(fullscreen_rect);

        let exited = model.set_fullscreen(win, false).unwrap();

        assert_eq!(
            exited,
            FullscreenTransition {
                monitor_id,
                change: FullscreenChange::Exited {
                    restore_rect: Some(floating_rect)
                },
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
            FullscreenTransition {
                monitor_id,
                change: FullscreenChange::Entered {
                    monitor_rect: Rect::new(0, 0, 1920, 1080),
                    projection: FullscreenEntryProjection::Animated,
                },
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
    fn tiled_placement_is_projected_as_client_maximized() {
        let (model, win, _) = model_with_client(ClientMode::tiled());

        assert_eq!(model.client_protocol_maximized(win), Some(true));
    }

    #[test]
    fn client_maximize_tiles_in_a_tiling_presentation() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());

        assert_eq!(
            model.apply_client_maximize_intent(win, true).unwrap(),
            ClientMaximizeIntentTransition {
                monitor_id,
                outcome: ClientMaximizeIntentOutcome::Placement {
                    placement: ClientPlacement::Tiling,
                    changed: true,
                    visible_restore_rect: None,
                },
            }
        );
        assert!(model.client(win).unwrap().mode().is_normal_tiling());
        assert_eq!(model.client_protocol_maximized(win), Some(true));
    }

    #[test]
    fn client_restore_floats_and_restores_geometry_in_a_tiling_presentation() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        let floating = model.client(win).unwrap().geo;
        let _ = model.apply_client_maximize_intent(win, true).unwrap();
        model
            .client_mut(win)
            .unwrap()
            .update_geometry(Rect::new(0, 0, 1920, 1080));

        assert_eq!(
            model.apply_client_maximize_intent(win, false).unwrap(),
            ClientMaximizeIntentTransition {
                monitor_id,
                outcome: ClientMaximizeIntentOutcome::Placement {
                    placement: ClientPlacement::Floating,
                    changed: true,
                    visible_restore_rect: Some(floating),
                },
            }
        );
        assert!(model.client(win).unwrap().mode().is_normal_floating());
        assert_eq!(model.client(win).unwrap().geo, floating);
        assert_eq!(model.client_protocol_maximized(win), Some(false));
    }

    #[test]
    fn client_maximize_is_literal_in_the_global_floating_presentation() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = crate::layouts::PresentationMode::Floating;

        assert_eq!(
            model.apply_client_maximize_intent(win, true).unwrap(),
            ClientMaximizeIntentTransition {
                monitor_id,
                outcome: ClientMaximizeIntentOutcome::FloatingPresentation(
                    MaximizedChange::Entered {
                        work_rect: Rect::new(0, 0, 1920, 1080),
                    },
                ),
            }
        );
        assert!(model.client(win).unwrap().mode().is_maximized());
        assert_eq!(model.client_protocol_maximized(win), Some(true));
    }

    #[test]
    fn global_floating_restore_preserves_tiled_placement_and_visible_geometry() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::tiled());
        model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = crate::layouts::PresentationMode::Floating;
        let floating_rect = model.client(win).unwrap().geo;
        let _ = model.apply_client_maximize_intent(win, true).unwrap();
        model
            .client_mut(win)
            .unwrap()
            .update_geometry(Rect::new(0, 0, 1920, 1080));

        assert_eq!(
            model.apply_client_maximize_intent(win, false).unwrap(),
            ClientMaximizeIntentTransition {
                monitor_id,
                outcome: ClientMaximizeIntentOutcome::FloatingPresentation(
                    MaximizedChange::Exited {
                        restore_rect: Some(floating_rect),
                    },
                ),
            }
        );
        let client = model.client(win).unwrap();
        assert!(client.mode().is_normal_tiling());
        assert_eq!(client.geo, floating_rect);
        assert_eq!(model.client_protocol_maximized(win), Some(false));
    }

    #[test]
    fn fixed_size_client_rejects_tiling_maximize_intent() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        model.client_mut(win).unwrap().is_fixed_size = true;

        assert_eq!(
            model.apply_client_maximize_intent(win, true).unwrap(),
            ClientMaximizeIntentTransition {
                monitor_id,
                outcome: ClientMaximizeIntentOutcome::Rejected,
            }
        );
        assert!(model.client(win).unwrap().mode().is_normal_floating());
        assert_eq!(model.client_protocol_maximized(win), Some(false));
    }

    #[test]
    fn leaving_global_floating_collapses_client_maximize_into_tiling() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = crate::layouts::PresentationMode::Floating;
        let _ = model.apply_client_maximize_intent(win, true).unwrap();
        model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = crate::layouts::PresentationMode::Tiled;

        assert_eq!(
            model.reconcile_client_maximization_for_tiling(monitor_id),
            vec![win]
        );
        assert!(model.client(win).unwrap().mode().is_normal_tiling());
        assert_eq!(model.client_protocol_maximized(win), Some(true));
    }

    #[test]
    fn maximize_during_fullscreen_changes_the_tiling_restore_placement() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        let _ = model.set_fullscreen(win, true).unwrap();

        let transition = model.apply_client_maximize_intent(win, true).unwrap();

        assert_eq!(
            transition,
            ClientMaximizeIntentTransition {
                monitor_id,
                outcome: ClientMaximizeIntentOutcome::Placement {
                    placement: ClientPlacement::Tiling,
                    changed: true,
                    visible_restore_rect: None,
                },
            }
        );
        assert!(model.client(win).unwrap().mode().is_true_fullscreen());
        assert_eq!(
            model.set_fullscreen(win, false).unwrap(),
            FullscreenTransition {
                monitor_id,
                change: FullscreenChange::Exited { restore_rect: None },
            }
        );
    }

    #[test]
    fn leave_literal_client_maximize_restores_geometry() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::floating());
        model
            .monitor_mut(monitor_id)
            .unwrap()
            .per_tag_state()
            .presentation = crate::layouts::PresentationMode::Floating;
        let _ = model.apply_client_maximize_intent(win, true).unwrap();

        let left = model.leave_maximized(win).unwrap();

        assert!(matches!(
            left.change,
            MaximizedChange::Exited {
                restore_rect: Some(_)
            }
        ));
        assert!(model.client(win).unwrap().mode().is_normal_floating());
        assert_eq!(model.client_protocol_maximized(win), Some(false));
    }
}
