//! Geometry intent and configure/commit correlation for one window.
//!
//! A scheduled resize already owns geometry, even before an animation sends
//! it. The previous configure is retained while scheduling its replacement:
//! cancelling an unsent resize must not forget a still-live protocol request.
//! Animation state only controls dispatch timing; it never grants geometry
//! authority. Core policy decides whether a current response may constrain
//! the requested size.

use smithay::utils::Serial;

use crate::geometry::{GeometryResponse, reconcile_geometry_commit};
use crate::types::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Configure {
    size: Size,
    serial: Option<Serial>,
    acknowledged: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::backend::wayland::compositor) struct WindowGeometrySync {
    scheduled: Option<Size>,
    configured: Option<Configure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::backend::wayland::compositor) struct CommitDecision {
    pub accept_client_size: bool,
    pub needs_dispatch: bool,
}

impl WindowGeometrySync {
    /// Replace the desired size. Returns whether it still needs dispatch.
    /// A return to the last configured size cancels an unsent replacement,
    /// preserving the original configure and its acknowledgement state.
    pub fn schedule(&mut self, size: Size) -> bool {
        self.scheduled = (self.configured_size() != Some(size)).then_some(size);
        self.scheduled.is_some()
    }

    pub fn scheduled_size(&self) -> Option<Size> {
        self.scheduled
    }

    pub fn configured_size(&self) -> Option<Size> {
        self.configured.map(|configure| configure.size)
    }

    /// Only actual protocol dispatch advances scheduled -> sent. X11 has
    /// no configure acknowledgement and uses `None` for the serial.
    pub fn sent(&mut self, size: Size, serial: Option<Serial>) {
        self.scheduled = None;
        self.configured = Some(Configure {
            size,
            serial,
            acknowledged: serial.is_none(),
        });
    }

    fn response(&self, acknowledged: Option<Serial>) -> GeometryResponse {
        if self.scheduled.is_some() {
            return GeometryResponse::Stale;
        }
        let Some(configure) = self.configured else {
            return GeometryResponse::Unsolicited;
        };
        if let Some(requested) = configure.serial
            && !acknowledged.is_some_and(|answered| answered.is_no_older_than(&requested))
        {
            return GeometryResponse::Stale;
        }
        if configure.acknowledged {
            GeometryResponse::Unsolicited
        } else {
            GeometryResponse::Current
        }
    }

    pub fn observe(
        &mut self,
        actual: Size,
        acknowledged: Option<Serial>,
        client_size_is_authoritative: bool,
    ) -> CommitDecision {
        let decision =
            reconcile_geometry_commit(self.response(acknowledged), client_size_is_authoritative);
        if decision.settle_request
            && let Some(configure) = self.configured.as_mut()
        {
            configure.acknowledged = true;
        }
        // A constrained current reply is legitimate. Converge protocol state
        // to the accepted size through the same schedule/dispatch lifecycle.
        let needs_dispatch = decision.accept_client_size && self.schedule(actual);
        CommitDecision {
            accept_client_size: decision.accept_client_size,
            needs_dispatch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILED: Size = Size::new(1920, 1080);
    const FLOATING: Size = Size::new(640, 360);
    const CONSTRAINED: Size = Size::new(640, 352);

    fn serial(value: u32) -> Option<Serial> {
        Some(Serial::from(value))
    }

    #[test]
    fn scheduled_intent_rejects_both_redraws_and_previous_configure_replies() {
        for previous_acknowledged in [false, true] {
            let mut sync = WindowGeometrySync::default();
            sync.sent(TILED, serial(1));
            if previous_acknowledged {
                sync.observe(TILED, serial(1), false);
            }
            assert!(sync.schedule(FLOATING));
            for acknowledged in [None, serial(1)] {
                assert_eq!(
                    sync.observe(TILED, acknowledged, true),
                    CommitDecision {
                        accept_client_size: false,
                        needs_dispatch: false,
                    }
                );
                assert_eq!(sync.scheduled_size(), Some(FLOATING));
            }
        }
    }

    #[test]
    fn current_reply_can_constrain_a_restore_then_converges_once() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(TILED, serial(1));
        sync.schedule(FLOATING);
        sync.sent(FLOATING, serial(2));
        assert!(!sync.observe(TILED, serial(1), true).accept_client_size);
        assert_eq!(
            sync.observe(CONSTRAINED, serial(2), true),
            CommitDecision {
                accept_client_size: true,
                needs_dispatch: true,
            }
        );
        assert_eq!(sync.scheduled_size(), Some(CONSTRAINED));
        // No feedback may undo the accepted size before convergence dispatch.
        assert!(!sync.observe(TILED, serial(2), true).accept_client_size);
        sync.sent(CONSTRAINED, serial(3));
        assert_eq!(
            sync.observe(CONSTRAINED, serial(3), true),
            CommitDecision {
                accept_client_size: true,
                needs_dispatch: false,
            }
        );
        assert!(!sync.schedule(CONSTRAINED));
    }

    #[test]
    fn layout_owned_reply_settles_without_resizing_or_retrying() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(TILED, serial(1));
        let decision = sync.observe(CONSTRAINED, serial(1), false);
        assert!(!decision.accept_client_size);
        assert!(!decision.needs_dispatch);
        assert_eq!(sync.response(serial(1)), GeometryResponse::Unsolicited);
        assert_eq!(sync.configured_size(), Some(TILED));
        assert!(!sync.schedule(TILED));
    }

