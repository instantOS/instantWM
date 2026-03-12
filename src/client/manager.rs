//! Structured data and low-level logic for clients.

use crate::types::{Client, ClientId, WindowId};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

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

impl DerefMut for ClientManager {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.clients
    }
}

impl ClientManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map(&self) -> &HashMap<WindowId, Client> {
        &self.clients
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
}
