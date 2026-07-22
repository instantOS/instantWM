//! Backend-neutral presentation-mode transactions.
//!
//! Each transaction resolves the client and its monitor exactly once, commits
//! the complete authoritative model change, and returns an owned snapshot for
//! backend I/O, layout scheduling, and animation after the model borrow ends.

use crate::model::WmModel;
use crate::types::{BaseClientMode, Client, ClientMode, MonitorId, Rect, WindowId};

/// Commit only the client-local portion of a fullscreen transition.
///
/// Model transactions use this directly, and compound policy transactions may
/// reuse it while already holding the sole mutable client borrow. Keeping the
/// state-machine operation here prevents those transactions from duplicating
/// fullscreen semantics or looking the client up again.
pub(crate) fn set_client_fullscreen(client: &mut Client, fullscreen: bool) -> (ClientMode, bool) {
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
            client.restore_border_width();
        }
    }

    (previous_mode, changed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionKind {
    Entered,
    Exited,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the transition contains required backend and scheduling work"]
pub(crate) struct FullscreenTransition {
    monitor_id: MonitorId,
    monitor_rect: Rect,
    previous_mode: ClientMode,
    old_geo: Rect,
    kind: TransitionKind,
}

impl FullscreenTransition {
    #[inline]
    pub(crate) fn changed(self) -> bool {
        self.kind != TransitionKind::Unchanged
    }

    #[inline]
    pub(crate) fn entered(self) -> bool {
        self.kind == TransitionKind::Entered
    }

    #[inline]
    pub(crate) fn exited(self) -> bool {
        self.kind == TransitionKind::Exited
    }

    #[inline]
    pub(crate) fn monitor_id(self) -> MonitorId {
        self.monitor_id
    }

    #[inline]
    pub(crate) fn monitor_rect(self) -> Rect {
        self.monitor_rect
    }

    #[inline]
    pub(crate) fn old_geo(self) -> Rect {
        self.old_geo
    }

    #[inline]
    pub(crate) fn was_fake_fullscreen(self) -> bool {
        self.previous_mode.is_fake_fullscreen()
    }

    #[inline]
    pub(crate) fn was_floating(self) -> bool {
        self.previous_mode.is_floating()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "the transition contains required geometry and scheduling work"]
pub(crate) struct MaximizedTransition {
    monitor_id: MonitorId,
    work_rect: Rect,
    previous_mode: ClientMode,
    restore_rect: Rect,
    kind: TransitionKind,
}

impl MaximizedTransition {
    #[inline]
    pub(crate) fn entered(self) -> bool {
        self.kind == TransitionKind::Entered
    }

    #[inline]
    pub(crate) fn exited(self) -> bool {
        self.kind == TransitionKind::Exited
    }

    #[inline]
    pub(crate) fn monitor_id(self) -> MonitorId {
        self.monitor_id
    }

    #[inline]
    pub(crate) fn work_rect(self) -> Rect {
        self.work_rect
    }

    #[inline]
    pub(crate) fn restore_rect(self) -> Rect {
        self.restore_rect
    }

    #[inline]
    pub(crate) fn restore_base(self) -> BaseClientMode {
        self.previous_mode.base_mode()
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

        let (previous_mode, changed) = set_client_fullscreen(client, fullscreen);

        let kind = match (changed, fullscreen) {
            (true, true) => TransitionKind::Entered,
            (true, false) => TransitionKind::Exited,
            (false, _) => TransitionKind::Unchanged,
        };
        Some(FullscreenTransition {
            monitor_id: client.monitor_id,
            monitor_rect: monitor.monitor_rect,
            previous_mode,
            old_geo: client.old_geo,
            kind,
        })
    }

    /// Set maximized presentation and return the complete backend plan.
    pub(crate) fn set_maximized(
        &mut self,
        win: WindowId,
        maximized: bool,
    ) -> Option<MaximizedTransition> {
        let clients = &mut self.clients;
        let monitors = &self.monitors;
        let client = clients.get_mut(&win)?;
        let monitor = monitors.get(client.monitor_id)?;

        let previous_mode = client.mode();
        let changed = if maximized {
            !previous_mode.is_maximized()
        } else {
            previous_mode.is_maximized()
        };

        if changed {
            if maximized {
                if !previous_mode.is_floating() {
                    client.float_geo = client.geo;
                }
                client.enter_maximized();
            } else {
                client.restore_mode();
            }
        }

        let kind = match (changed, maximized) {
            (true, true) => TransitionKind::Entered,
            (true, false) => TransitionKind::Exited,
            (false, _) => TransitionKind::Unchanged,
        };
        Some(MaximizedTransition {
            monitor_id: client.monitor_id,
            work_rect: monitor.work_rect(),
            previous_mode,
            restore_rect: client.float_geo,
            kind,
        })
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
        let (mut model, win, monitor_id) = model_with_client(ClientMode::Tiling);

        let transition = model.set_fullscreen(win, true).unwrap();

        assert!(transition.entered());
        assert_eq!(transition.monitor_id(), monitor_id);
        assert_eq!(transition.monitor_rect(), Rect::new(0, 0, 1920, 1080));
        assert_eq!(transition.old_geo(), Rect::new(30, 40, 640, 480));
        let client = model.client(win).unwrap();
        assert!(client.mode().is_true_fullscreen());
        assert_eq!(client.border_width, 0);
        assert_eq!(client.old_border_width, 2);
    }

    #[test]
    fn fullscreen_transaction_is_idempotent() {
        let (mut model, win, _) = model_with_client(ClientMode::Tiling);
        assert!(model.set_fullscreen(win, true).unwrap().changed());
        assert!(!model.set_fullscreen(win, true).unwrap().changed());
    }

    #[test]
    fn unfullscreen_transaction_restores_base_mode_and_border() {
        let (mut model, win, _) = model_with_client(ClientMode::Tiling);
        let _ = model.set_fullscreen(win, true).unwrap();

        let transition = model.set_fullscreen(win, false).unwrap();

        assert!(transition.exited());
        let client = model.client(win).unwrap();
        assert!(client.mode().is_tiling());
        assert_eq!(client.border_width, 2);
    }

    #[test]
    fn real_fullscreen_request_promotes_fake_fullscreen() {
        let (mut model, win, _) = model_with_client(ClientMode::Tiling.as_fake_fullscreen());

        let transition = model.set_fullscreen(win, true).unwrap();

        assert!(transition.changed());
        assert!(transition.was_fake_fullscreen());
        assert!(model.client(win).unwrap().mode().is_true_fullscreen());
    }

    #[test]
    fn maximize_transaction_returns_work_and_restore_geometry() {
        let (mut model, win, monitor_id) = model_with_client(ClientMode::Tiling);

        let transition = model.set_maximized(win, true).unwrap();

        assert!(transition.entered());
        assert_eq!(transition.monitor_id(), monitor_id);
        assert_eq!(transition.restore_rect(), Rect::new(10, 20, 800, 600));
        assert!(model.client(win).unwrap().mode().is_maximized());
    }

    #[test]
    fn unrelated_presentation_is_not_destroyed_by_unmaximize() {
        let (mut model, win, _) = model_with_client(ClientMode::Tiling.as_fullscreen());

        let transition = model.set_maximized(win, false).unwrap();

        assert_eq!(transition.kind, TransitionKind::Unchanged);
        assert!(model.client(win).unwrap().mode().is_true_fullscreen());
    }
}
