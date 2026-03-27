//! Client/window management types.
//!
//! Types for managed windows and client lists.

use std::collections::HashMap;

use crate::types::TagMask;
use crate::types::WindowId;
use crate::types::core::MonitorId;
use crate::types::geometry::{Rect, SizeHints};
use crate::types::input::SnapPosition;

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

    /// Whether size hints are valid.
    pub size_hints_valid: i32,
    /// Current border width.
    pub border_width: i32,
    /// Previous border width.
    pub old_border_width: i32,
    /// Tags this client belongs to.
    pub tags: TagMask,
    /// Whether the window has fixed size.
    pub is_fixed_size: bool,
    /// Whether the window is floating.
    pub is_floating: bool,
    /// Whether the window has urgency hint.
    pub is_urgent: bool,
    /// Whether the window should never receive focus.
    pub never_focus: bool,
    /// Old window state.
    pub oldstate: i32,
    /// Whether the window is fullscreen.
    pub is_fullscreen: bool,
    /// Whether the window is in fake fullscreen mode.
    pub isfakefullscreen: bool,
    /// Whether the window is locked (can't be closed accidentally).
    pub is_locked: bool,
    /// Whether the window is sticky (visible on all tags).
    pub issticky: bool,
    /// Whether the window is minimized/hidden.
    pub is_hidden: bool,
    /// Current snap position.
    pub snap_status: SnapPosition,
    /// Scratchpad name (empty if not a scratchpad).
    pub scratchpad_name: String,
    /// Tags to restore when unhiding from scratchpad.
    pub scratchpad_restore_tags: TagMask,
    /// Monitor this client is on.
    pub monitor_id: MonitorId,
    /// Window ID.
    pub win: WindowId,
}

