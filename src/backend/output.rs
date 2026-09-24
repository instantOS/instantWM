//! Backend-neutral output configuration transactions.
//!
//! Producers submit complete desired output configurations. The active backend
//! validates or commits them asynchronously and returns an authoritative
//! snapshot. Protocol objects and backend-native handles deliberately do not
//! cross this boundary.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::types::Point;
use crate::types::Rect;

/// Ownership of an output's logical position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPositionSource {
    /// Chosen by instantWM's fallback layout and eligible for compaction.
    Automatic,
    /// Anchored by persistent monitor configuration.
    Configured,
    /// Anchored by a runtime output-management client or external tool.
    ClientManaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputPlacement {
    pub id: String,
    pub rect: Rect,
    pub source: OutputPositionSource,
}

/// Plan a gap-free left-to-right layout for automatic outputs while treating
/// configured and client-managed outputs as immovable anchors.
pub fn plan_automatic_output_positions(outputs: &mut [OutputPlacement]) -> Vec<(String, Point)> {
    outputs.sort_by(|a, b| (a.rect.x, &a.id).cmp(&(b.rect.x, &b.id)));
    let mut cursor = 0;
    let mut moves = Vec::new();
    for output in outputs {
        if output.source != OutputPositionSource::Automatic {
            cursor = cursor.max(output.rect.x.saturating_add(output.rect.w));
            continue;
        }
        if output.rect.x != cursor {
            moves.push((output.id.clone(), Point::new(cursor, output.rect.y)));
        }
        cursor = cursor.saturating_add(output.rect.w);
    }
    moves
}

/// Top-left position immediately right of every rectangle in `rects`.
///
/// This is where an output entering the layout without a configured position
/// starts before [`plan_automatic_output_positions`] closes any hole.
pub fn position_after(rects: impl IntoIterator<Item = Rect>) -> Point {
    Point::new(rects.into_iter().map(Rect::right).max().unwrap_or(0), 0)
}

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

/// A mode requested by monitor configuration, independent of the output API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorModeRequest {
    width: i32,
    height: i32,
    refresh_hz: Option<f32>,
}

impl MonitorModeRequest {
    /// Any refresh rate at exactly `width`x`height`.
    pub fn size(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            refresh_hz: None,
        }
    }

    pub fn parse(resolution: &str, refresh_hz: Option<f32>) -> Option<Self> {
        let (width, height) = resolution.split_once('x')?;
        let width = width.parse().ok()?;
        let height = height.parse().ok()?;
        (width > 0 && height > 0).then_some(Self {
            width,
            height,
            refresh_hz,
        })
    }

    /// Unknown refresh is acceptable only when no rate was requested.
    pub fn matches(self, width: i32, height: i32, refresh_millihertz: Option<u32>) -> bool {
        if self.width != width || self.height != height {
            return false;
        }
        match self.refresh_hz {
            None => true,
            Some(requested) => refresh_millihertz.is_some_and(|actual| {
                (f64::from(actual) / 1000.0 - f64::from(requested)).abs() < 0.1
            }),
        }
    }
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

impl OutputTransform {
    /// Parse a transform string as used in config and IPC (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "normal" => Some(Self::Normal),
            "90" => Some(Self::Rotate90),
            "180" => Some(Self::Rotate180),
            "270" => Some(Self::Rotate270),
            "flipped" => Some(Self::Flipped),
            "flipped-90" | "flipped90" => Some(Self::Flipped90),
            "flipped-180" | "flipped180" => Some(Self::Flipped180),
            "flipped-270" | "flipped270" => Some(Self::Flipped270),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::Flipped => "flipped",
            Self::Flipped90 => "flipped-90",
            Self::Flipped180 => "flipped-180",
            Self::Flipped270 => "flipped-270",
        }
    }
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

