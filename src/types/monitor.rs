//! Monitor/screen types.
//!
//! Types for managing multiple monitors/screens.

use std::collections::HashMap;

use crate::layouts::LayoutKind;
use crate::types::client::{Client, ClientListIter, ClientStackIter, TiledClientInfo};
use crate::types::geometry::Rect;
use crate::types::input::BarHoverState;
use crate::types::input::OverlayMode;
use crate::types::tag::Tag;
use crate::types::tag_types::MonitorDirection;
use crate::types::TagMask;
use crate::types::WindowId;

/// Internal state of a monitor (screen) in the window manager.
///
/// This struct holds all runtime state for a monitor, including
/// geometry, tag state, client lists, and UI configuration.
#[derive(Debug, Clone)]
pub struct Monitor {
    /// Position of this monitor in the global monitors Vec (its `MonitorId`).
    ///
    /// This is set by `Globals` whenever the monitor is inserted or the vec is
    /// compacted after a removal.  Code should read it via `Monitor::id()`
    /// rather than accessing the field directly.
    pub(crate) monitor_id: usize,
    /// Master factor for tiling layouts (0.0 to 1.0).
    pub mfact: f32,
    /// Number of clients in the master area for tiling layouts.
    pub nmaster: i32,
    /// Monitor index number (0-based).
    pub num: i32,
    /// Bar Y position (vertical position of the status bar).
    pub bar_y: i32,
    /// Width reserved for client title display in the bar.
    pub bar_clients_width: i32,
    /// Full monitor geometry (including bar).
    pub monitor_rect: Rect,
    /// Work area geometry (excluding bar).
    pub work_rect: Rect,
    /// Currently selected tag set index.
    pub sel_tags: u32,
    /// Tag sets (two sets for switching).
    pub tag_set: [u32; 2],
    /// Active offset for bar display.
    pub activeoffset: u32,
    /// Title offset for bar display.
    pub titleoffset: u32,
    /// Number of clients on this monitor.
    pub clientcount: u32,
    /// Whether to show the bar.
    pub showbar: bool,
    /// Whether the bar is at the top.
    pub topbar: bool,
    /// Overlay status.
    pub overlaystatus: i32,
    /// Overlay mode (which edge it slides from).
    pub overlaymode: OverlayMode,
    /// Current gesture state.
    pub bar_hover_state: BarHoverState,
    /// Bar window handle.
    pub bar_win: WindowId,
    /// Which tags to show.
    pub showtags: u32,
    /// Current tag index (1-based).
    pub current_tag: usize,
    /// Previous tag index (1-based).
    pub prev_tag: usize,
    /// Tags owned by this monitor.
    pub tags: Vec<Tag>,
    /// Client list (focus order).
    pub clients: Vec<WindowId>,
    /// Currently selected client.
    pub sel: Option<WindowId>,
    /// Overlay window.
    pub overlay: Option<WindowId>,
    /// Stack list (stacking order).
    pub stack: Vec<WindowId>,
    /// Currently fullscreen client.
    pub fullscreen: Option<WindowId>,
    /// Monitor name (e.g., "DP-1", "HDMI-1").
    pub name: String,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            monitor_id: 0,
            mfact: 0.55,
            nmaster: 1,
            num: 0,
            bar_y: 0,
            bar_clients_width: 0,
            monitor_rect: Rect::default(),
            work_rect: Rect::default(),
            sel_tags: 0,
            tag_set: [0; 2],
            activeoffset: 0,
            titleoffset: 0,
            clientcount: 0,
            showbar: true,
            topbar: true,
            overlaystatus: 0,
            overlaymode: OverlayMode::default(),
            bar_hover_state: BarHoverState::default(),
            bar_win: WindowId::default(),
            showtags: 0,
            current_tag: 0,
            prev_tag: 0,
            tags: Vec::new(),
            clients: Vec::new(),
            sel: None,
            overlay: None,
            stack: Vec::new(),
            fullscreen: None,
            name: String::new(),
        }
    }
}

impl Monitor {
    /// Create a new monitor with specific configuration values.
    ///
    /// Note: tags must be initialized separately via `init_tags()`.
    pub fn new_with_values(mfact: f32, nmaster: i32, showbar: bool, topbar: bool) -> Self {
        Self {
            mfact,
            nmaster,
            showbar,
            topbar,
            tag_set: [1, 1],
            clientcount: 0,
            overlaymode: OverlayMode::Top,
            current_tag: 1,
            prev_tag: 1,
            tags: Vec::new(),
            monitor_id: 0,
            ..Default::default()
        }
    }

    /// Return the `MonitorId` (index into `Globals::monitors`) of this monitor.
    ///
    /// This is kept in sync by `Globals` whenever monitors are added or removed,
    /// so it is always valid for the lifetime of the monitor.
    #[inline]
    pub fn id(&self) -> usize {
        self.monitor_id
    }

    /// Initialize tags from a template.
    pub fn init_tags(&mut self, template: &[Tag]) {
        self.tags = template.to_vec();
    }