impl Client {
    /// Check whether this client still carries scratchpad metadata.
    #[inline]
    pub fn has_scratchpad_identity(&self) -> bool {
        !self.scratchpad_name.is_empty()
    }

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
        self.has_scratchpad_identity()
            && (self.tags.is_scratchpad_only() || self.is_hidden || self.issticky)
    }

    /// Clear scratchpad-only metadata after the window has been moved to normal tags.
    pub fn clear_scratchpad_state(&mut self) {
        self.scratchpad_name.clear();
        self.scratchpad_restore_tags = TagMask::EMPTY;
        self.issticky = false;
    }

    /// Keep scratchpad metadata consistent with the current tag assignment.
    pub fn sync_scratchpad_state(&mut self) {
        if self.has_scratchpad_identity()
            && !self.tags.is_scratchpad_only()
            && !self.is_hidden
            && !self.issticky
        {
            self.clear_scratchpad_state();
        }
    }

    /// Assign a new tag bitmask and normalize any dependent client state.
    pub fn set_tag_mask(&mut self, tags: TagMask) {
        self.tags = tags;
        self.sync_scratchpad_state();
    }

    /// Transform the tag bitmask in place and normalize dependent client state.
    pub fn update_tag_mask(&mut self, f: impl FnOnce(TagMask) -> TagMask) {
        self.tags = f(self.tags);
        self.sync_scratchpad_state();
    }

    /// Check if the client is on the selected tags, ignoring hidden state.
    #[inline]
    pub fn is_on_selected_tags(&self, selected_tags: TagMask) -> bool {
        self.issticky || self.tags.intersects(selected_tags)
    }

    /// Check if the client is actually visible for the given tag-set.
    #[inline]
    pub fn is_visible(&self, selected_tags: TagMask) -> bool {
        self.is_on_selected_tags(selected_tags) && !self.is_hidden
    }

    /// Check if this client should be included in tiling calculations.
    ///
    /// Returns true if the client is:
    /// - Not floating
    /// - Not in true fullscreen mode
    /// - Visible on the selected tags
    /// - Not hidden
    #[inline]
    pub fn is_tiled(&self, selected_tags: TagMask) -> bool {
        !self.is_floating && !self.is_true_fullscreen() && self.is_visible(selected_tags)
    }

    /// Clear the urgency flag for this client.
    pub fn clear_urgency(&mut self) {
        self.is_urgent = false;
    }

    /// Check if the client is in true fullscreen mode (not fake fullscreen).
    #[inline]
    pub fn is_true_fullscreen(&self) -> bool {
        self.is_fullscreen && !self.isfakefullscreen
    }

    /// Get the border width.
    #[inline]
    pub fn border_width(&self) -> i32 {
        self.border_width
    }

    /// Get the monitor's size (width, height) for this client.
    ///
    /// Returns `(0, 0)` if the client is not assigned to a monitor.
    pub fn monitor_size(&self, globals: &crate::globals::Globals) -> (i32, i32) {
        globals
            .monitor(self.monitor_id)
            .map(|m| (m.monitor_rect.w, m.monitor_rect.h))
            .unwrap_or((0, 0))
    }

    /// Returns the floating geometry if valid, otherwise falls back to current geometry.
    ///
    /// When a window has never been floated, `float_geo` is zeroed. This method
    /// provides the correct dimensions to use for floating: saved float dimensions
    /// if available, otherwise the current tiled dimensions.
    pub fn effective_float_geo(&self) -> Rect {
        if self.float_geo.is_valid() {
            self.float_geo
        } else {
            self.geo
        }
    }

    /// Returns the geometry to use when restoring a window from tiled to floating.
    ///
    /// If the window is already floating, returns current geometry.
    /// Otherwise returns effective float geometry (saved float dims or current tiled dims).
    pub fn restore_geo_for_float(&self) -> Rect {
        if self.is_floating {
            self.geo
        } else {
            self.effective_float_geo()
        }
    }

    pub fn update_geometry(&mut self, rect: Rect) {
        self.old_geo = self.geo;
        self.geo = rect;
    }

    pub fn save_border_width(&mut self) {
        if self.border_width != 0 {
            self.old_border_width = self.border_width;
        }
    }

    pub fn restore_border_width(&mut self) {
        if self.old_border_width != 0 {
            self.border_width = self.old_border_width;
        }
    }

    pub fn set_tags(
        &mut self,
        mask: crate::types::TagMask,
        core: &mut crate::contexts::CoreCtx,
        x11: &crate::backend::x11::X11BackendRef,
        x11_runtime: &mut crate::backend::x11::X11RuntimeConfig,
    ) {
        let tag_mask = TagMask::from_bits(core.globals().tags.mask());
        let effective_mask = mask & tag_mask;

        if effective_mask.is_empty() {
            return;
        }

        if self.tags.is_scratchpad_only() {
            self.issticky = false;
        }

        self.set_tag_mask(effective_mask);

        crate::backend::x11::properties::set_client_tag_prop(core, x11, x11_runtime, self.win);
        crate::focus::focus_soft_x11(core, x11, x11_runtime, None);
        let selmon_id = core.globals().selected_monitor_id();
        crate::layouts::arrange(
            &mut crate::contexts::WmCtx::X11(crate::contexts::WmCtxX11 {
                core: core.reborrow(),
                backend: crate::backend::BackendRef::from_x11(x11.conn, x11.screen_num),
                x11: crate::backend::x11::X11BackendRef::new(x11.conn, x11.screen_num),
                x11_runtime,
                systray: None,
            }),
            Some(selmon_id),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Client;
    use crate::types::{SCRATCHPAD_MASK, TagMask};

    #[test]
    fn scratchpad_requires_scratchpad_tag() {
        let client = Client {
            scratchpad_name: "term".to_string(),
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        assert!(!client.is_scratchpad());
    }

    #[test]
    fn sync_clears_stale_scratchpad_metadata() {
        let mut client = Client {
            scratchpad_name: "term".to_string(),
            scratchpad_restore_tags: TagMask::single(2).unwrap(),
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert!(client.scratchpad_name.is_empty());
        assert_eq!(client.scratchpad_restore_tags, TagMask::EMPTY);
        assert!(!client.issticky);
    }

    #[test]
    fn sync_keeps_valid_scratchpad_metadata() {
        let mut client = Client {
            scratchpad_name: "term".to_string(),
            scratchpad_restore_tags: TagMask::single(2).unwrap(),
            issticky: true,
            tags: TagMask::from_bits(SCRATCHPAD_MASK),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert_eq!(client.scratchpad_name, "term");
        assert_eq!(client.scratchpad_restore_tags, TagMask::single(2).unwrap());
        assert!(client.issticky);
        assert!(client.is_scratchpad());
    }

    #[test]
    fn sync_keeps_hidden_scratchpad_metadata_off_scratchpad_tag() {
        let mut client = Client {
            scratchpad_name: "term".to_string(),
            scratchpad_restore_tags: TagMask::single(2).unwrap(),
            is_hidden: true,
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert_eq!(client.scratchpad_name, "term");
        assert!(client.is_scratchpad());
    }

    #[test]
    fn sync_keeps_sticky_scratchpad_metadata_off_scratchpad_tag() {
        let mut client = Client {
            scratchpad_name: "term".to_string(),
            scratchpad_restore_tags: TagMask::single(2).unwrap(),
            issticky: true,
            tags: TagMask::single(1).unwrap(),
            ..Client::default()
        };

        client.sync_scratchpad_state();

        assert_eq!(client.scratchpad_name, "term");
        assert!(client.is_scratchpad());
    }
}

/// Lightweight snapshot of a tiled client for layout calculations.
///
/// Layout algorithms collect these once and then work purely with
/// geometry — no further access to `ClientManager` needed.
#[derive(Debug, Clone, Copy)]
pub struct TiledClientInfo {
    pub win: WindowId,
    pub border_width: i32,
    pub total_height: i32,
    pub total_width: i32,
}

/// Iterator over a monitor's client list (focus order).
///
/// Yields `(Window, &Client)` pairs so call-sites keep the window id and the
/// corresponding client tightly coupled.
pub struct ClientListIter<'a> {
    iter: std::slice::Iter<'a, WindowId>,
    clients: &'a HashMap<WindowId, Client>,
}

impl<'a> ClientListIter<'a> {
    #[inline]
    pub fn new(clients: &'a [WindowId], map: &'a HashMap<WindowId, Client>) -> Self {
        Self {
            iter: clients.iter(),
            clients: map,
        }
    }
}

impl<'a> Iterator for ClientListIter<'a> {
    type Item = (WindowId, &'a Client);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let win = match self.iter.next() {
                Some(&w) => w,
                None => return None,
            };
            if let Some(c) = self.clients.get(&win) {
                return Some((win, c));
            }
        }
    }
}

/// Iterator over a monitor's stack list (stacking order).
///
/// Yields `(Window, &Client)` pairs so restack/showhide style logic can use the
/// correct ordering while keeping the window id available.
///
/// This uses the same implementation as [`ClientListIter`] — the distinction
/// is semantic (stacking order vs focus order).
pub struct ClientStackIter<'a>(ClientListIter<'a>);

impl<'a> ClientStackIter<'a> {
    #[inline]
    pub fn new(stack: &'a [WindowId], map: &'a HashMap<WindowId, Client>) -> Self {
        Self(ClientListIter::new(stack, map))
    }
}

impl<'a> Iterator for ClientStackIter<'a> {
    type Item = <ClientListIter<'a> as Iterator>::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}