#[derive(Debug, Clone)]
pub struct OutputPowerRequest {
    pub output: OutputId,
    pub mode: OutputPowerMode,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputPowerError {
    #[error("output {0} is not available for power management")]
    Unavailable(String),
    #[error("backend rejected the output power change: {0}")]
    Backend(String),
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

    /// Pin every realizable mirror head to its source's logical position and
    /// scale.
    ///
    /// A mirror presents its source's region, so it owns no position of its
    /// own; reporting the source's position keeps output-management clients
    /// showing the pair as overlapping. The scale must match because the
    /// mirror is rendered from elements built at the source's scale. Mode and
    /// transform stay the mirror head's own: the renderer fits the source's
    /// content to whatever the head scans out. Pairs with a disabled or
    /// missing head are left alone, since such a mirror is an ordinary output.
    ///
    /// Runs before `validate` on every transaction, so neither policy nor
    /// output-management clients can separate a realized pair.
    pub fn apply_mirrors(&mut self, mirrors: &crate::output_mirror::MirrorMap) {
        let enabled: HashMap<String, (Point, f64)> = self
            .heads
            .iter()
            .filter(|head| head.enabled)
            .map(|head| (head.id.0.clone(), (head.position, head.scale)))
            .collect();
        let pinned: Vec<(String, Point, f64)> = mirrors
            .active_pairs(|name| enabled.contains_key(name))
            .map(|(mirror, target)| {
                let (position, scale) = enabled[&target.source];
                (mirror.to_string(), position, scale)
            })
            .collect();
        for (mirror, position, scale) in pinned {
            if let Some(head) = self.heads.iter_mut().find(|head| head.id.0 == mirror) {
                head.position = position;
                head.scale = scale;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(u64);

/// FIFO of backend work submitted by protocol handlers and the completions
/// the backend reports back, both keyed by a [`RequestId`].
#[derive(Debug)]
pub struct RequestQueue<P, C> {
    next_id: u64,
    pending: VecDeque<(RequestId, P)>,
    completed: Vec<(RequestId, C)>,
}

impl<P, C> Default for RequestQueue<P, C> {
    fn default() -> Self {
        Self {
            next_id: 0,
            pending: VecDeque::new(),
            completed: Vec::new(),
        }
    }
}

impl<P, C> RequestQueue<P, C> {
    pub fn submit(&mut self, request: P) -> RequestId {
        self.next_id = self.next_id.wrapping_add(1);
        let id = RequestId(self.next_id);
        self.pending.push_back((id, request));
        id
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn take_next_pending(&mut self) -> Option<(RequestId, P)> {
        self.pending.pop_front()
    }

    /// Put a request the backend cannot serve yet back at the head of the
    /// queue, ahead of later submissions.
    pub fn requeue(&mut self, id: RequestId, request: P) {
        self.pending.push_front((id, request));
    }

    pub fn cancel(&mut self, ids: &[RequestId]) {
        self.pending.retain(|(id, _)| !ids.contains(id));
    }

    pub fn complete(&mut self, id: RequestId, completion: C) {
        self.completed.push((id, completion));
    }

    pub fn take_completed(&mut self) -> Vec<(RequestId, C)> {
        std::mem::take(&mut self.completed)
    }
}

/// Backend-neutral queue joining protocol requests to backend-native DPMS.
pub type OutputPowerQueue =
    RequestQueue<OutputPowerRequest, (OutputId, Result<OutputPowerMode, OutputPowerError>)>;

#[derive(Debug, Clone)]
pub struct OutputTransactionRequest {
    pub kind: OutputTransactionKind,
    pub transaction: OutputTransaction,
    /// Compositor policy rather than an output-management client request.
    pub policy: bool,
}

pub type OutputTransactionQueue = RequestQueue<
    OutputTransactionRequest,
    (
        OutputTransactionKind,
        Result<OutputSnapshot, OutputTransactionError>,
    ),
>;

impl OutputTransactionQueue {
    /// Queue an output-management client transaction. These are never
    /// coalesced and retain one completion per request.
    pub fn submit_client(
        &mut self,
        kind: OutputTransactionKind,
        transaction: OutputTransaction,
    ) -> RequestId {
        self.submit(OutputTransactionRequest {
            kind,
            transaction,
            policy: false,
        })
    }

    /// Queue compositor policy/configuration state. Consecutive policy writes
    /// are collapsed because only their final desired state is observable.
    pub fn submit_coalescing_apply(&mut self, transaction: OutputTransaction) -> RequestId {
        if let Some((id, pending)) = self.pending.back_mut()
            && pending.kind == OutputTransactionKind::Apply
            && pending.policy
        {
            pending.transaction = transaction;
            return *id;
        }
        self.submit(OutputTransactionRequest {
            kind: OutputTransactionKind::Apply,
            transaction,
            policy: true,
        })
    }

    pub fn latest_pending_apply(&self) -> Option<&OutputTransaction> {
        self.pending
            .iter()
            .rev()
            .find(|(_, pending)| pending.kind == OutputTransactionKind::Apply)
            .map(|(_, pending)| &pending.transaction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_mode_requests_parse_and_match_consistently() {
        let request = MonitorModeRequest::parse("1920x1080", Some(59.94)).unwrap();
        assert!(request.matches(1920, 1080, Some(60_000)));
        assert!(!request.matches(1920, 1080, Some(120_000)));
        assert!(!request.matches(1920, 1080, None));
        assert!(!request.matches(2560, 1080, Some(60_000)));
        assert!(
            MonitorModeRequest::parse("1920x1080", None)
                .unwrap()
                .matches(1920, 1080, None)
        );
        assert!(MonitorModeRequest::parse("0x1080", None).is_none());
        assert!(MonitorModeRequest::parse("1920x-1", None).is_none());
        assert!(MonitorModeRequest::parse("1920x1080x60", None).is_none());
    }

    #[test]
    fn automatic_layout_compacts_only_automatic_outputs() {
        let mut outputs = vec![
            OutputPlacement {
                id: "automatic-left".into(),
                rect: Rect::new(1920, 0, 1920, 1080),
                source: OutputPositionSource::Automatic,
            },
            OutputPlacement {
                id: "configured-anchor".into(),
                rect: Rect::new(5000, 0, 1000, 1000),
                source: OutputPositionSource::Configured,
            },
            OutputPlacement {
                id: "automatic-right".into(),
                rect: Rect::new(7000, 0, 1280, 720),
                source: OutputPositionSource::Automatic,
            },
        ];
        assert_eq!(
            plan_automatic_output_positions(&mut outputs),
            vec![
                ("automatic-left".into(), Point::new(0, 0)),
                ("automatic-right".into(), Point::new(6000, 0)),
            ]
        );
    }

    #[test]
    fn output_transform_strings_round_trip() {
        for transform in [
            OutputTransform::Normal,
            OutputTransform::Rotate90,
            OutputTransform::Rotate180,
            OutputTransform::Rotate270,
            OutputTransform::Flipped,
            OutputTransform::Flipped90,
            OutputTransform::Flipped180,
            OutputTransform::Flipped270,
        ] {
            assert_eq!(OutputTransform::parse(transform.as_str()), Some(transform));
        }

        assert_eq!(
            OutputTransform::parse("FLIPPED90"),
            Some(OutputTransform::Flipped90)
        );
        assert_eq!(OutputTransform::parse("sideways"), None);
    }

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
    fn policy_updates_coalesce_but_client_requests_do_not() {
        let mut service = OutputTransactionQueue::default();
        let policy_id = service.submit_coalescing_apply(transaction());
        assert_eq!(service.submit_coalescing_apply(transaction()), policy_id);
        service.submit_client(OutputTransactionKind::Apply, transaction());

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

    fn mirror_map(pairs: &[(&str, &str)]) -> crate::output_mirror::MirrorMap {
        crate::output_mirror::MirrorMap::from_pairs(
            pairs
                .iter()
                .map(|(mirror, source)| (mirror.to_string(), source.to_string())),
        )
    }

    #[test]
    fn position_after_starts_right_of_every_rectangle() {
        let rects = [Rect::new(-1280, 0, 1280, 1024), Rect::new(0, 0, 1920, 1080)];
        assert_eq!(position_after(rects), Point::new(1920, 0));
        assert_eq!(position_after([]), Point::new(0, 0));
    }

    #[test]
    fn apply_mirrors_pins_position_and_scale_but_keeps_mode_and_transform() {
        let mut mirror = head("DP-1");
        mirror.position = Point::new(500, 500);
        mirror.scale = 2.0;
        mirror.transform = OutputTransform::Rotate90;
        let mut source = head("eDP-1");
        source.position = Point::new(1920, 0);
        source.scale = 1.5;
        source.transform = OutputTransform::Rotate180;
        source.mode = Some(OutputMode {
            width: 2560,
            height: 1440,
            refresh_millihertz: 144_000,
        });
        let mut transaction = OutputTransaction {
            heads: vec![mirror.clone(), source],
        };

        transaction.apply_mirrors(&mirror_map(&[("DP-1", "eDP-1")]));

        let pinned = &transaction.heads[0];
        assert_eq!(pinned.position, Point::new(1920, 0));
        assert_eq!(pinned.scale, 1.5);
        assert_eq!(pinned.transform, mirror.transform);
        assert_eq!(pinned.mode, mirror.mode);
    }

    #[test]
    fn apply_mirrors_leaves_pairs_with_an_inactive_head_alone() {
        let map = mirror_map(&[("DP-1", "eDP-1")]);
        let mut mirror = head("DP-1");
        mirror.position = Point::new(7, 7);
        mirror.scale = 2.0;

        // A disabled source leaves the mirror an ordinary, enabled output.
        let mut source = head("eDP-1");
        source.enabled = false;
        let mut transaction = OutputTransaction {
            heads: vec![mirror.clone(), source],
        };
        transaction.apply_mirrors(&map);
        assert_eq!(transaction.heads[0], mirror);

        // A missing source head likewise.
        let mut transaction = OutputTransaction {
            heads: vec![mirror.clone()],
        };
        transaction.apply_mirrors(&map);
        assert_eq!(transaction.heads[0], mirror);

        // A missing mirror head leaves the source untouched.
        let source = head("eDP-1");
        let mut transaction = OutputTransaction {
            heads: vec![source.clone()],
        };
        transaction.apply_mirrors(&map);
        assert_eq!(transaction.heads[0], source);
    }

    #[test]
    fn a_pinned_transaction_passes_validation() {
        let mut mirror = head("DP-1");
        mirror.position = Point::new(9999, 9999);
        mirror.scale = 3.0;
        let mut transaction = OutputTransaction {
            heads: vec![mirror, head("eDP-1")],
        };
        let capabilities = vec![capability("DP-1"), capability("eDP-1")];

        transaction.apply_mirrors(&mirror_map(&[("DP-1", "eDP-1")]));
        assert_eq!(transaction.validate(&capabilities), Ok(()));
    }
}
