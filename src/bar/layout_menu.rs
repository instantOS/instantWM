//! Placement of the external chooser comes from the bar's rendered hit cache,
//! shared by X11 and Wayland, rather than querying the pointer in a new client.
use crate::contexts::WmCtx;
use crate::types::Monitor;

/// State already available to the WM at the time the chooser is launched.
/// Passing this snapshot avoids querying the compositor before drawing it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutMenuContext {
    pub monitor_id: u64,
    pub tags: Vec<u32>,
    pub active: crate::layouts::LayoutCommand,
}

impl LayoutMenuContext {
    fn from_monitor(monitor: &Monitor) -> Self {
        Self {
            monitor_id: monitor.id().get(),
            tags: monitor
                .visible_tags()
                .iter()
                .filter(|&tag| tag <= monitor.tags.len())
                .map(|tag| (tag - 1) as u32)
                .collect(),
            active: monitor.current_layout_command(),
        }
    }
}

fn anchor(monitor: &Monitor, layout_start: i32) -> (i32, i32) {
    (
        monitor.work_rect().x - monitor.monitor_rect.x + layout_start,
        monitor.bar_y() + monitor.bar_height - monitor.monitor_rect.y,
    )
}

pub(crate) fn open(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let core = ctx.core();
    let monitor = core.state.model.expect_selected_monitor();
    let hit = core
        .bar
        .monitor_hit_cache(monitor.id())
        .ok_or("layout button has not been rendered yet")?;
    let (x, y) = anchor(monitor, hit.layout_start);
    let snapshot = serde_json::to_string(&LayoutMenuContext::from_monitor(monitor))
        .map_err(|error| error.to_string())?;
    let argv = vec![
        "instantwmctl".into(),
        "layout".into(),
        "menu".into(),
        "--context".into(),
        snapshot,
        "--monitor".into(),
        monitor.num.max(0).to_string(),
        "--x-offset".into(),
        x.to_string(),
        "--y-offset".into(),
        y.to_string(),
    ];
    crate::util::spawn(ctx, &argv).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MonitorBuilder;
    use crate::types::Rect;

    #[test]
    fn launch_snapshot_uses_the_same_zero_based_tags_as_ipc() {
        use crate::types::TagMask;
        let monitor = MonitorBuilder::new()
            .tag_count(3)
            .selected_tags(TagMask::single(1).unwrap() | TagMask::single(3).unwrap())
            .build();
        let snapshot = LayoutMenuContext::from_monitor(&monitor);
        assert_eq!(snapshot.tags, [0, 2]);
        assert_eq!(snapshot.active, monitor.current_layout_command());
        let roundtrip: LayoutMenuContext =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(roundtrip.monitor_id, snapshot.monitor_id);
        assert_eq!(roundtrip.tags, snapshot.tags);
        assert_eq!(roundtrip.active, snapshot.active);
    }

    #[test]
    fn anchor_handles_negative_output_origins_and_reserved_side_space() {
        let monitor = MonitorBuilder::new()
            .monitor_rect(Rect::new(-1920, -200, 1920, 1080))
            .configure(|monitor| {
                monitor.available_rect = Rect::new(-1900, -200, 1900, 1080);
                monitor.bar_height = 36;
            })
            .build();
        assert_eq!(anchor(&monitor, 140), (160, 36));
    }
}
