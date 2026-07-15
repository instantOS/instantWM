use crate::bar::paint::BarPainter;
use crate::bar::scene::MonitorBarSnapshot;
use crate::contexts::CoreCtx;
use crate::types::{Gesture, MonitorId};

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

    if !monitor.shows_bar() {
        core.bar.recursion_exit();
        return;
    }

    let bar_height = monitor.bar_height;
    if monitor.work_rect().w <= 0 || bar_height <= 0 {
        core.bar.recursion_exit();
        return;
    }

    core.bar.clear_cached_widths();
    let output = crate::bar::scene::render_monitor_snapshot(snapshot, painter);
    core.bar
        .replace_hit_cache(snapshot.monitor_id, output.hit_cache);

    if let Some(mon) = core.model_mut().monitor_mut(mon_idx) {
        mon.bar_clients_width = output.bar_clients_width;
        mon.activeoffset = output.activeoffset;
    }

    core.bar.recursion_exit();
}

pub fn reset_bar_common(model: &mut crate::model::WmModel) {
    if model.selected_monitor().gesture == Gesture::None {
        return;
    }

    model.selected_monitor_mut().gesture = Gesture::None;
}
