//! Structured data and low-level logic for clients.

use crate::types::{Client, ClientId, WindowId};
use std::collections::HashMap;
use std::ops::Deref;

#[derive(Default)]
pub struct ClientManager {
    pub clients: HashMap<WindowId, Client>,
    pub client_list: Vec<ClientId>,
}

impl Deref for ClientManager {
    type Target = HashMap<WindowId, Client>;

    fn deref(&self) -> &Self::Target {
        &self.clients
    }
}

impl ClientManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -------------------------------------------------------------------------
    // Map access
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
