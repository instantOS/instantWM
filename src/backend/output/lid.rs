//! Laptop lid suppression, independent of configuration storage and native APIs.

use std::collections::HashSet;

use super::OutputId;

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
    pub has_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LidOutputDecision {
    LidOpen,
    KeepLastDisplay,
    SuppressInternalDisplays,
}

pub struct LidOutputProjection {
    pub decision: LidOutputDecision,
    pub suppressed: HashSet<OutputId>,
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

    /// Keep the last usable display configured for recovery after undocking.
    /// Native restoration and applying this result belong to the backend.
    pub fn evaluate(&self, outputs: impl IntoIterator<Item = LidOutput>) -> LidOutputProjection {
        let mut internal = HashSet::new();
        let mut usable_external = false;
        for output in outputs {
            match output.connector {
                OutputConnectorKind::Internal if output.enabled => {
                    internal.insert(output.id);
                }
                OutputConnectorKind::External => {
                    usable_external |= output.enabled && output.has_mode;
                }
                _ => {}
            }
        }
        let decision = if !self.closed {
            LidOutputDecision::LidOpen
        } else if usable_external {
            LidOutputDecision::SuppressInternalDisplays
        } else {
            LidOutputDecision::KeepLastDisplay
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
            has_mode: true,
        }
    }

    fn dock(enabled: bool, has_mode: bool) -> LidOutput {
        LidOutput {
            id: "dock".into(),
            connector: OutputConnectorKind::External,
            enabled,
            has_mode,
        }
    }

    #[test]
    fn closing_and_opening_only_suppresses_enabled_internal_outputs() {
        let mut policy = LidOutputPolicy::default();
        assert!(
            policy
                .evaluate([panel(true), dock(true, true)])
                .suppressed
                .is_empty()
        );
        assert!(policy.set_closed(true));
        assert!(!policy.set_closed(true));
        let closed = policy.evaluate([panel(true), dock(true, true)]);
        assert_eq!(closed.decision, LidOutputDecision::SuppressInternalDisplays);
        assert_eq!(closed.suppressed, HashSet::from(["panel".into()]));
        assert!(
            policy
                .evaluate([panel(false), dock(true, true)])
                .suppressed
                .is_empty()
        );
        policy.set_closed(false);
        assert_eq!(
            policy.evaluate([panel(true), dock(true, true)]).decision,
            LidOutputDecision::LidOpen
        );
    }

    #[test]
    fn disabled_modeless_or_absent_external_outputs_keep_the_last_display() {
        let mut policy = LidOutputPolicy::default();
        policy.set_closed(true);
        for outputs in [
            vec![panel(true)],
            vec![panel(true), dock(false, true)],
            vec![panel(true), dock(true, false)],
        ] {
            let result = policy.evaluate(outputs);
            assert_eq!(result.decision, LidOutputDecision::KeepLastDisplay);
            assert!(result.suppressed.is_empty());
        }
    }

    #[test]
    fn hotplug_and_reclassification_use_only_the_supplied_snapshot() {
        let mut policy = LidOutputPolicy::default();
        policy.set_closed(true);
        assert!(policy.evaluate([panel(true)]).suppressed.is_empty());
        assert!(
            policy
                .evaluate([panel(true), dock(true, true)])
                .suppressed
                .contains(&"panel".into())
        );
        let mut replacement = panel(true);
        replacement.connector = OutputConnectorKind::External;
        assert!(
            policy
                .evaluate([replacement, dock(true, true)])
                .suppressed
                .is_empty()
        );
        assert!(
            policy
                .evaluate([panel(true), dock(true, true)])
                .suppressed
                .contains(&"panel".into())
        );
    }
}
