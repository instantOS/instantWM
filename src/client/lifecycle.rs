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
    pub pid: Option<u32>,
    pub startup_id: Option<String>,
    pub context: LaunchContext,
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
    pid: Option<u32>,
    startup_id: Option<String>,
    context: LaunchContext,
) {
    prune_pending_launches(pending_launches);
    pending_launches.push_back(PendingLaunch {
        recorded_at: Instant::now(),
        pid,
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
    let pid_match = pid.and_then(|target_pid| {
        pending_launches
            .iter()
            .position(|launch| launch.pid == Some(target_pid))
    });
    let idx = startup_match.or(pid_match)?;

    pending_launches.remove(idx).map(|launch| launch.context)
}

fn prune_pending_launches(pending_launches: &mut VecDeque<PendingLaunch>) {
    let now = Instant::now();
    pending_launches.retain(|launch| now.duration_since(launch.recorded_at) <= PENDING_LAUNCH_TTL);
}
