//! Successfully committed output intent, before transient policy is applied.

use super::{OutputHeadConfiguration, OutputId, OutputTransaction};
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct RequestedOutputState {
    committed: HashMap<OutputId, OutputHeadConfiguration>,
}

impl RequestedOutputState {
    /// Call only after a successful apply, never for tests or failed commits.
    pub fn commit(&mut self, requested: &OutputTransaction) {
        self.committed = requested
            .heads
            .iter()
            .map(|head| (head.id.clone(), head.clone()))
            .collect();
    }

    pub fn forget(&mut self, id: &OutputId) {
        self.committed.remove(id);
    }

    /// Resolve intent against available heads. Observed state seeds new heads;
    /// committed intent restores suppressed settings; queued changes win last.
    /// Neither remembered nor pending state can resurrect an absent head.
    pub fn resolve(
        &self,
        observed: &OutputTransaction,
        pending: Option<&OutputTransaction>,
    ) -> OutputTransaction {
        OutputTransaction {
            heads: observed
                .heads
                .iter()
                .map(|head| {
                    pending
                        .and_then(|transaction| {
                            transaction.heads.iter().find(|other| other.id == head.id)
                        })
                        .or_else(|| self.committed.get(&head.id))
                        .unwrap_or(head)
                        .clone()
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::output::{AdaptiveSyncPolicy, OutputMode, OutputTransform};
    use crate::types::Point;

    fn head(name: &str) -> OutputHeadConfiguration {
        OutputHeadConfiguration {
            id: name.into(),
            enabled: true,
            mode: Some(OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 60_000,
            }),
            position: Point::new(100, 200),
            transform: OutputTransform::Rotate90,
            scale: 1.5,
            adaptive_sync: Some(AdaptiveSyncPolicy::Automatic),
        }
    }

    #[test]
    fn effective_changes_do_not_replace_requested_settings() {
        let mut intent = RequestedOutputState::default();
        let requested = OutputTransaction {
            heads: vec![head("panel"), head("dock")],
        };
        intent.commit(&requested);
        let mut effective = requested.clone();
        effective.heads[0].enabled = false;
        effective.heads[1].position = Point::new(0, 0);
        effective.heads[1].scale = 2.0;
        effective.heads[1].transform = OutputTransform::Normal;
        effective.heads[1].adaptive_sync = Some(AdaptiveSyncPolicy::Disabled);
        assert_eq!(intent.resolve(&effective, None), requested);
    }

    #[test]
    fn pending_changes_win_without_committing_and_cannot_resurrect_absent_heads() {
        let mut intent = RequestedOutputState::default();
        let committed = OutputTransaction {
            heads: vec![head("panel"), head("unplugged")],
        };
        intent.commit(&committed);
        let mut pending = committed.clone();
        pending.heads[0].enabled = false;
        pending.heads[0].scale = 2.0;
        let observed = OutputTransaction {
            heads: vec![head("panel"), head("new-dock")],
        };
        let resolved = intent.resolve(&observed, Some(&pending));
        assert_eq!(
            resolved.heads,
            vec![pending.heads[0].clone(), observed.heads[1].clone()]
        );
        // Removing a failed or tested request from the queue reveals committed
        // intent again; merely resolving it has no persistence side effects.
        let resolved = intent.resolve(&observed, None);
        assert_eq!(
            resolved.heads,
            vec![committed.heads[0].clone(), observed.heads[1].clone()]
        );
    }

    #[test]
    fn only_committing_changes_restoration_and_disconnect_forgets_old_intent() {
        let mut intent = RequestedOutputState::default();
        let observed = OutputTransaction {
            heads: vec![head("panel")],
        };
        let mut disabled = observed.clone();
        disabled.heads[0].enabled = false;
        intent.commit(&disabled);
        assert_eq!(intent.resolve(&observed, None), disabled);
        intent.forget(&"panel".into());
        assert_eq!(intent.resolve(&observed, None), observed);
    }
}
