//! Backend-neutral output configuration transactions.
//!
//! Producers submit complete desired output configurations. The active backend
//! validates or commits them asynchronously and returns an authoritative
//! snapshot. Protocol objects and backend-native handles deliberately do not
//! cross this boundary.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::types::Point;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutputId(pub String);

impl From<String> for OutputId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for OutputId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputMode {
    pub width: i32,
    pub height: i32,
    pub refresh_millihertz: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputTransform {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdaptiveSyncPolicy {
    Disabled,
    Enabled,
    Automatic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputHeadConfiguration {
    pub id: OutputId,
    pub enabled: bool,
    pub mode: Option<OutputMode>,
    pub position: Point,
    pub transform: OutputTransform,
    pub scale: f64,
    /// `None` preserves the backend's current policy.
    pub adaptive_sync: Option<AdaptiveSyncPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTransactionKind {
    Test,
    Apply,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputTransaction {
    pub heads: Vec<OutputHeadConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputHeadCapabilities {
    pub id: OutputId,
    pub modes: Vec<OutputMode>,
    pub adaptive_sync: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputHeadSnapshot {
    pub configuration: OutputHeadConfiguration,
    pub modes: Vec<OutputMode>,
    pub adaptive_sync_policy: AdaptiveSyncPolicy,
    pub adaptive_sync_enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSnapshot {
    pub heads: Vec<OutputHeadSnapshot>,
}

/// Physical power state of a configured output.
///
/// This is deliberately separate from [`OutputHeadConfiguration::enabled`]:
/// powering an output off keeps it in the compositor's logical output space,
/// while disabling an output removes it from that space entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPowerMode {
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputPowerRequestId(u64);

#[derive(Debug, Clone)]
pub struct PendingOutputPowerRequest {
    pub id: OutputPowerRequestId,
    pub output: OutputId,
    pub mode: OutputPowerMode,
}

#[derive(Debug, Clone)]
pub struct CompletedOutputPowerRequest {
    pub id: OutputPowerRequestId,
    pub output: OutputId,
    pub result: Result<OutputPowerMode, OutputPowerError>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputPowerError {
    #[error("output {0} is not available for power management")]
    Unavailable(String),
    #[error("backend rejected the output power change: {0}")]
    Backend(String),
}

/// Backend-neutral queue joining protocol requests to backend-native DPMS.
#[derive(Debug, Default)]
pub struct OutputPowerService {
    next_id: u64,
    pending: VecDeque<PendingOutputPowerRequest>,
    completed: VecDeque<CompletedOutputPowerRequest>,
}

impl OutputPowerService {
    pub fn submit(&mut self, output: OutputId, mode: OutputPowerMode) -> OutputPowerRequestId {
        self.next_id = self.next_id.wrapping_add(1);
        let id = OutputPowerRequestId(self.next_id);
        self.pending
            .push_back(PendingOutputPowerRequest { id, output, mode });
        id
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn take_next_pending(&mut self) -> Option<PendingOutputPowerRequest> {
        self.pending.pop_front()
    }

    pub fn requeue(&mut self, request: PendingOutputPowerRequest) {
        self.pending.push_front(request);
    }

    pub fn cancel(&mut self, ids: &[OutputPowerRequestId]) {
        self.pending.retain(|request| !ids.contains(&request.id));
    }

    pub fn complete(
        &mut self,
        request: PendingOutputPowerRequest,
        result: Result<OutputPowerMode, OutputPowerError>,
    ) {
        self.completed.push_back(CompletedOutputPowerRequest {
            id: request.id,
            output: request.output,
            result,
        });
    }

    pub fn complete_by_id(
        &mut self,
        id: OutputPowerRequestId,
        output: OutputId,
        result: Result<OutputPowerMode, OutputPowerError>,
    ) {
        self.completed
            .push_back(CompletedOutputPowerRequest { id, output, result });
    }

    pub fn take_completed(&mut self) -> Vec<CompletedOutputPowerRequest> {
        self.completed.drain(..).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputTransactionError {
    #[error("configuration omitted output {0}")]
    MissingOutput(String),
    #[error("configuration referenced unknown output {0}")]
    UnknownOutput(String),
    #[error("configuration contains output {0} more than once")]
    DuplicateOutput(String),
    #[error("configuration would disable every output")]
    NoEnabledOutputs,
    #[error("output {0} has no selected mode")]
    MissingMode(String),
    #[error("output {output} does not support mode {mode:?}")]
    UnsupportedMode { output: String, mode: OutputMode },
    #[error("output {0} does not support adaptive sync")]
    AdaptiveSyncUnsupported(String),
    #[error("output {0} has an invalid scale")]
    InvalidScale(String),
    #[error("backend rejected the output configuration: {0}")]
    Backend(String),
}

impl OutputTransaction {
    /// Validate backend-independent transaction invariants against the current
    /// set of outputs. Backends may perform additional test-only checks before
    /// committing, but should not duplicate these structural rules.
    pub fn validate(
        &self,
        capabilities: &[OutputHeadCapabilities],
    ) -> Result<(), OutputTransactionError> {
        let available: HashMap<_, _> = capabilities
            .iter()
            .map(|capability| (&capability.id, capability))
            .collect();
        let mut configured = HashSet::new();

        for head in &self.heads {
            if !configured.insert(&head.id) {
                return Err(OutputTransactionError::DuplicateOutput(head.id.0.clone()));
            }
            let Some(capability) = available.get(&head.id) else {
                return Err(OutputTransactionError::UnknownOutput(head.id.0.clone()));
            };
            if !head.scale.is_finite() || head.scale <= 0.0 {
                return Err(OutputTransactionError::InvalidScale(head.id.0.clone()));
            }
            if !head.enabled {
                continue;
            }
            let Some(mode) = head.mode else {
                return Err(OutputTransactionError::MissingMode(head.id.0.clone()));
            };
            if !capability.modes.contains(&mode) {
                return Err(OutputTransactionError::UnsupportedMode {
                    output: head.id.0.clone(),
                    mode,
                });
            }
            // Automatic is a policy, not a demand that the backend enable
            // adaptive sync.  It must remain valid on unsupported hardware so
            // that an unrelated mode, position, or scale update can preserve
            // the compositor's default policy.  Only an explicit request to
            // enable adaptive sync requires backend support.
            if !capability.adaptive_sync
                && matches!(head.adaptive_sync, Some(AdaptiveSyncPolicy::Enabled))
            {
                return Err(OutputTransactionError::AdaptiveSyncUnsupported(
                    head.id.0.clone(),
                ));
            }
        }

        for capability in capabilities {
            if !configured.contains(&capability.id) {
                return Err(OutputTransactionError::MissingOutput(
                    capability.id.0.clone(),
                ));
            }
        }
        if !self.heads.iter().any(|head| head.enabled) {
            return Err(OutputTransactionError::NoEnabledOutputs);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputTransactionId(u64);

#[derive(Debug, Clone)]
pub struct PendingOutputTransaction {
    pub id: OutputTransactionId,
    pub kind: OutputTransactionKind,
    pub transaction: OutputTransaction,
    coalescible: bool,
}

#[derive(Debug, Clone)]
pub struct CompletedOutputTransaction {
    pub id: OutputTransactionId,
    pub kind: OutputTransactionKind,
    pub result: Result<OutputSnapshot, OutputTransactionError>,
}

#[derive(Debug, Default)]
pub struct OutputTransactionService {
    next_id: u64,
    pending: VecDeque<PendingOutputTransaction>,
    completed: VecDeque<CompletedOutputTransaction>,
}

impl OutputTransactionService {
    #[inline]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn submit(
        &mut self,
        kind: OutputTransactionKind,
        transaction: OutputTransaction,
    ) -> OutputTransactionId {
        self.next_id = self.next_id.wrapping_add(1);
        let id = OutputTransactionId(self.next_id);
        self.pending.push_back(PendingOutputTransaction {
            id,
            kind,
            transaction,
            coalescible: false,
        });
        id
    }

    /// Queue compositor policy/configuration state. Consecutive policy writes
    /// are collapsed because only their final desired state is observable.
    /// Client transactions submitted through [`Self::submit`] are never
    /// coalesced and retain one completion per request.
    pub fn submit_coalescing_apply(
        &mut self,
        transaction: OutputTransaction,
    ) -> OutputTransactionId {
        if let Some(pending) = self.pending.back_mut()
            && pending.kind == OutputTransactionKind::Apply
            && pending.coalescible
        {
            pending.transaction = transaction;
            return pending.id;
        }
        self.next_id = self.next_id.wrapping_add(1);
        let id = OutputTransactionId(self.next_id);
        self.pending.push_back(PendingOutputTransaction {
            id,
            kind: OutputTransactionKind::Apply,
            transaction,
            coalescible: true,
        });
        id
    }

    pub fn take_next_pending(&mut self) -> Option<PendingOutputTransaction> {
        self.pending.pop_front()
    }

    pub fn latest_pending_apply(&self) -> Option<&OutputTransaction> {
        self.pending
            .iter()
            .rev()
            .find(|pending| pending.kind == OutputTransactionKind::Apply)
            .map(|pending| &pending.transaction)
    }

    pub fn requeue(&mut self, transaction: PendingOutputTransaction) {
        self.pending.push_front(transaction);
    }

    pub fn complete(
        &mut self,
        transaction: PendingOutputTransaction,
        result: Result<OutputSnapshot, OutputTransactionError>,
    ) {
        self.completed.push_back(CompletedOutputTransaction {
            id: transaction.id,
            kind: transaction.kind,
            result,
        });
    }

    pub fn take_completed(&mut self) -> Vec<CompletedOutputTransaction> {
        self.completed.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction() -> OutputTransaction {
        OutputTransaction { heads: Vec::new() }
    }

    fn head(id: &str) -> OutputHeadConfiguration {
        OutputHeadConfiguration {
            id: id.into(),
            enabled: true,
            mode: Some(OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 60_000,
            }),
            position: Point::new(0, 0),
            transform: OutputTransform::Normal,
            scale: 1.0,
            adaptive_sync: Some(AdaptiveSyncPolicy::Disabled),
        }
    }

    fn capability(id: &str) -> OutputHeadCapabilities {
        OutputHeadCapabilities {
            id: id.into(),
            modes: vec![OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 60_000,
            }],
            adaptive_sync: false,
        }
    }

    #[test]
    fn service_preserves_submission_order_and_identity() {
        let mut service = OutputTransactionService::default();
        assert!(!service.has_pending());
        let first = service.submit(OutputTransactionKind::Test, transaction());
        let second = service.submit(OutputTransactionKind::Apply, transaction());
        assert!(service.has_pending());
        assert_eq!(service.take_next_pending().unwrap().id, first);
        assert!(service.has_pending());
        assert_eq!(service.take_next_pending().unwrap().id, second);
        assert!(!service.has_pending());
        assert!(service.take_next_pending().is_none());
    }

    #[test]
    fn requeued_transactions_remain_pending_without_completing() {
        let mut service = OutputTransactionService::default();
        service.submit(OutputTransactionKind::Apply, transaction());
        let pending = service.take_next_pending().unwrap();
        service.requeue(pending);

        assert!(service.take_next_pending().is_some());
        assert!(service.take_next_pending().is_none());
        assert!(service.take_completed().is_empty());
    }

    #[test]
    fn deferred_transaction_keeps_priority_over_later_work() {
        let mut service = OutputTransactionService::default();
        let first = service.submit(OutputTransactionKind::Apply, transaction());
        let second = service.submit(OutputTransactionKind::Apply, transaction());
        let pending = service.take_next_pending().unwrap();
        service.requeue(pending);

        assert_eq!(service.take_next_pending().unwrap().id, first);
        assert_eq!(service.take_next_pending().unwrap().id, second);
    }

    #[test]
    fn completion_retains_kind_and_result() {
        let mut service = OutputTransactionService::default();
        service.submit(OutputTransactionKind::Test, transaction());
        let pending = service.take_next_pending().unwrap();
        service.complete(pending, Ok(OutputSnapshot { heads: Vec::new() }));
        let completed = service.take_completed().pop().unwrap();

        assert_eq!(completed.kind, OutputTransactionKind::Test);
        assert!(completed.result.is_ok());
    }

    #[test]
    fn policy_updates_coalesce_but_client_requests_do_not() {
        let mut service = OutputTransactionService::default();
        let policy_id = service.submit_coalescing_apply(transaction());
        assert_eq!(service.submit_coalescing_apply(transaction()), policy_id);
        service.submit(OutputTransactionKind::Apply, transaction());

        assert!(service.take_next_pending().is_some());
        assert!(service.take_next_pending().is_some());
        assert!(service.take_next_pending().is_none());
    }

    #[test]
    fn validation_requires_one_configuration_per_available_output() {
        let mut one = head("one");
        let transaction = OutputTransaction {
            heads: vec![one.clone()],
        };
        assert_eq!(
            transaction.validate(&[capability("one"), capability("two")]),
            Err(OutputTransactionError::MissingOutput("two".into()))
        );

        one.id = "unknown".into();
        assert_eq!(
            OutputTransaction { heads: vec![one] }.validate(&[capability("one")]),
            Err(OutputTransactionError::UnknownOutput("unknown".into()))
        );
    }

    #[test]
    fn validation_rejects_duplicate_heads_and_an_all_disabled_layout() {
        let enabled = head("one");
        assert_eq!(
            OutputTransaction {
                heads: vec![enabled.clone(), enabled.clone()]
            }
            .validate(&[capability("one")]),
            Err(OutputTransactionError::DuplicateOutput("one".into()))
        );

        let mut disabled = enabled;
        disabled.enabled = false;
        disabled.mode = None;
        assert_eq!(
            OutputTransaction {
                heads: vec![disabled]
            }
            .validate(&[capability("one")]),
            Err(OutputTransactionError::NoEnabledOutputs)
        );
    }

    #[test]
    fn output_power_requests_preserve_order_and_backend_results() {
        let mut service = OutputPowerService::default();
        let off = service.submit("DP-1".into(), OutputPowerMode::Off);
        let on = service.submit("DP-1".into(), OutputPowerMode::On);

        let first = service.take_next_pending().unwrap();
        assert_eq!(first.id, off);
        assert_eq!(first.mode, OutputPowerMode::Off);
        service.complete(first, Ok(OutputPowerMode::Off));

        let second = service.take_next_pending().unwrap();
        assert_eq!(second.id, on);
        service.requeue(second);
        let second = service.take_next_pending().unwrap();
        service.complete(
            second,
            Err(OutputPowerError::Backend("modeset failed".into())),
        );

        let completed = service.take_completed();
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].result, Ok(OutputPowerMode::Off));
        assert!(matches!(
            completed[1].result,
            Err(OutputPowerError::Backend(_))
        ));

        let cancelled = service.submit("DP-2".into(), OutputPowerMode::Off);
        service.cancel(&[cancelled]);
        assert!(!service.has_pending());
    }

    #[test]
    fn validation_checks_mode_scale_and_explicit_adaptive_sync() {
        let mut configured = head("one");
        configured.scale = 0.0;
        assert_eq!(
            OutputTransaction {
                heads: vec![configured.clone()]
            }
            .validate(&[capability("one")]),
            Err(OutputTransactionError::InvalidScale("one".into()))
        );

        configured.scale = 1.0;
        configured.mode.as_mut().unwrap().width = 1280;
        assert!(matches!(
            OutputTransaction {
                heads: vec![configured.clone()]
            }
            .validate(&[capability("one")]),
            Err(OutputTransactionError::UnsupportedMode { .. })
        ));

        configured.mode.as_mut().unwrap().width = 1920;
        configured.adaptive_sync = Some(AdaptiveSyncPolicy::Enabled);
        assert_eq!(
            OutputTransaction {
                heads: vec![configured.clone()]
            }
            .validate(&[capability("one")]),
            Err(OutputTransactionError::AdaptiveSyncUnsupported(
                "one".into()
            ))
        );
    }

    #[test]
    fn automatic_adaptive_sync_is_valid_without_backend_support() {
        let mut configured = head("one");
        configured.adaptive_sync = Some(AdaptiveSyncPolicy::Automatic);
        assert_eq!(
            OutputTransaction {
                heads: vec![configured]
            }
            .validate(&[capability("one")]),
            Ok(())
        );
    }
}
