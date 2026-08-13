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
use crate::types::{MonitorRule, Rule, RuleFloat, TagMask, MAX_TAGS};
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
            // Validate the 1-indexed tag early so users don't get a silent
            // mismatched-bitmask on the rare matched window. MAX_TAGS (not
            // u32::BITS) is the real limit: bits beyond it are masked out by
            // `TagMask::from_bits`.
            if let Some(n) = tag
                && (n == 0 || n as usize > MAX_TAGS)
            {
                return Response::err(format!("tag must be in 1..={MAX_TAGS}"));
            }

            // A negative backend monitor index is always user error; `None`
            // already means "leave placement alone".
            if let Some(num) = on_monitor
                && num < 0
            {
                return Response::err("on-monitor must be >= 0");
            }

            let rule = Rule {
                class: class.map(Cow::Owned),
                instance: instance.map(Cow::Owned),
                title: title.map(Cow::Owned),
                tags: tag.map(n_to_tag_mask).unwrap_or(TagMask::EMPTY),
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
                    tag: tag_mask_to_n(p.rule.tags),
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
            let mut cancelled = false;
            wm.ctx().with_behavior_mut(|behavior| {
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

/// Build a single-tag mask from a 1-indexed tag number.
fn n_to_tag_mask(n: u32) -> TagMask {
    TagMask::from_bits(1u32 << (n - 1))
}

/// Inverse of [`n_to_tag_mask`]: pick the lowest set bit as a 1-indexed tag.
///
/// Returns `None` for an empty mask (because the field is optional).
fn tag_mask_to_n(mask: TagMask) -> Option<u32> {
    let bits = mask.bits();
    if bits == 0 {
        return None;
    }
    Some(bits.trailing_zeros() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mask_round_trip() {
        assert_eq!(tag_mask_to_n(n_to_tag_mask(1)), Some(1));
        assert_eq!(tag_mask_to_n(n_to_tag_mask(3)), Some(3));
        assert_eq!(tag_mask_to_n(TagMask::EMPTY), None);
    }
}
