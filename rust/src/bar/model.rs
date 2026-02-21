use crate::globals::Globals;
use crate::types::MonitorInner;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientBarStats {
    pub occupied_tags: u32,
    pub urgent_tags: u32,
    pub visible_clients: i32,
}

impl ClientBarStats {
    pub(crate) fn collect(monitor: &MonitorInner, globals: &Globals) -> Self {
        let mut stats = Self::default();

        for client in globals.clients.values() {
            let on_selected_monitor = client.mon_id.map_or(false, |mon_id| {
                globals
                    .selmon
                    .map_or(false, |selmon_idx| mon_id == selmon_idx)
            });

            if !on_selected_monitor {
                continue;
            }

            if client.is_visible() {
                stats.visible_clients += 1;
            }

            stats.occupied_tags |= if client.tags == 255 { 0 } else { client.tags };
            if client.isurgent {
                stats.urgent_tags |= client.tags;
            }
        }

        stats
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BarLayout {
    pub systray_width: i32,
    pub status_start_x: i32,
    pub title_width: i32,
}
