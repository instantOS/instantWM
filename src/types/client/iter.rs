//! Iterators and layout snapshots for ordered client collections.

use std::collections::HashMap;

use super::Client;
use crate::types::WindowId;

/// Lightweight snapshot of a tiled client for layout calculations.
///
/// Layout algorithms collect these once and then work purely with geometry.
#[derive(Debug, Clone, Copy)]
pub struct TiledClientInfo {
    pub win: WindowId,
    pub border_width: i32,
}

/// Iterator joining an ordered window list with the client map.
///
/// Stale IDs are skipped so callers never have to separate ordering from
/// lookup or handle partially removed clients.
pub struct OrderedClients<'a> {
    windows: std::slice::Iter<'a, WindowId>,
    clients: &'a HashMap<WindowId, Client>,
}

impl<'a> OrderedClients<'a> {
    #[inline]
    pub fn new(windows: &'a [WindowId], clients: &'a HashMap<WindowId, Client>) -> Self {
        Self {
            windows: windows.iter(),
            clients,
        }
    }
}

impl<'a> Iterator for OrderedClients<'a> {
    type Item = (WindowId, &'a Client);

    fn next(&mut self) -> Option<Self::Item> {
        self.windows
            .find_map(|window| self.clients.get(window).map(|client| (*window, client)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::OrderedClients;
    use crate::types::{Client, WindowId};

    #[test]
    fn preserves_order_while_skipping_stale_window_ids() {
        let first = WindowId(1);
        let stale = WindowId(2);
        let last = WindowId(3);
        let clients = HashMap::from([(first, Client::new(first)), (last, Client::new(last))]);

        let windows = [last, stale, first];
        let ordered = OrderedClients::new(&windows, &clients)
            .map(|(window, _)| window)
            .collect::<Vec<_>>();

        assert_eq!(ordered, [last, first]);
    }
}
