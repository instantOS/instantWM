//! Laptop lid suppression, independent of configuration storage and native APIs.

use std::collections::HashSet;

use super::{OutputHeadCapabilities, OutputId, OutputTransaction, OutputTransactionError};

/// Physical classification supplied by the backend for this output's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputConnectorKind {
    Internal,
    External,
}

/// Only the facts needed to decide whether a panel should be suppressed.
pub struct LidOutput {
    pub id: OutputId,
    pub connector: OutputConnectorKind,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LidOutputDecision {
    LidOpen,
    SuppressInternalDisplays,
}

pub struct LidOutputProjection {
    pub decision: LidOutputDecision,
    pub suppressed: HashSet<OutputId>,
}

impl LidOutputProjection {
    /// A valid requested layout may become entirely dark when the lid closes.
    /// Only lid suppression may disable its last enabled head; clients still
    /// cannot submit an all-disabled layout, and other validation errors stand.
    pub fn validate_transaction(
        &self,
        requested: &OutputTransaction,
        effective: &OutputTransaction,
        capabilities: &[OutputHeadCapabilities],
    ) -> Result<(), OutputTransactionError> {
        requested.validate(capabilities)?;
        match effective.validate(capabilities) {
            Err(OutputTransactionError::NoEnabledOutputs)
                if requested
                    .heads
                    .iter()
                    .filter(|head| head.enabled)
                    .all(|head| self.suppressed.contains(&head.id)) =>
            {
                Ok(())
            }
            result => result,
        }
    }
}

/// Owns only observed lid state. Output identity and configuration are supplied
/// on every evaluation, so hotplug cannot leave stale classification here.
#[derive(Debug, Default, Clone)]
pub struct LidOutputPolicy {
    closed: bool,
}

impl LidOutputPolicy {
    pub fn set_closed(&mut self, closed: bool) -> bool {
        let changed = self.closed != closed;
        self.closed = closed;
        changed
    }

    /// Keep internal displays off until the lid opens, including after undocking.
    /// Native restoration and applying this result belong to the backend.
    pub fn evaluate(&self, outputs: impl IntoIterator<Item = LidOutput>) -> LidOutputProjection {
        let mut internal = HashSet::new();
        for output in outputs {
            if output.connector == OutputConnectorKind::Internal && output.enabled {
                internal.insert(output.id);
            }
        }
        let decision = if !self.closed {
            LidOutputDecision::LidOpen
        } else {
            LidOutputDecision::SuppressInternalDisplays
        };
        if decision != LidOutputDecision::SuppressInternalDisplays {
            internal.clear();
        }
        LidOutputProjection {
            decision,
            suppressed: internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(enabled: bool) -> LidOutput {
        LidOutput {
            id: "panel".into(),
            connector: OutputConnectorKind::Internal,
            enabled,
        }
    }

    fn dock(enabled: bool) -> LidOutput {
        LidOutput {
            id: "dock".into(),
            connector: OutputConnectorKind::External,
            enabled,
        }
    }

    #[test]
    fn closing_and_opening_only_suppresses_enabled_internal_outputs() {
        let mut policy = LidOutputPolicy::default();
        assert!(
            policy
                .evaluate([panel(true), dock(true)])
                .suppressed
                .is_empty()
        );
        assert!(policy.set_closed(true));
        assert!(!policy.set_closed(true));
        let closed = policy.evaluate([panel(true), dock(true)]);
        assert_eq!(closed.decision, LidOutputDecision::SuppressInternalDisplays);
        assert_eq!(closed.suppressed, HashSet::from(["panel".into()]));
        assert!(
            policy
                .evaluate([panel(false), dock(true)])
                .suppressed
                .is_empty()
        );
        policy.set_closed(false);
        assert_eq!(
            policy.evaluate([panel(true), dock(true)]).decision,
            LidOutputDecision::LidOpen
        );
    }

    #[test]
    fn disabled_or_absent_external_outputs_do_not_wake_a_closed_panel() {
        let mut policy = LidOutputPolicy::default();
        policy.set_closed(true);
        for outputs in [vec![panel(true)], vec![panel(true), dock(false)]] {
            let result = policy.evaluate(outputs);
            assert_eq!(result.decision, LidOutputDecision::SuppressInternalDisplays);
            assert_eq!(result.suppressed, HashSet::from(["panel".into()]));
        }
    }

    #[test]
    fn hotplug_and_reclassification_use_only_the_supplied_snapshot() {
        let mut policy = LidOutputPolicy::default();
        policy.set_closed(true);
        assert!(
            policy
                .evaluate([panel(true)])
                .suppressed
                .contains(&"panel".into())
        );
        assert!(
            policy
                .evaluate([panel(true), dock(true)])
                .suppressed
                .contains(&"panel".into())
        );
        let mut replacement = panel(true);
        replacement.connector = OutputConnectorKind::External;
        assert!(
            policy
                .evaluate([replacement, dock(true)])
                .suppressed
                .is_empty()
        );
        assert!(
            policy
                .evaluate([panel(true), dock(true)])
                .suppressed
                .contains(&"panel".into())
        );
    }
}
