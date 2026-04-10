use crate::bar::paint::BarPainter;
use crate::bar::scene::MonitorBarSnapshot;
use crate::contexts::CoreCtx;
use crate::types::{Gesture, MonitorId};

/// Core bar drawing implementation shared between backends.
///
/// Systray width must be cached in `core.globals().bar_runtime.systray_width` by the caller
/// before invoking this function.
pub(crate) fn draw_bar(core: &mut CoreCtx, mon_idx: MonitorId, painter: &mut dyn BarPainter) {
    let monitor = match core.globals().monitor(mon_idx).cloned() {
        Some(m) => m,
        None => return,
    };

    let snapshots = crate::bar::scene::build_monitor_snapshots(core, None);
    let Some(snapshot) = snapshots.iter().find(|s| s.monitor_id == monitor.id()) else {
        return;
    };

    draw_bar_snapshot(core, mon_idx, &monitor, snapshot, painter);
}

pub(crate) fn draw_bar_snapshot(
    core: &mut CoreCtx,
    mon_idx: MonitorId,
    monitor: &crate::types::Monitor,
    snapshot: &MonitorBarSnapshot,
    painter: &mut dyn BarPainter,
) {
    if !core.bar.try_recursion_enter() {
        return;
    }

    if !monitor.shows_bar() || core.bar.pausedraw() {
        core.bar.recursion_exit();
        return;
    }

    let bar_height = monitor.bar_height;
    if monitor.work_rect.w <= 0 || bar_height <= 0 {
        core.bar.recursion_exit();
        return;
    }

    core.bar.clear_cached_widths();
    let output = crate::bar::scene::render_monitor_snapshot(snapshot, painter);
    core.bar
        .replace_hit_cache(snapshot.monitor_id, output.hit_cache);

    if let Some(mon) = core.globals_mut().monitor_mut(mon_idx) {
        mon.bar_clients_width = output.bar_clients_width;
        mon.activeoffset = output.activeoffset;
    }

    core.bar.recursion_exit();
}

pub fn reset_bar_common(core: &mut CoreCtx) {
    let selmon = core.globals().selected_monitor();
    if selmon.gesture == Gesture::None {
        return;
    }

    core.globals_mut().selected_monitor_mut().gesture = Gesture::None;
}
