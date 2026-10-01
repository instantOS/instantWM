//! Laptop display policy, independent of the input and display backends.
//!
//! Requested configurations survive temporary lid suppression. Only successful
//! applies update them; tests and failed commits must not change restoration.

use std::collections::{HashMap, HashSet};

use super::{OutputHeadConfiguration, OutputId, OutputTransaction};

#[derive(Debug, Default, Clone)]
pub struct LidOutputPolicy {
    closed: bool,
    internal: HashSet<OutputId>,
    requested: HashMap<OutputId, OutputHeadConfiguration>,
}

impl LidOutputPolicy {
    pub fn set_closed(&mut self, closed: bool) -> bool {
        let changed = self.closed != closed;
        self.closed = closed;
        changed
    }

    /// Physical connector classification comes from the active backend.
    pub fn set_internal(&mut self, id: OutputId, internal: bool) {
        if internal {
            self.internal.insert(id);
        } else {
            self.internal.remove(&id);
        }
    }

    pub fn forget(&mut self, id: &OutputId) {
        self.internal.remove(id);
        self.requested.remove(id);
    }

    /// Overlay remembered intent onto currently available physical heads.
    /// Newly connected heads keep their backend defaults; absent heads stay absent.
    pub fn requested(&self, current: &OutputTransaction) -> OutputTransaction {
        OutputTransaction {
            heads: current
                .heads
                .iter()
                .map(|head| self.requested.get(&head.id).unwrap_or(head).clone())
                .collect(),
        }
    }

    pub fn remember_applied(&mut self, requested: &OutputTransaction) {
        self.requested = requested
            .heads
            .iter()
            .map(|head| (head.id.clone(), head.clone()))
            .collect();
    }

    /// Suppress built-in panels only with a usable external head. Keeping the
    /// last display configured lets logind retain ownership of suspension and
    /// allows recovery when a dock is unplugged while the lid remains closed.
    pub fn project(&self, requested: &OutputTransaction) -> OutputTransaction {
        let mut effective = requested.clone();
        if self.closed
            && requested.heads.iter().any(|head| {
                head.enabled && head.mode.is_some() && !self.internal.contains(&head.id)
            })
        {
            for head in &mut effective.heads {
                if self.internal.contains(&head.id) {
                    head.enabled = false;
                }
            }
        }
        effective
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::output::{OutputMode, OutputTransform};
    use crate::types::Point;

    fn head(name: &str, enabled: bool) -> OutputHeadConfiguration {
        OutputHeadConfiguration {
            id: name.into(),
            enabled,
            mode: Some(OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihertz: 60_000,
            }),
            position: Point::new(0, 0),
            transform: OutputTransform::Normal,
            scale: 1.0,
            adaptive_sync: None,
        }
    }

    fn policy() -> LidOutputPolicy {
        let mut policy = LidOutputPolicy::default();
        policy.set_internal("panel".into(), true);
        policy.set_closed(true);
        policy
    }

    #[test]
    fn closed_lid_suppresses_only_internal_panels_and_preserves_settings() {
        let mut policy = policy();
        let requested = OutputTransaction {
            heads: vec![head("panel", true), head("dock", true)],
        };
        policy.remember_applied(&requested);
        let effective = policy.project(&requested);
        assert!(!effective.heads[0].enabled);
        assert!(effective.heads[1].enabled);
        assert_eq!(policy.requested(&effective), requested);
        policy.set_closed(false);
        assert_eq!(policy.project(&policy.requested(&effective)), requested);
    }

    #[test]
    fn undocking_and_redocking_while_closed_reprojects_remembered_intent() {
        let mut policy = policy();
        let requested = OutputTransaction {
            heads: vec![head("panel", true), head("dock", true)],
        };
        policy.remember_applied(&requested);
        let effective = policy.project(&requested);
        policy.forget(&"dock".into());
        let only_panel = OutputTransaction {
            heads: vec![effective.heads[0].clone()],
        };
        let restored = policy.project(&policy.requested(&only_panel));
        assert!(restored.heads[0].enabled);
        policy.remember_applied(&policy.requested(&only_panel));
        let reconnected = OutputTransaction {
            heads: vec![restored.heads[0].clone(), head("dock", true)],
        };
        assert!(!policy.project(&policy.requested(&reconnected)).heads[0].enabled);
    }

    #[test]
    fn manually_disabled_panel_stays_disabled_after_opening() {
        let mut policy = policy();
        let requested = OutputTransaction {
            heads: vec![head("panel", false), head("dock", true)],
        };
        policy.remember_applied(&requested);
        policy.set_closed(false);
        assert!(!policy.project(&policy.requested(&requested)).heads[0].enabled);
    }

    #[test]
    fn disabled_or_modeless_external_head_does_not_hide_the_last_display() {
        let policy = policy();
        for mut dock in [head("dock", false), head("dock", true)] {
            if dock.enabled {
                dock.mode = None;
            }
            let requested = OutputTransaction {
                heads: vec![head("panel", true), dock],
            };
            assert!(policy.project(&requested).heads[0].enabled);
        }
    }

    #[test]
    fn uncommitted_requests_do_not_change_restoration_and_removed_heads_are_forgotten() {
        let mut policy = policy();
        let requested = OutputTransaction {
            heads: vec![head("panel", true), head("dock", true)],
        };
        policy.remember_applied(&requested);
        let mut uncommitted = requested.clone();
        uncommitted.heads[0].scale = 2.0;
        let _ = policy.project(&uncommitted);
        assert_eq!(policy.requested(&uncommitted), requested);
        policy.forget(&"panel".into());
        assert_eq!(policy.requested(&uncommitted).heads[0].scale, 2.0);
    }
}
