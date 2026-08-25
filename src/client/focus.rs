//! Shared focus state (no X11 types).

use crate::types::WindowId;

/// One committed change to the globally selected window.
///
/// This is a domain transition, not a backend command. Backends may project it
/// into their own protocols after the shared transaction has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionTransition {
    pub previous: Option<WindowId>,
    pub current: Option<WindowId>,
}

impl SelectionTransition {
    pub fn new(previous: Option<WindowId>, current: Option<WindowId>) -> Option<Self> {
        (previous != current).then_some(Self { previous, current })
    }

    pub fn merge(self, next: Self) -> Option<Self> {
        debug_assert_eq!(
            self.current, next.previous,
            "selection transitions must form a continuous transaction"
        );
        Self::new(self.previous, next.current)
    }
}

/// Non-backend-specific focus tracking state.
#[derive(Default)]
pub struct FocusState {
    /// The previously focused window (0 = none), used by focus-last-client logic.
    pub last_client: WindowId,
    /// Selection changes accumulated since the last shared runtime tick.
    pending_selection: Option<SelectionTransition>,
}

impl FocusState {
    /// Record a committed global selection change.
    ///
    /// Multiple changes in one tick coalesce to the externally observable
    /// transition from the first previous window to the final current window.
    pub fn record_selection(
        &mut self,
        previous: Option<WindowId>,
        current: Option<WindowId>,
    ) -> Option<SelectionTransition> {
        let transition = SelectionTransition::new(previous, current)?;
        self.pending_selection = match self.pending_selection {
            Some(pending) => pending.merge(transition),
            None => Some(transition),
        };
        Some(transition)
    }

    pub fn take_pending_selection(&mut self) -> Option<SelectionTransition> {
        self.pending_selection.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_selection_coalesces_to_the_tick_boundary() {
        let mut state = FocusState::default();
        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);

        state.record_selection(Some(a), Some(b));
        state.record_selection(Some(b), Some(c));

        assert_eq!(
            state.take_pending_selection(),
            Some(SelectionTransition {
                previous: Some(a),
                current: Some(c),
            })
        );
    }

    #[test]
    fn a_round_trip_within_one_tick_has_no_external_transition() {
        let mut state = FocusState::default();
        let a = WindowId(1);
        let b = WindowId(2);

        state.record_selection(Some(a), Some(b));
        state.record_selection(Some(b), Some(a));

        assert_eq!(state.take_pending_selection(), None);
    }
}
