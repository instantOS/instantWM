//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::model::WmModel;
use crate::types::{Client, ClientPlacement, MonitorId, TagMask, WindowId};
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
    let previous_focus = ctx.core().model().selected_win();
    let removed = ctx
        .core_mut()
        .mutate_selection(|model| model.remove_client(win))?;
    let monitor_id = removed.monitor_id;

    let overview_became_empty = ctx
        .core()
        .model()
        .monitor(monitor_id)
        .is_some_and(|monitor| {
            monitor_id == ctx.core().model().selected_monitor_id()
                && monitor.overview_state.is_some()
                && !crate::overview::has_cards(monitor, &ctx.core().model().clients)
        });
    if overview_became_empty {
        crate::overview::exit_overview(ctx, crate::overview::ExitMode::RestorePrevious);
    }

    crate::focus::refresh_focus_after_selection(ctx, previous_focus, None);
    crate::layouts::arrange(ctx, Some(monitor_id));
    ctx.request_bar_update();
    ctx.sync_client_list();
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

/// Assign a new client to the same backend-neutral destination policy used by
/// every window-system frontend.
///
/// Transients follow a managed parent. Otherwise a still-valid launch context
/// wins, followed by the currently selected monitor. Launch contexts may
/// outlive an output, so stale monitor IDs deliberately fall through.
pub(crate) fn assign_initial_monitor_and_tags(
    model: &WmModel,
    client: &mut Client,
    transient_for: Option<WindowId>,
    launch_context: Option<LaunchContext>,
) -> bool {
    if let Some(view) = transient_for.and_then(|window| model.client_view(window)) {
        client.monitor_id = view.monitor.id();
        client.set_tag_mask(view.client.tags);
        return true;
    }

    if let Some(launch_context) = launch_context
        && model.monitor(launch_context.monitor_id).is_some()
    {
        client.monitor_id = launch_context.monitor_id;
        client.set_tag_mask(launch_context.tags);
        client.set_placement(if launch_context.is_floating {
            ClientPlacement::Floating
        } else {
            ClientPlacement::Tiling
        });
        return true;
    }

    let Some(selected_monitor) = model.selected_monitor() else {
        return false;
    };
    client.monitor_id = selected_monitor.id();
    client.set_tag_mask(selected_monitor.selected_tags());
    true
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
    use crate::types::{Client, ClientPlacement, Monitor, MonitorId, TagMask, WindowId};

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

    #[test]
    fn transient_destination_takes_priority_over_launch_context() {
        let mut model = WmModel::new();
        let selected_id = model.monitors.push(Monitor {
            tag_set: [TagMask::single(1).unwrap(); 2],
            ..Monitor::default()
        });
        let parent_monitor_id = model.monitors.push(Monitor {
            tag_set: [TagMask::single(2).unwrap(); 2],
            ..Monitor::default()
        });
        model.set_selected_monitor(selected_id);

        let parent = WindowId(10);
        let parent_tags = TagMask::single(3).unwrap();
        model.insert_client(Client {
            win: parent,
            monitor_id: parent_monitor_id,
            tags: parent_tags,
            ..Client::default()
        });

        let mut client = Client::new(WindowId(11));
        assert!(assign_initial_monitor_and_tags(
            &model,
            &mut client,
            Some(parent),
            Some(LaunchContext {
                monitor_id: selected_id,
                tags: TagMask::single(4).unwrap(),
                is_floating: true,
            }),
        ));
        assert_eq!(client.monitor_id, parent_monitor_id);
        assert_eq!(client.tags, parent_tags);
        assert_eq!(client.placement(), ClientPlacement::Tiling);
    }

    #[test]
    fn stale_launch_destination_falls_back_to_selected_monitor() {
        let mut model = WmModel::new();
        let selected_tags = TagMask::single(2).unwrap();
        let selected_id = model.monitors.push(Monitor {
            tag_set: [selected_tags; 2],
            ..Monitor::default()
        });
        model.set_selected_monitor(selected_id);

        let mut client = Client::new(WindowId(20));
        assert!(assign_initial_monitor_and_tags(
            &model,
            &mut client,
            None,
            Some(LaunchContext {
                monitor_id: MonitorId::from_raw(999),
                tags: TagMask::single(4).unwrap(),
                is_floating: true,
            }),
        ));
        assert_eq!(client.monitor_id, selected_id);
        assert_eq!(client.tags, selected_tags);
        assert_eq!(client.placement(), ClientPlacement::Tiling);
    }

    #[test]
    fn overview_projection_is_not_used_as_a_launch_destination() {
        let mut model = WmModel::new();
        model.tags.num_tags = 4;
        let real_tags = TagMask::single(2).unwrap();
        let monitor_id = model.monitors.push(Monitor {
            tag_set: [real_tags; 2],
            overview_state: Some(crate::overview::OverviewState::new(
                TagMask::all(4),
                Vec::new(),
                std::collections::HashMap::new(),
                None,
            )),
            ..Monitor::default()
        });
        model.set_selected_monitor(monitor_id);

        assert_eq!(
            model.expect_selected_monitor().visible_tags(),
            TagMask::all(4)
        );
        assert_eq!(current_launch_context(&model).tags, real_tags);

        let mut client = Client::new(WindowId(30));
        assert!(assign_initial_monitor_and_tags(
            &model,
            &mut client,
            None,
            None,
        ));
        assert_eq!(client.tags, real_tags);
    }
}
