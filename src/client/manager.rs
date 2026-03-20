//! Structured data and low-level logic for clients.

use crate::types::{Client, ClientId, WindowId};
use std::collections::HashMap;

#[derive(Default)]
pub struct ClientManager {
    clients: HashMap<WindowId, Client>,
    pub client_list: Vec<ClientId>,
}

impl ClientManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map(&self) -> &HashMap<WindowId, Client> {
        &self.clients
    }

    pub fn get(&self, win: &WindowId) -> Option<&Client> {
        self.clients.get(win)
    }

    pub fn get_mut(&mut self, win: &WindowId) -> Option<&mut Client> {
        self.clients.get_mut(win)
    }

    pub fn contains_key(&self, win: &WindowId) -> bool {
        self.clients.contains_key(win)
    }

    pub fn insert(&mut self, win: WindowId, client: Client) -> Option<Client> {
        self.clients.insert(win, client)
    }

    pub fn remove(&mut self, win: &WindowId) -> Option<Client> {
        self.clients.remove(win)
    }

    pub fn values(&self) -> std::collections::hash_map::Values<'_, WindowId, Client> {
        self.clients.values()
    }

    pub fn keys(&self) -> std::collections::hash_map::Keys<'_, WindowId, Client> {
        self.clients.keys()
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, WindowId, Client> {
        self.clients.iter()
    }

    pub fn monitor_id(&self, win: WindowId) -> Option<usize> {
        self.clients.get(&win).map(|c| c.monitor_id)
    }

    pub fn is_hidden(&self, win: WindowId) -> bool {
        self.clients.get(&win).is_some_and(|c| c.is_hidden)
    }

    pub fn is_floating(&self, win: WindowId) -> bool {
        self.clients.get(&win).is_some_and(|c| c.is_floating)
    }

    pub fn is_locked(&self, win: WindowId) -> bool {
        self.clients.get(&win).is_none_or(|c| c.is_locked)
    }

    pub fn geo(&self, win: WindowId) -> Option<crate::types::Rect> {
        self.clients.get(&win).map(|c| c.geo)
    }

    pub fn tags(&self, win: WindowId) -> Option<u32> {
        self.clients.get(&win).map(|c| c.tags)
    }

    pub fn effective_float_geo(&self, win: WindowId) -> Option<crate::types::Rect> {
        self.clients.get(&win).map(|c| c.effective_float_geo())
    }

    pub fn win_to_client(&self, win: WindowId) -> Option<WindowId> {
        self.clients.contains_key(&win).then_some(win)
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
            client.update_geometry(rect);
        }
    }
}
