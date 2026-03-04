//! Structured data and low-level logic for clients.

use crate::monitor::MonitorManager;
use crate::types::{Client, ClientId, WindowId};
use std::collections::HashMap;

#[derive(Default)]
pub struct ClientManager {
    pub clients: HashMap<WindowId, Client>,
    pub client_list: Vec<ClientId>,
}

impl ClientManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -------------------------------------------------------------------------
    // List Invariants (The "Plumbing")
    // -------------------------------------------------------------------------

    pub fn attach(&mut self, monitors: &mut MonitorManager, win: WindowId) {
        let mon_id = match self.clients.get(&win).and_then(|c| c.mon_id) {
            Some(id) => id,
            None => return,
        };
        let old_head = monitors.get(mon_id).and_then(|m| m.clients);
        if let Some(c) = self.clients.get_mut(&win) {
            c.next = old_head;
        }
        if let Some(mon) = monitors.get_mut(mon_id) {
            mon.clients = Some(win);
        }
    }

    pub fn detach(&mut self, monitors: &mut MonitorManager, win: WindowId) {
        let mon_id = match self.clients.get(&win).and_then(|c| c.mon_id) {
            Some(id) => id,
            None => return,
        };
        let client_next = self.clients.get(&win).and_then(|c| c.next);
        let mut current = monitors.get(mon_id).and_then(|m| m.clients);
        let mut prev: Option<WindowId> = None;

        while let Some(cur_win) = current {
            if cur_win == win {
                if let Some(p) = prev {
                    if let Some(pc) = self.clients.get_mut(&p) {
                        pc.next = client_next;
                    }
                } else if let Some(mon) = monitors.get_mut(mon_id) {
                    mon.clients = client_next;
                }
                return;
            }
            prev = Some(cur_win);
            current = self.clients.get(&cur_win).and_then(|c| c.next);
        }
    }

    pub fn attach_stack(&mut self, monitors: &mut MonitorManager, win: WindowId) {
        let mon_id = match self.clients.get(&win).and_then(|c| c.mon_id) {
            Some(id) => id,
            None => return,
        };
        let old_stack = monitors.get(mon_id).and_then(|m| m.stack);
        if let Some(c) = self.clients.get_mut(&win) {
            c.snext = old_stack;
        }
        if let Some(mon) = monitors.get_mut(mon_id) {
            mon.stack = Some(win);
            if mon.sel.is_none() {
                mon.sel = Some(win);
            }
        }
    }

    pub fn detach_stack(&mut self, monitors: &mut MonitorManager, win: WindowId) {
        let mon_id = match self.clients.get(&win).and_then(|c| c.mon_id) {
            Some(id) => id,
            None => return,
        };
        let client_snext = self.clients.get(&win).and_then(|c| c.snext);
        let mut current = monitors.get(mon_id).and_then(|m| m.stack);
        let mut prev: Option<WindowId> = None;

        while let Some(cur_win) = current {
            if cur_win == win {
                if let Some(p) = prev {
                    if let Some(pc) = self.clients.get_mut(&p) {
                        pc.snext = client_snext;
                    }
                } else if let Some(mon) = monitors.get_mut(mon_id) {
                    mon.stack = client_snext;
                }
                if let Some(mon) = monitors.get_mut(mon_id) {
                    if mon.sel == Some(win) {
                        mon.sel = mon.stack;
                    }
                }
                return;
            }
            prev = Some(cur_win);
            current = self.clients.get(&cur_win).and_then(|c| c.snext);
        }
    }

    // -------------------------------------------------------------------------
    // High-Level Logic (Hiding the litter)
    // -------------------------------------------------------------------------

    pub fn contains(&self, win: &WindowId) -> bool {
        self.clients.contains_key(win)
    }
    pub fn len(&self) -> usize {
        self.clients.len()
    }
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
    pub fn keys(&self) -> std::collections::hash_map::Keys<'_, WindowId, Client> {
        self.clients.keys()
    }
    pub fn values(&self) -> std::collections::hash_map::Values<'_, WindowId, Client> {
        self.clients.values()
    }
    pub fn insert(&mut self, win: WindowId, client: Client) {
        self.clients.insert(win, client);
    }
    pub fn remove(&mut self, win: &WindowId) -> Option<Client> {
        self.clients.remove(win)
    }

    pub fn is_hidden(&self, win: WindowId) -> bool {
        self.clients.get(&win).map_or(false, |c| c.is_hidden)
    }

    pub fn win_to_client(&self, win: WindowId) -> Option<WindowId> {
        if self.clients.contains_key(&win) {
            Some(win)
        } else {
            None
        }
    }

    pub fn list_push(&mut self, id: ClientId) {
        self.client_list.push(id);
    }
    pub fn list_retain<F>(&mut self, f: F)
    where
        F: FnMut(&usize) -> bool,
    {
        self.client_list.retain(f);
    }

    pub fn update_geometry(&mut self, win: WindowId, rect: crate::types::Rect) {
        if let Some(client) = self.clients.get_mut(&win) {
            client.old_geo = client.geo;
            client.geo = rect;
        }
    }

    pub fn save_border_width(&mut self, win: WindowId) {
        if let Some(client) = self.clients.get_mut(&win) {
            if client.border_width != 0 {
                client.old_border_width = client.border_width;
            }
        }
    }

    pub fn restore_border_width(&mut self, win: WindowId) {
        if let Some(client) = self.clients.get_mut(&win) {
            if client.old_border_width != 0 {
                client.border_width = client.old_border_width;
            }
        }
    }

    // -------------------------------------------------------------------------
    // Legacy support
    // -------------------------------------------------------------------------

    pub fn map(&self) -> &HashMap<WindowId, Client> {
        &self.clients
    }
    pub fn get(&self, win: &WindowId) -> Option<&Client> {
        self.clients.get(win)
    }
    pub fn get_mut(&mut self, win: &WindowId) -> Option<&mut Client> {
        self.clients.get_mut(win)
    }
}