    /// Get the currently selected tags for this monitor.
    #[inline]
    pub fn selected_tags(&self) -> u32 {
        self.tag_set[self.sel_tags as usize]
    }

    /// Set the currently selected tags for this monitor.
    #[inline]
    pub fn set_selected_tags(&mut self, mask: u32) {
        self.tag_set[self.sel_tags as usize] = mask;
    }

    /// Get the currently selected tags as a type-safe mask.
    #[inline]
    pub fn selected_tag_mask(&self) -> TagMask {
        TagMask::from_bits(self.selected_tags())
    }

    /// Iterate the monitor's client list (focus order).
    #[inline]
    pub fn iter_clients<'a>(
        &'a self,
        clients: &'a HashMap<WindowId, Client>,
    ) -> ClientListIter<'a> {
        ClientListIter::new(&self.clients, clients)
    }

    /// Iterate the monitor's stack list (stacking order).
    #[inline]
    pub fn iter_stack<'a>(&'a self, clients: &'a HashMap<WindowId, Client>) -> ClientStackIter<'a> {
        ClientStackIter::new(&self.stack, clients)
    }

    /// Check if a point is within this monitor's work area.
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        self.work_rect.contains_point(x, y)
    }

    /// Calculate the intersection area between a rectangle and this monitor's work area.
    pub fn intersect_area(&self, rect: &Rect) -> i32 {
        let x1 = rect.x.max(self.work_rect.x);
        let y1 = rect.y.max(self.work_rect.y);
        let x2 = (rect.x + rect.w).min(self.work_rect.x + self.work_rect.w);
        let y2 = (rect.y + rect.h).min(self.work_rect.y + self.work_rect.h);
        (x2 - x1).max(0) * (y2 - y1).max(0)
    }

    /// Get the center point of this monitor's work area.
    pub fn center(&self) -> (i32, i32) {
        self.work_rect.center()
    }

    /// Count the number of visible clients on this monitor.
    pub fn client_count(&self, clients: &HashMap<WindowId, Client>) -> usize {
        let selected = self.selected_tags();
        let mut count = 0;
        for (_win, c) in self.iter_clients(clients) {
            if c.is_visible_on_tags(selected) {
                count += 1;
            }
        }
        count
    }

    /// Count the number of tiled clients on this monitor.
    pub fn tiled_client_count(&self, clients: &HashMap<WindowId, Client>) -> usize {
        let selected = self.selected_tags();
        let mut count = 0;
        for (_win, c) in self.iter_clients(clients) {
            if c.is_visible_on_tags(selected) && !c.is_floating && !c.is_hidden {
                count += 1;
            }
        }
        count
    }

    /// Collect tiled clients into lightweight info snapshots for layout use.
    ///
    /// This replaces the per-layout boilerplate of filtering + snapshotting.
    pub fn collect_tiled(&self, clients: &HashMap<WindowId, Client>) -> Vec<TiledClientInfo> {
        let selected_tags = self.selected_tags();
        self.clients
            .iter()
            .filter_map(|&win| {
                let c = clients.get(&win)?;
                if !c.is_tiled(selected_tags) {
                    return None;
                }
                Some(TiledClientInfo {
                    win,
                    border_width: c.border_width(),
                    total_height: c.total_height(),
                    total_width: c.total_width(),
                })
            })
            .collect()
    }

    /// Get the currently selected client window, if any.
    pub fn selected_client(&self) -> Option<WindowId> {
        self.sel
    }

    /// Check if this monitor has a selected client.
    pub fn has_selection(&self) -> bool {
        self.sel.is_some()
    }

    /// Set the selected client for this monitor.
    pub fn set_selected(&mut self, win: Option<WindowId>) {
        self.sel = win;
    }

    /// Check if this monitor shows the bar.
    pub fn shows_bar(&self) -> bool {
        if !self.showbar {
            return false;
        }
        self.current_tag().map(|t| t.showbar).unwrap_or(true)
    }

    /// Get the current tag for this monitor.
    pub fn current_tag(&self) -> Option<&Tag> {
        if self.current_tag > 0 && self.current_tag <= self.tags.len() {
            Some(&self.tags[self.current_tag - 1])
        } else {
            None
        }
    }

    /// Get a mutable reference to the current tag.
    pub fn current_tag_mut(&mut self) -> Option<&mut Tag> {
        let idx = self.current_tag;
        if idx > 0 && idx <= self.tags.len() {
            Some(&mut self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Get the current layout symbol for this monitor.
    pub fn layout_symbol(&self) -> String {
        self.current_tag()
            .map(|t| t.layouts.symbol().to_string())
            .unwrap_or_else(|| "[]=".to_string())
    }

    /// Check if the current layout is a tiling layout.
    pub fn is_tiling_layout(&self) -> bool {
        self.current_tag()
            .map(|t| t.layouts.is_tiling())
            .unwrap_or(true)
    }

    /// Check if the current layout is a monocle layout.
    pub fn is_monocle_layout(&self) -> bool {
        self.current_tag()
            .map(|t| t.layouts.is_monocle())
            .unwrap_or(false)
    }

    /// Get the current layout kind.
    pub fn current_layout(&self) -> LayoutKind {
        self.current_tag()
            .map(|t| t.layouts.get_layout())
            .unwrap_or(LayoutKind::Tile)
    }

    /// Toggle between primary and secondary layout slots.
    pub fn toggle_layout_slot(&mut self) {
        if let Some(tag) = self.current_tag_mut() {
            tag.layouts.toggle_slot();
        }
    }

    /// Update the bar position based on monitor geometry.
    pub fn update_bar_position(&mut self, bar_height: i32) {
        let safe_bh = bar_height.max(0).min(self.monitor_rect.h.max(0));
        if self.showbar {
            self.work_rect.y = if self.topbar {
                self.monitor_rect.y + safe_bh
            } else {
                self.monitor_rect.y
            };
            self.work_rect.h = (self.monitor_rect.h - safe_bh).max(1);
            self.bar_y = if self.topbar {
                self.monitor_rect.y
            } else {
                self.monitor_rect.y + self.monitor_rect.h - safe_bh
            };
        } else {
            self.work_rect.y = self.monitor_rect.y;
            self.work_rect.h = self.monitor_rect.h.max(1);
            self.bar_y = if self.topbar {
                -safe_bh
            } else {
                self.monitor_rect.h.max(0)
            };
        }
    }

    /// Get the width of the monitor's work area.
    pub fn width(&self) -> i32 {
        self.work_rect.w
    }

    /// Get the height of the monitor's work area.
    pub fn height(&self) -> i32 {
        self.work_rect.h
    }

    /// Get the monitor's work area.
    pub fn work_area(&self) -> Rect {
        self.work_rect
    }

    /// Get the monitor's full geometry.
    pub fn monitor_area(&self) -> Rect {
        self.monitor_rect
    }

    /// Return true if the tag at `tag_index` should be hidden.
    ///
    /// A tag is hidden when `showtags != 0` and it is neither occupied nor selected.
    pub fn should_hide_tag(&self, tag_index: usize, occupied: TagMask) -> bool {
        if self.showtags == 0 {
            return false;
        }
        let tag_num = tag_index + 1;
        !occupied.contains(tag_num) && !TagMask::from_bits(self.selected_tags()).contains(tag_num)
    }

    /// Map a bar slot (0..8) to the actual tag index.
    ///
    /// Slot 8 is remapped to `current_tag - 1` when the monitor has more than 9
    /// tags active (the "overflow" slot).
    pub fn tag_index_for_slot(&self, slot: usize) -> usize {
        const MAX_BAR_SLOTS: usize = 9;
        if slot == MAX_BAR_SLOTS - 1 && self.current_tag > MAX_BAR_SLOTS {
            self.current_tag - 1
        } else {
            slot
        }
    }

    /// Compute a bitmask of tags that have at least one client on this monitor.
    ///
    /// Excludes the scratchpad tag from the result.
    pub fn occupied_tags(&self, clients: &HashMap<WindowId, Client>) -> TagMask {
        let mut occupied: u32 = 0;
        for (_win, c) in self.iter_clients(clients) {
            occupied |= c.tags;
        }
        TagMask::from_bits(occupied).without_scratchpad()
    }

    /// Compute which logical bar region the cursor's **monitor-local** x coordinate
    /// falls in.
    pub fn bar_position_at_x(
        &self,
        core: &crate::contexts::CoreCtx,
        local_x: i32,
    ) -> crate::types::BarPosition {
        use crate::bar::model::{build_fallback_hit_cache, hit_test};

        let is_selmon = core.g.selected_monitor().num == self.num;

        // Prefer the pre-built hit cache populated during rendering; fall back to
        // computing a temporary one from the same utility functions.
        let owned;
        let hit: &crate::bar::MonitorHitCache = match core.bar.monitor_hit_cache(self.id()) {
            Some(h) => h,
            None => {
                owned = build_fallback_hit_cache(self, core);
                &owned
            }
        };

        hit_test(hit, self, core, is_selmon, local_x)
    }
}

/// Find a monitor in a given direction from the current one.
pub fn find_monitor_by_direction(
    monitors: &[Monitor],
    current: usize,
    direction: MonitorDirection,
) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }
    if monitors.len() <= 1 {
        return Some(current);
    }

    if direction.is_next() {
        if current + 1 >= monitors.len() {
            Some(0)
        } else {
            Some(current + 1)
        }
    } else if current == 0 {
        Some(monitors.len() - 1)
    } else {
        Some(current - 1)
    }
}

/// Find the monitor that contains the given rectangle (by maximum intersection area).
pub fn find_monitor_by_rect(monitors: &[Monitor], rect: &Rect) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }

    let mut best_idx = 0;
    let mut max_area = 0;

    for (i, m) in monitors.iter().enumerate() {
        let area = m.intersect_area(rect);
        if area > max_area {
            max_area = area;
            best_idx = i;
        }
    }

    Some(best_idx)
}
