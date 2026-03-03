//! Client/window management types.
//!
//! Types for managed windows and client lists.

use std::collections::HashMap;

use crate::types::core::MonitorId;
use crate::types::geometry::{Rect, SizeHints};
use crate::types::input::SnapPosition;
use crate::types::WindowId;

/// Represents a managed client window in the window manager.
///
/// This struct contains all state for a window managed by instantWM,
/// including geometry, tags, flags, and relationships to other clients.
#[derive(Debug, Clone, Default)]
pub struct Client {
    /// Window title/name displayed in the bar.
    pub name: String,
    /// Minimum aspect ratio constraint from WM_NORMAL_HINTS.
    pub min_aspect: f32,
    /// Maximum aspect ratio constraint from WM_NORMAL_HINTS.
    pub max_aspect: f32,
    /// Current geometry.
    pub geo: Rect,
    /// Geometry when floating.
    pub float_geo: Rect,
    /// Previous geometry (for restoring).
    pub old_geo: Rect,
    /// Size hints from WM_NORMAL_HINTS property.
    pub size_hints: SizeHints,

    // Backward-compatible size hint fields
    /// Base width.
    pub base_width: i32,
    /// Base height.
    pub base_height: i32,
    /// Minimum width.
    pub min_width: i32,
    /// Minimum height.
    pub min_height: i32,
    /// Maximum width.
    pub max_width: i32,
    /// Maximum height.
    pub max_height: i32,
    /// Width increment.
    pub inc_width: i32,
    /// Height increment.
    pub inc_height: i32,
    /// Base aspect numerator.
    pub base_aspect_num: i32,
    /// Base aspect denominator.
    pub base_aspect_denom: i32,
    /// Minimum aspect numerator.
    pub min_aspect_num: i32,
    /// Minimum aspect denominator.
    pub min_aspect_denom: i32,
    /// Maximum aspect numerator.
    pub max_aspect_num: i32,
    /// Maximum aspect denominator.
    pub max_aspect_denom: i32,

    /// Whether size hints are valid.
    pub hintsvalid: i32,
    /// Current border width.
    pub border_width: i32,
    /// Previous border width.
    pub old_border_width: i32,
    /// Tags this client belongs to (bitmask).
    pub tags: u32,
    /// Whether the window has fixed size.
    pub isfixed: bool,
    /// Whether the window is floating.
    pub isfloating: bool,
    /// Whether the window has urgency hint.
    pub isurgent: bool,
    /// Whether the window should never receive focus.
    pub neverfocus: bool,
    /// Old window state.
    pub oldstate: i32,
    /// Whether the window is fullscreen.
    pub is_fullscreen: bool,
    /// Whether the window is in fake fullscreen mode.
    pub isfakefullscreen: bool,
    /// Whether the window is locked (can't be closed accidentally).
    pub islocked: bool,
    /// Whether the window is sticky (visible on all tags).
    pub issticky: bool,
    /// Whether the window is minimized/hidden.
    pub is_hidden: bool,
    /// Current snap position.
    pub snapstatus: SnapPosition,
    /// Scratchpad name (empty if not a scratchpad).
    pub scratchpad_name: String,
    /// Tags to restore when unhiding from scratchpad.
    pub scratchpad_restore_tags: u32,
    /// Monitor this client is on.
    pub mon_id: Option<MonitorId>,
    /// Window ID.
    pub win: WindowId,
    /// Next client in the client list (focus order).
    pub next: Option<WindowId>,
    /// Next client in the stack list (stacking order).
    pub snext: Option<WindowId>,
}

impl Client {
    /// Calculate total width including borders.
    pub fn total_width(&self) -> i32 {
        self.geo.total_width(self.border_width)
    }

    /// Calculate total height including borders.
    pub fn total_height(&self) -> i32 {
        self.geo.total_height(self.border_width)
    }

    /// Check if this client is a scratchpad window.
    pub fn is_scratchpad(&self) -> bool {
        !self.scratchpad_name.is_empty()
    }

    /// Check if the client should be visible for a given tag-set.
    ///
    /// This is intentionally pure: callers provide the currently selected
    /// tag-mask for the monitor the client is on.
    #[inline]
    pub fn is_visible_on_tags(&self, selected_tags: u32) -> bool {
        self.issticky || (self.tags & selected_tags) != 0
    }

    /// Check if the client is in true fullscreen mode (not fake fullscreen).
    #[inline]
    pub fn is_true_fullscreen(&self) -> bool {
        self.is_fullscreen && !self.isfakefullscreen
    }

    /// Get the border width and next client in focus order.
    #[inline]
    pub fn border_and_next(&self) -> (i32, Option<WindowId>) {
        (self.border_width, self.next)
    }
}

/// Iterator over a monitor's client list (focus order).
///
/// Yields `(Window, &Client)` pairs so call-sites keep the window id and the
/// corresponding client tightly coupled.
pub struct ClientListIter<'a> {
    next: Option<WindowId>,
    clients: &'a HashMap<WindowId, Client>,
}

impl<'a> ClientListIter<'a> {
    /// Create a new client list iterator.
    #[inline]
    pub fn new(head: Option<WindowId>, clients: &'a HashMap<WindowId, Client>) -> Self {
        Self {
            next: head,
            clients,
        }
    }
}

impl<'a> Iterator for ClientListIter<'a> {
    type Item = (WindowId, &'a Client);

    fn next(&mut self) -> Option<Self::Item> {
        let win = self.next?;
        let clients = self.clients;
        let c = clients.get(&win)?;
        self.next = c.next;
        Some((win, c))
    }
}

/// Iterator over a monitor's stack list (stacking order).
///
/// Yields `(Window, &Client)` pairs so restack/showhide style logic can use the
/// correct ordering while keeping the window id available.
pub struct ClientStackIter<'a> {
    next: Option<WindowId>,
    clients: &'a HashMap<WindowId, Client>,
}

impl<'a> ClientStackIter<'a> {
    /// Create a new stack list iterator.
    #[inline]
    pub fn new(head: Option<WindowId>, clients: &'a HashMap<WindowId, Client>) -> Self {
        Self {
            next: head,
            clients,
        }
    }
}

impl<'a> Iterator for ClientStackIter<'a> {
    type Item = (WindowId, &'a Client);

    fn next(&mut self) -> Option<Self::Item> {
        let win = self.next?;
        let clients = self.clients;
        let c = clients.get(&win)?;
        self.next = c.snext;
        Some((win, c))
    }
}
