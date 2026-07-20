use crate::bar::paint::BarPainter;
use crate::bar::scene::MonitorBarSnapshot;
use crate::contexts::CoreCtx;
use crate::types::MonitorId;

pub(crate) fn draw_bar_snapshot(
    core: &mut CoreCtx,
    mon_idx: MonitorId,
    snapshot: &MonitorBarSnapshot,
    painter: &mut dyn BarPainter,
) {
    core.bar.clear_cached_widths();
    let output = crate::bar::scene::render_monitor_snapshot(snapshot, painter);
    core.bar
        .replace_hit_cache(snapshot.monitor_id, output.hit_cache);

    if let Some(mon) = core.model_mut().monitor_mut(mon_idx) {
        mon.bar_clients_width = output.bar_clients_width;
        mon.activeoffset = output.activeoffset;
    }
}
