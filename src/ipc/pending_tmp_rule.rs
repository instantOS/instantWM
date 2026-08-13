//! Pending tmp rules — runtime-added one-shot window rules.
//!
//! A pending tmp rule is a normal [`Rule`] that has been queued at runtime
//! (via `instantwmctl pending-tmp-rule add ...`) and is consumed on the
//! first matching window's initial rule application. Each entry has an
//! absolute deadline and is dropped lazily when it expires.
//!
//! See [`crate::client::rules`] for the matching/consumption path.

use crate::ipc_types::{
    PendingTmpRuleCmd, PendingTmpRuleInfo, Response,
};
use crate::types::{MonitorRule, Rule, RuleFloat, TagMask};
use crate::wm::Wm;
use std::borrow::Cow;
use std::time::{Duration, Instant};

/// Maximum accepted TTL. Caps nonsense inputs from CLI scripts.
const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1_000;

/// Holds the next id assigned to an added entry.
static NEXT_PENDING_TMP_RULE_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Dispatch a `PendingTmpRuleCmd` IPC request.
pub fn handle_pending_tmp_rule(wm: &mut Wm, cmd: PendingTmpRuleCmd) -> Response {
    match cmd {
        PendingTmpRuleCmd::Add {
            class,
            instance,
            title,
            is_floating,
            tag,
            on_monitor,
            timeout_ms,
        } => {
            if timeout_ms == 0 {
                return Response::err("timeout-ms must be greater than zero");
            }
            if timeout_ms > MAX_TIMEOUT_MS {
                return Response::err(format!(
                    "timeout-ms must be <= {MAX_TIMEOUT_MS} (24h)"
                ));
            }

            // A rule with no matcher and no effect would match every window
            // and change nothing — while still suppressing the config-rule
            // pass for that window (see client/rules.rs). Require at least one
            // of either so a typo'd or empty `add` is rejected up front.
            let has_matcher = class.is_some() || instance.is_some() || title.is_some();
            let has_effect = is_floating.is_some() || tag.is_some() || on_monitor.is_some();
            if !has_matcher && !has_effect {
                return Response::err(
                    "pending-tmp-rule needs at least one matcher \
                     (class/instance/title) or effect (floating/tag/on-monitor)",
                );
            }

            // Validate the 1-indexed tag against the *runtime* tag count, not
            // MAX_TAGS: a tag beyond num_tags is masked out to nothing at apply
            // time and the window silently lands on the monitor's current tags
            // — while the one-shot rule is still consumed.
            let num_tags = wm.ctx().core().model().tags.num_tags;
            if let Some(n) = tag
                && (n == 0 || n as usize > num_tags)
            {
                return Response::err(format!("tag must be in 1..={num_tags}"));
            }

            // Reject monitor indices that no present monitor owns, so the
            // one-shot isn't silently consumed by a no-op move at apply time
            // (`apply_monitor_rule` can only target existing monitors).
            if let Some(num) = on_monitor
                && !wm.ctx().core().state().monitors_iter().any(|(_, m)| m.num == num)
            {
                return Response::err(format!(
                    "on-monitor {num} does not match any connected monitor"
                ));
            }

            let rule = Rule {
                class: class.map(Cow::Owned),
                instance: instance.map(Cow::Owned),
                title: title.map(Cow::Owned),
                tags: tag
                    .and_then(|n| TagMask::single(n as usize))
                    .unwrap_or(TagMask::EMPTY),
                is_floating: is_floating.map(|f| {
                    if f {
                        RuleFloat::Float
                    } else {
                        RuleFloat::Tiled
                    }
                }),
                monitor: match on_monitor {
                    Some(num) => MonitorRule::Index(num as usize),
                    None => MonitorRule::Any,
                },
            };

            let id = NEXT_PENDING_TMP_RULE_ID
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let entry = crate::types::PendingTmpRule {
                id,
                rule,
                deadline: Instant::now() + Duration::from_millis(timeout_ms),
            };

            wm.ctx()
                .with_behavior_mut(|behavior| behavior.pending_tmp_rules.push(entry));

            Response::PendingTmpRuleAdded { id, timeout_ms }
        }
        PendingTmpRuleCmd::List => {
            let now = Instant::now();
            // Drop expired entries so `list` never reports already-inert rules
            // (clamped to 0ms remaining). They would otherwise linger until the
            // next window spawn sweeps them on the apply path.
            wm.ctx().with_behavior_mut(|behavior| {
                behavior.pending_tmp_rules.retain(|p| p.deadline > now);
            });
            let entries: Vec<PendingTmpRuleInfo> = wm
                .ctx()
                .core()
                .behavior()
                .pending_tmp_rules
                .iter()
                .map(|p| PendingTmpRuleInfo {
                    id: p.id,
                    class: p.rule.class.as_ref().map(|c| c.to_string()),
                    instance: p.rule.instance.as_ref().map(|i| i.to_string()),
                    title: p.rule.title.as_ref().map(|t| t.to_string()),
                    is_floating: match p.rule.is_floating {
                        Some(RuleFloat::Tiled) => Some(false),
                        Some(_) => Some(true),
                        None => None,
                    },
                    tag: p.rule.tags.first_tag().map(|i| i as u32),
                    on_monitor: match p.rule.monitor {
                        MonitorRule::Any => None,
                        MonitorRule::Index(idx) => Some(idx as i32),
                    },
                    ms_remaining: p
                        .deadline
                        .checked_duration_since(now)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                })
                .collect();
            Response::PendingTmpRuleList(entries)
        }
        PendingTmpRuleCmd::Cancel(id) => {
            let now = Instant::now();
            let mut cancelled = false;
            wm.ctx().with_behavior_mut(|behavior| {
                // Sweep expired entries first so cancelling an id that already
                // expired reports "not found" rather than a spurious success.
                behavior.pending_tmp_rules.retain(|p| p.deadline > now);
                let before = behavior.pending_tmp_rules.len();
                behavior.pending_tmp_rules.retain(|p| p.id != id);
                cancelled = behavior.pending_tmp_rules.len() != before;
            });
            if cancelled {
                Response::Message(format!("pending-tmp-rule {id} cancelled"))
            } else {
                Response::err(format!("pending-tmp-rule {id} not found"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Monitor, PendingTmpRule, Rule, TagMask};
    use crate::wm::Wm;

    /// Build a Wm with `num_tags` tags and a single monitor reporting `num=0`.
    fn wm_with(num_tags: usize) -> Wm {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = num_tags;
        wm.core.model.monitors.push(Monitor {
            num: 0,
            ..Monitor::default()
        });
        wm
    }

    /// Minimal Add command: only the given knobs are set, rest left to defaults.
    fn add(class: Option<&str>, tag: Option<u32>, on_monitor: Option<i32>) -> PendingTmpRuleCmd {
        PendingTmpRuleCmd::Add {
            class: class.map(str::to_owned),
            instance: None,
            title: None,
            is_floating: None,
            tag,
            on_monitor,
            timeout_ms: 5_000,
        }
    }

    /// Push an expired entry directly, bypassing Add validation.
    fn inject_expired(wm: &mut Wm, id: u64) {
        wm.ctx().with_behavior_mut(|b| {
            b.pending_tmp_rules.push(PendingTmpRule {
                id,
                rule: Rule {
                    class: None,
                    instance: None,
                    title: None,
                    tags: TagMask::EMPTY,
                    is_floating: None,
                    monitor: MonitorRule::Any,
                },
                deadline: Instant::now() - Duration::from_secs(1),
            });
        });
    }

    #[test]
    fn bare_add_without_matcher_or_effect_is_rejected() {
        let mut wm = wm_with(9);
        let resp = handle_pending_tmp_rule(&mut wm, add(None, None, None));
        assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    }

    #[test]
    fn tag_beyond_runtime_num_tags_is_rejected() {
        let mut wm = wm_with(3);
        // 3 is the last valid tag with num_tags=3; 4 exceeds it.
        assert!(matches!(
            handle_pending_tmp_rule(&mut wm, add(Some("x"), Some(3), None)),
            Response::PendingTmpRuleAdded { .. }
        ));
        let resp = handle_pending_tmp_rule(&mut wm, add(Some("x"), Some(4), None));
        assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    }

    #[test]
    fn on_monitor_with_no_matching_monitor_is_rejected() {
        let mut wm = wm_with(9); // only monitor num 0 exists
        assert!(matches!(
            handle_pending_tmp_rule(&mut wm, add(Some("x"), None, Some(0))),
            Response::PendingTmpRuleAdded { .. }
        ));
        let resp = handle_pending_tmp_rule(&mut wm, add(Some("x"), None, Some(1)));
        assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    }

    #[test]
    fn list_omits_and_drops_expired_entries() {
        let mut wm = wm_with(9);
        inject_expired(&mut wm, 7);
        let resp = handle_pending_tmp_rule(&mut wm, PendingTmpRuleCmd::List);
        let Response::PendingTmpRuleList(entries) = resp else {
            panic!("unexpected response {resp:?}");
        };
        assert!(entries.is_empty());
        // The entry is actually removed, not just hidden from the listing.
        assert!(wm
            .ctx()
            .core()
            .behavior()
            .pending_tmp_rules
            .is_empty());
    }

    #[test]
    fn cancel_of_expired_entry_reports_not_found() {
        let mut wm = wm_with(9);
        inject_expired(&mut wm, 7);
        let resp = handle_pending_tmp_rule(&mut wm, PendingTmpRuleCmd::Cancel(7));
        assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    }
}
