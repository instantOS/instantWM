use crate::globals::Globals;
use crate::types::Monitor;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientBarStats {
    pub occupied_tags: u32,
    pub urgent_tags: u32,
    pub visible_clients: i32,
}

impl ClientBarStats {
    /// Collect bar statistics for the given monitor.
    ///
    /// * `visible_clients` — counted by walking the intrusive linked list so
    ///   the number exactly matches what `draw_window_titles` will draw and
    ///   what `classify_bar_click` uses for hit-testing.  The draw/hit-test
    ///   code skips clients that fail `is_visible()`, so we apply the same
    ///   predicate here.
    ///
    /// * `occupied_tags` / `urgent_tags` — accumulated from all clients on the
    ///   monitor regardless of list order; order does not matter for bitmasks.
    pub(crate) fn collect(monitor: &Monitor, globals: &Globals) -> Self {
        let mut stats = Self::default();

        // ── Pass 1: visible_clients via the linked list ───────────────────
        // Walking the linked list (monitor.clients → client.next) gives the
        // same iteration order as draw_window_titles and classify_bar_click,
        // so the count is guaranteed to be consistent with what is drawn and
        // what click regions are calculated for.
        let mut current = monitor.clients;
        while let Some(c_win) = current {
            let Some(client) = globals.clients.get(&c_win) else {
                break;
            };
            current = client.next;

            if client.is_visible() {
                stats.visible_clients += 1;
            }
        }

        // ── Pass 2: occupied / urgent tag bits from all clients on this monitor
        // Use the monitor's numeric id for matching so that clients on other
        // monitors (including ones not yet attached to any list) are excluded.
        for client in globals.clients.values() {
            if client.mon_id != Some(monitor.num as usize) {
                continue;
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
