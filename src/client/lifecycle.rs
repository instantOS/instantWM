//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::model::WmModel;
use crate::types::{MonitorId, TagMask, WindowId};
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PENDING_LAUNCH_TTL: Duration = Duration::from_secs(30);
const MAX_PENDING_LAUNCHES: usize = 128;

/// Remove a managed client and reconcile every shared consumer of that state.
///
/// Backends perform protocol-specific teardown before entering this function.
/// Normal destruction and defensive stale-window recovery deliberately converge
/// here so focus, layout, bars, and EWMH state cannot drift between backends.
pub(crate) fn remove_managed_client(
    ctx: &mut crate::contexts::WmCtx<'_>,
    win: WindowId,
) -> Option<crate::types::Client> {
    let removed = ctx.core_mut().model_mut().remove_client(win)?;
    let monitor_id = removed.monitor_id;

    crate::focus::refresh_focus(ctx, None);
    crate::layouts::arrange(ctx, Some(monitor_id));
    ctx.request_bar_update();

    if let crate::contexts::WmCtx::X11(x11) = ctx {
        crate::backend::x11::properties::update_client_list(
            x11.core.state(),
            &x11.x11,
            x11.x11_runtime,
        );
    }
    Some(removed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchContext {
    pub monitor_id: MonitorId,
    pub tags: TagMask,
    pub is_floating: bool,
}

#[derive(Debug, Clone)]
pub struct PendingLaunch {
    pub recorded_at: Instant,
    pub process_group: Option<LaunchProcessGroupId>,
    pub startup_id: Option<String>,
    pub context: LaunchContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchProcessGroupId(u32);

impl LaunchProcessGroupId {
    pub fn from_spawned_pid(pid: u32) -> Self {
        Self(pid)
    }

    fn matches(self, process_group: u32) -> bool {
        self.0 == process_group
    }
}

pub fn current_launch_context(model: &WmModel) -> LaunchContext {
    LaunchContext {
        monitor_id: model.selected_monitor_id(),
        tags: model.expect_selected_monitor().selected_tags(),
        is_floating: false,
    }
}

pub fn new_startup_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("instantwm-{ts}")
}

pub fn record_pending_launch(
    pending_launches: &mut VecDeque<PendingLaunch>,
    process_group: Option<LaunchProcessGroupId>,
    startup_id: Option<String>,
    context: LaunchContext,
) {
    prune_pending_launches(pending_launches);
    pending_launches.push_back(PendingLaunch {
        recorded_at: Instant::now(),
        process_group,
        startup_id,
        context,
    });
    while pending_launches.len() > MAX_PENDING_LAUNCHES {
        pending_launches.pop_front();
    }
}

pub fn take_pending_launch(
    pending_launches: &mut VecDeque<PendingLaunch>,
    pid: Option<u32>,
    startup_id: Option<&str>,
) -> Option<LaunchContext> {
    prune_pending_launches(pending_launches);

    let startup_match = startup_id.and_then(|id| {
        pending_launches
            .iter()
            .position(|launch| launch.startup_id.as_deref() == Some(id))
    });

    let idx = startup_match.or_else(|| {
        pid.and_then(|pid| find_pending_launch_by_pid(pending_launches, pid, get_process_group_id))
    })?;

    pending_launches.remove(idx).map(|launch| launch.context)
}

/// Read the process-group ID of `pid` from `/proc/{pid}/stat`.
///
/// Every compositor launch starts a new process group whose ID is the spawned
/// child's PID. Descendants retain that group across forks and parent exit, so
/// it is a more durable launch identity than the current parent chain.
fn get_process_group_id(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_process_group_id(&stat)
}

fn parse_process_group_id(stat: &str) -> Option<u32> {
    // /proc/PID/stat: "pid (comm) state ppid ..."
    // comm may contain spaces and parentheses, so parse from the last ')'.
    let after_comm = stat.rfind(')')?;
    let rest = stat[after_comm + 1..].trim_start();
    let mut fields = rest.split_whitespace();
    // field 3: state, field 4: ppid, field 5: pgrp
    fields.next()?; // state
    fields.next()?; // ppid
    fields.next()?.parse().ok()
}

fn find_pending_launch_by_pid(
    pending_launches: &VecDeque<PendingLaunch>,
    pid: u32,
    process_group: impl FnOnce(u32) -> Option<u32>,
) -> Option<usize> {
    pending_launches
        .iter()
        .position(|launch| {
            launch
                .process_group
                .is_some_and(|process_group| process_group.matches(pid))
        })
        .or_else(|| {
            let process_group = process_group(pid)?;
            pending_launches.iter().position(|launch| {
                launch
                    .process_group
                    .is_some_and(|launch_group| launch_group.matches(process_group))
            })
        })
}

fn prune_pending_launches(pending_launches: &mut VecDeque<PendingLaunch>) {
    let now = Instant::now();
    pending_launches.retain(|launch| now.duration_since(launch.recorded_at) <= PENDING_LAUNCH_TTL);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(pid: u32) -> PendingLaunch {
        PendingLaunch {
            recorded_at: Instant::now(),
            process_group: Some(LaunchProcessGroupId::from_spawned_pid(pid)),
            startup_id: None,
            context: LaunchContext {
                monitor_id: MonitorId::default(),
                tags: TagMask::default(),
                is_floating: false,
            },
        }
    }

    #[test]
    fn proc_stat_parser_handles_spaces_and_parentheses_in_command_name() {
        assert_eq!(
            parse_process_group_id("4321 (odd) command) S 123 987 0 0 0"),
            Some(987)
        );
    }

    #[test]
    fn launch_pid_matching_accepts_a_process_group_descendant() {
        let launches = VecDeque::from([pending(100), pending(200)]);
        assert_eq!(
            find_pending_launch_by_pid(&launches, 201, |_| Some(200)),
            Some(1)
        );
    }

    #[test]
    fn exact_launch_pid_does_not_require_procfs() {
        let launches = VecDeque::from([pending(100)]);
        assert_eq!(
            find_pending_launch_by_pid(&launches, 100, |_| panic!("unexpected proc lookup")),
            Some(0)
        );
    }
}
