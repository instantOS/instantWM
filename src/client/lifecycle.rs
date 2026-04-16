//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::globals::Globals;
use crate::types::{MonitorId, TagMask, WindowId};
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PENDING_LAUNCH_TTL: Duration = Duration::from_secs(30);
const MAX_PENDING_LAUNCHES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchContext {
    pub monitor_id: MonitorId,
    pub tags: TagMask,
    pub is_floating: bool,
}

#[derive(Debug, Clone)]
pub struct PendingLaunch {
    pub recorded_at: Instant,
    pub pid: Option<u32>,
    pub startup_id: Option<String>,
    pub context: LaunchContext,
}

pub fn current_launch_context(g: &Globals) -> LaunchContext {
    LaunchContext {
        monitor_id: g.selected_monitor_id(),
        tags: g.selected_monitor().selected_tags(),
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
    g: &mut Globals,
    pid: Option<u32>,
    startup_id: Option<String>,
    context: LaunchContext,
) {
    prune_pending_launches(&mut g.pending_launches);
    g.pending_launches.push_back(PendingLaunch {
        recorded_at: Instant::now(),
        pid,
        startup_id,
        context,
    });
    while g.pending_launches.len() > MAX_PENDING_LAUNCHES {
        g.pending_launches.pop_front();
    }
}

pub fn take_pending_launch(
    g: &mut Globals,
    pid: Option<u32>,
    startup_id: Option<&str>,
) -> Option<LaunchContext> {
    prune_pending_launches(&mut g.pending_launches);

    let startup_match = startup_id.and_then(|id| {
        g.pending_launches
            .iter()
            .position(|launch| launch.startup_id.as_deref() == Some(id))
    });
    let pid_match = pid.and_then(|target_pid| {
        g.pending_launches
            .iter()
            .position(|launch| launch.pid == Some(target_pid))
    });
    let idx = startup_match.or(pid_match)?;

    g.pending_launches.remove(idx).map(|launch| launch.context)
}

fn prune_pending_launches(pending_launches: &mut VecDeque<PendingLaunch>) {
    let now = Instant::now();
    pending_launches.retain(|launch| now.duration_since(launch.recorded_at) <= PENDING_LAUNCH_TTL);
}

/// Initial tag mask for a newly managed client on `monitor_id`.
///
/// This mirrors DWM semantics: a new client appears on all tags currently
/// visible on its target monitor.
pub fn initial_tags_for_monitor(g: &Globals, monitor_id: MonitorId) -> TagMask {
    g.monitor(monitor_id)
        .map(|m| m.selected_tags())
        .filter(|tags| !tags.is_empty())
        .unwrap_or(TagMask::single(1).unwrap_or(TagMask::EMPTY))
}

/// Select `win` on its assigned monitor.
///
/// This is WM policy, not backend policy: backends may discover a new window
/// or an activation request, but the choice to make that window the monitor's
/// selected client lives in shared state.
pub fn select_client(g: &mut Globals, win: WindowId) {
    let Some(monitor_id) = g.clients.monitor_id(win) else {
        return;
    };
    let is_tiled = g
        .clients
        .get(&win)
        .is_some_and(|client| !client.is_floating);
    if let Some(mon) = g.monitor_mut(monitor_id) {
        mon.sel = Some(win);
        if is_tiled {
            mon.tag_tiled_focus_history
                .insert(mon.selected_tags().bits(), win);
        }
    }
}