    #[test]
    fn acknowledged_request_still_rejects_older_queued_observations() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(FLOATING, serial(2));
        assert!(sync.observe(FLOATING, serial(2), true).accept_client_size);
        assert!(!sync.observe(TILED, serial(1), true).accept_client_size);
        assert!(!sync.observe(TILED, None, true).accept_client_size);
        assert_eq!(sync.scheduled_size(), None);
    }

    #[test]
    fn same_size_requests_are_correlated_by_serial() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(FLOATING, serial(1));
        sync.sent(FLOATING, serial(2));
        assert!(!sync.observe(FLOATING, serial(1), true).accept_client_size);
        assert_eq!(sync.response(serial(2)), GeometryResponse::Current);
        assert!(sync.observe(FLOATING, serial(2), true).accept_client_size);
    }

    #[test]
    fn cancelling_unsent_replacement_preserves_the_live_configure() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(TILED, serial(1));
        sync.schedule(FLOATING);
        assert!(!sync.schedule(TILED));
        assert_eq!(sync.response(serial(1)), GeometryResponse::Current);
        assert!(!sync.observe(TILED, None, true).accept_client_size);
        assert!(sync.observe(TILED, serial(1), true).accept_client_size);
    }

    #[test]
    fn retargeting_replaces_unsent_intent_and_rejects_superseded_replies() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(TILED, serial(1));
        sync.schedule(FLOATING);
        sync.schedule(CONSTRAINED);
        assert_eq!(sync.scheduled_size(), Some(CONSTRAINED));
        sync.sent(CONSTRAINED, serial(2));
        sync.schedule(FLOATING);
        assert!(
            !sync
                .observe(CONSTRAINED, serial(2), true)
                .accept_client_size
        );
        sync.sent(FLOATING, serial(3));
        assert!(
            !sync
                .observe(CONSTRAINED, serial(2), true)
                .accept_client_size
        );
        assert!(sync.observe(FLOATING, serial(3), true).accept_client_size);
    }

    #[test]
    fn client_originated_resize_uses_the_same_convergence_lifecycle() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(FLOATING, serial(1));
        sync.observe(FLOATING, serial(1), true);
        assert_eq!(
            sync.observe(CONSTRAINED, serial(1), true),
            CommitDecision {
                accept_client_size: true,
                needs_dispatch: true,
            }
        );
        assert_eq!(sync.scheduled_size(), Some(CONSTRAINED));
    }

    #[test]
    fn state_only_newer_configure_and_serial_wrap_can_answer_a_size_request() {
        let mut sync = WindowGeometrySync::default();
        sync.sent(FLOATING, serial(u32::MAX));
        assert!(
            !sync
                .observe(TILED, serial(u32::MAX - 1), true)
                .accept_client_size
        );
        assert!(sync.observe(FLOATING, serial(1), true).accept_client_size);
    }

    #[test]
    fn x11_dispatch_has_no_acknowledgement_to_wait_for() {
        let mut sync = WindowGeometrySync::default();
        sync.schedule(FLOATING);
        sync.sent(FLOATING, None);
        assert_eq!(sync.scheduled_size(), None);
        assert!(!sync.schedule(FLOATING));
        assert_eq!(sync.response(None), GeometryResponse::Unsolicited);
    }
}
