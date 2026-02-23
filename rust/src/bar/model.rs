use crate::globals::Globals;
use crate::types::Monitor;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientBarStats {
    pub occupied_tags: u32,
    pub urgent_tags: u32,
    pub visible_clients: i32,
}

impl ClientBarStats {
    pub(crate) fn collect(_monitor: &Monitor, globals: &Globals) -> Self {
        let mut stats = Self::default();

        for client in globals.clients.values() {
            let on_selected_monitor = client.mon_id == Some(globals.selmon);

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
