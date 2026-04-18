//! Monitor/screen types.
//!
//! Types for managing multiple monitors/screens.

use std::collections::HashMap;

use crate::layouts::LayoutKind;
use crate::types::MonitorId;
use crate::types::TagLayouts;
use crate::types::TagMask;
use crate::types::WindowId;
use crate::types::client::{Client, ClientListIter, ClientStackIter, TiledClientInfo};
use crate::types::geometry::Rect;
use crate::types::input::Gesture;
use crate::types::tag_types::MonitorDirection;

/// Persistent per-monitor client z-order.
///
/// The stored order is bottom-to-top. Layout policy may project this into a
/// different backend order temporarily (for example, monocle promotes the
/// focused client visually), but focus changes alone should not mutate this
/// persistent order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientZOrder {
    bottom_to_top: Vec<WindowId>,
}

impl ClientZOrder {
    pub fn as_slice(&self) -> &[WindowId] {
        &self.bottom_to_top
    }

    pub fn attach_top(&mut self, win: WindowId) {
        self.remove(win);
        self.bottom_to_top.push(win);
    }

    pub fn attach_bottom(&mut self, win: WindowId) {
        self.remove(win);
        self.bottom_to_top.insert(0, win);
    }

    pub fn remove(&mut self, win: WindowId) -> bool {
        let old_len = self.bottom_to_top.len();
        self.bottom_to_top.retain(|&w| w != win);
        self.bottom_to_top.len() != old_len
    }

    pub fn raise(&mut self, win: WindowId) -> bool {
        if !self.remove(win) {
            return false;
        }
        self.bottom_to_top.push(win);
        true
    }

    pub fn lower(&mut self, win: WindowId) -> bool {
        if !self.remove(win) {
            return false;
        }
        self.bottom_to_top.insert(0, win);
        true
    }

    pub fn iter_bottom_to_top(&self) -> impl DoubleEndedIterator<Item = WindowId> + '_ {
        self.bottom_to_top.iter().copied()
    }

    pub fn iter_top_to_bottom(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.bottom_to_top.iter().rev().copied()
    }
}

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
    pub(crate) monitor_id: MonitorId,
    /// Master factor for tiling layouts (0.0 to 1.0).
    pub mfact: f32,
    /// Number of clients in the master area for tiling layouts.
    pub nmaster: i32,
    /// Monitor index number (0-based).
    pub num: i32,
    /// Bar Y position (vertical position of the status bar).
    pub bar_y: i32,
    /// Per-monitor UI scale, currently used by the Wayland bar.
    pub ui_scale: f64,
    /// Effective bar height for this monitor.
    pub bar_height: i32,
    /// Effective horizontal padding for this monitor's bar.
    pub horizontal_padding: i32,
    /// Effective start menu width for this monitor's bar.
    pub startmenu_size: i32,
    /// Width reserved for client title display in the bar.
    pub bar_clients_width: i32,
    /// Full monitor geometry (including bar).
    pub monitor_rect: Rect,
    /// Work area geometry (excluding bar).
    pub work_rect: Rect,
    /// Currently selected tag set index.
    pub sel_tags: u32,
    /// Tag sets (two sets for switching).
    pub tag_set: [TagMask; 2],
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
    /// Current gesture state.
    pub gesture: Gesture,
    /// Bar window handle.
    pub bar_win: WindowId,
    /// Which tags to show.
    pub showtags: u32,
    /// Current tag index when the selected view is a single tag.
    pub current_tag: Option<usize>,
    /// Previously selected single tag index.
    pub prev_tag: Option<usize>,
    /// Tags owned by this monitor.
    pub tags: Vec<TagNames>,
    /// Client list (focus order).
    pub clients: Vec<WindowId>,
    /// Currently selected client.
    pub sel: Option<WindowId>,
    /// Focus history per tag mask.
    pub tag_focus_history: HashMap<u32, WindowId>,
    /// Last tiled focus per tag mask.
    ///
    /// This is distinct from `sel`: a floating dialog can hold keyboard focus
    /// while monocle still needs to keep the previously focused tiled client
    /// visible below it.
    pub tag_tiled_focus_history: HashMap<u32, WindowId>,
    /// Per-tag runtime state (master factor, nmaster, layouts, etc.).
    pub pertag: HashMap<u32, PertagState>,
    /// Persistent client z-order.
    pub z_order: ClientZOrder,
    /// Currently maximized client.
    pub maximized: Option<WindowId>,
    /// Monitor name (e.g., "DP-1", "HDMI-1").
    pub name: String,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            monitor_id: MonitorId(0),
            mfact: 0.55,
            nmaster: 1,
            num: 0,
            bar_y: 0,
            ui_scale: 1.0,
            bar_height: 0,
            horizontal_padding: 0,
            startmenu_size: 0,
            bar_clients_width: 0,
            monitor_rect: Rect::default(),
            work_rect: Rect::default(),
            sel_tags: 0,
            tag_set: [TagMask::EMPTY; 2],
            activeoffset: 0,
            titleoffset: 0,
            clientcount: 0,
            showbar: true,
            topbar: true,
            gesture: Gesture::default(),
            bar_win: WindowId::default(),
            showtags: 0,
            current_tag: None,
            prev_tag: None,
            tags: Vec::new(),
            clients: Vec::new(),
            sel: None,
            tag_focus_history: HashMap::new(),
            tag_tiled_focus_history: HashMap::new(),
            pertag: HashMap::new(),
            z_order: ClientZOrder::default(),
            maximized: None,
            name: String::new(),
        }
    }
}

impl Monitor {
    /// Create a new monitor with specific configuration values.
    ///
    /// Note: tags must be initialized separately via `init_tags()`.
    pub fn new_with_values(showbar: bool, topbar: bool) -> Self {
        Self {
            showbar,
            topbar,
            pertag: HashMap::new(),
            tag_set: [TagMask::single(1).unwrap(), TagMask::single(1).unwrap()],
            clientcount: 0,
            current_tag: Some(1),
            prev_tag: Some(1),
            tags: Vec::new(),
            monitor_id: MonitorId(0),
            ..Default::default()
        }
    }

    /// Return the `MonitorId` (index into `Globals::monitors`) of this monitor.
    ///
    /// This is kept in sync by `Globals` whenever monitors are added or removed,
    /// so it is always valid for the lifetime of the monitor.
    #[inline]
    pub fn id(&self) -> MonitorId {
        self.monitor_id
    }

    /// Initialize tags from a template.
    pub fn init_tags(&mut self, template: &[TagNames]) {
        self.tags = template.to_vec();
    }

    /// Get the currently selected tags for this monitor.
    #[inline]
    pub fn selected_tags(&self) -> TagMask {
        self.tag_set[self.sel_tags as usize]
    }

    /// Set the currently selected tags for this monitor.
    #[inline]
    pub fn set_selected_tags(&mut self, mask: TagMask) {
        self.tag_set[self.sel_tags as usize] = mask;
    }

    /// Get the currently selected tags for this monitor as raw bits.
    #[inline]
    pub fn selected_tags_bits(&self) -> u32 {
        self.tag_set[self.sel_tags as usize].bits()
    }

    /// Set the currently selected tags for this monitor from raw bits.
    #[inline]
    pub fn set_selected_tags_bits(&mut self, mask: u32) {
        self.tag_set[self.sel_tags as usize] = TagMask::from_bits(mask);
    }

    /// Get or initialize state for the current tag mask.
    pub fn pertag_state(&mut self) -> &mut PertagState {
        let mask = self.selected_tags().bits();
        let default_showbar = self.showbar;
        self.pertag
            .entry(mask)
            .or_insert_with(|| PertagState::new(default_showbar))
    }

    /// Get the currently selected tags as a type-safe mask.
    #[inline]
    pub fn selected_tag_mask(&self) -> TagMask {
        self.selected_tags()
    }

    #[inline]
    pub fn current_tag_index(&self) -> Option<usize> {
        self.current_tag
    }

    #[inline]
    pub fn previous_tag_index(&self) -> Option<usize> {
        self.prev_tag
    }

    #[inline]
    pub fn is_all_tags_view(&self) -> bool {
        self.current_tag.is_none()
    }

    /// Iterate the monitor's client list (focus order).
    #[inline]
    pub fn iter_clients<'a>(
        &'a self,
        clients: &'a HashMap<WindowId, Client>,
    ) -> ClientListIter<'a> {
        ClientListIter::new(&self.clients, clients)
    }

    /// Iterate the monitor's persistent z-order.
    #[inline]
    pub fn iter_stack<'a>(&'a self, clients: &'a HashMap<WindowId, Client>) -> ClientStackIter<'a> {
        ClientStackIter::new(self.z_order.as_slice(), clients)
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
            if c.is_visible(selected) {
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
            if c.is_tiled(selected) {
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

    /// Walk the persistent z-order and return the topmost visible, non-hidden
    /// client on the currently selected tags.
    ///
    /// `z_order` is bottom-to-top. Focus recovery walks it from the top so
    /// closing an overlapping window selects the window immediately below it.
    pub fn first_visible_client(&self, clients: &HashMap<WindowId, Client>) -> Option<WindowId> {
        let tags = self.selected_tags();
        self.z_order
            .iter_top_to_bottom()
            .find_map(|w| clients.get(&w).filter(|c| c.is_visible(tags)).map(|_| w))
    }

    /// Check if this monitor has a selected client.
    pub fn has_selection(&self) -> bool {
        self.sel.is_some()
    }

    /// Set the selected client for this monitor.
    pub fn set_selected(&mut self, win: Option<WindowId>) {
        self.sel = win;
    }

    /// Find the next tiled client on this monitor starting after `start_win`.
    pub fn next_tiled(
        &self,
        clients: &HashMap<WindowId, Client>,
        start_win: Option<WindowId>,
    ) -> Option<WindowId> {
        let selected = self.selected_tags();

        let start_idx = if let Some(win) = start_win {
            self.clients.iter().position(|&w| w == win)
        } else {
            None
        };

        let iter_start = start_idx.map(|i| i + 1).unwrap_or(0);

        for &win in self.clients.iter().skip(iter_start) {
            if let Some(c) = clients.get(&win)
                && c.mode.is_tiling()
                && c.is_visible(selected)
            {
                return Some(win);
            }
        }
        None
    }

    /// Check if this monitor shows the bar.
    pub fn shows_bar(&self) -> bool {
        self.showbar_for_mask(self.selected_tags())
    }

    /// Returns showbar state for the given tag mask.
    pub fn showbar_for_mask(&self, mask: TagMask) -> bool {
        self.pertag
            .get(&mask.bits())
            .map(|s| s.showbar)
            .unwrap_or(self.showbar)
    }

    /// Returns layout state for the given tag mask (immutable lookup).
    pub fn layouts_for_mask(&self, mask: TagMask) -> TagLayouts {
        self.pertag
            .get(&mask.bits())
            .map(|s| s.layouts)
            .unwrap_or_default()
    }

    /// Get the name data for a given tag index (1-based).
    pub fn tag_name(&self, tag_index: usize) -> Option<&TagNames> {
        tag_index.checked_sub(1).and_then(|i| self.tags.get(i))
    }

    /// Get the current tag name data for this monitor.
    pub fn current_tag(&self) -> Option<&TagNames> {
        let idx = self.current_tag?;
        if idx > 0 && idx <= self.tags.len() {
            Some(&self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Get a mutable reference to the current tag name data.
    pub fn current_tag_mut(&mut self) -> Option<&mut TagNames> {
        let idx = self.current_tag?;
        if idx > 0 && idx <= self.tags.len() {
            Some(&mut self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Get the current layout symbol for this monitor.
    pub fn layout_symbol(&self) -> String {
        self.layouts_for_mask(self.selected_tags())
            .symbol()
            .to_string()
    }

    /// Check if the current layout is a tiling layout.
    pub fn is_tiling_layout(&self) -> bool {
        self.layouts_for_mask(self.selected_tags()).is_tiling()
    }

    /// Check if the current layout is a monocle layout.
    pub fn is_monocle_layout(&self) -> bool {
        self.layouts_for_mask(self.selected_tags()).is_monocle()
    }

    /// Get the current layout kind.
    pub fn current_layout(&self) -> LayoutKind {
        self.layouts_for_mask(self.selected_tags()).get_layout()
    }

    /// Toggle between primary and secondary layout slots.
    pub fn toggle_layout_slot(&mut self) {
        self.pertag_state().layouts.toggle_slot();
    }

    /// Update the bar position based on monitor geometry.
    pub fn update_bar_position(&mut self, bar_height: i32) {
        self.bar_height = bar_height.max(0);
        let safe_bh = self.bar_height.min(self.monitor_rect.h.max(0));
        if self.pertag_state().showbar {
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

    /// Apply output-derived geometry and UI metrics from the compositor / RandR.
    ///
    /// Does not change workspace state (tags, client lists, focus, `pertag`, etc.).
    pub fn apply_output_layout(
        &mut self,
        index: usize,
        name: String,
        rect: Rect,
        scale: f64,
        bar_height: i32,
        horizontal_padding: i32,
        startmenu_size: i32,
    ) {
        self.num = index as i32;
        self.monitor_rect = rect;
        self.work_rect = rect;
        self.name = name;
        self.set_ui_metrics(scale, bar_height, horizontal_padding, startmenu_size);
        self.update_bar_position(bar_height);
    }

    /// Set effective UI metrics for this monitor.
    pub fn set_ui_metrics(
        &mut self,
        ui_scale: f64,
        bar_height: i32,
        horizontal_padding: i32,
        startmenu_size: i32,
    ) {
        self.ui_scale = if ui_scale.is_finite() && ui_scale > 0.0 {
            ui_scale
        } else {
            1.0
        };
        self.bar_height = bar_height.max(0);
        self.horizontal_padding = horizontal_padding.max(0);
        self.startmenu_size = startmenu_size.max(0);
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
        !occupied.contains(tag_num) && !self.selected_tags().contains(tag_num)
    }

    /// Map a bar slot (0..8) to the actual tag index.
    ///
    /// Slot 8 is remapped to `current_tag - 1` when the monitor has more than 9
    /// tags active (the "overflow" slot).
    pub fn tag_index_for_slot(&self, slot: usize) -> usize {
        const MAX_BAR_SLOTS: usize = 9;
        if slot == MAX_BAR_SLOTS - 1
            && let Some(current_tag) = self.current_tag
            && current_tag > MAX_BAR_SLOTS
        {
            current_tag - 1
        } else {
            slot
        }
    }

    /// Compute a bitmask of tags that have at least one client on this monitor.
    ///
    /// Excludes the scratchpad tag from the result.
    pub fn occupied_tags(&self, clients: &HashMap<WindowId, Client>) -> TagMask {
        let mut occupied = TagMask::EMPTY;
        for (_win, c) in self.iter_clients(clients) {
            occupied = occupied | c.tags;
        }
        occupied.without_scratchpad()
    }

    /// Compute which logical bar region the cursor's **monitor-local** x coordinate
    /// falls in.
    pub fn bar_position_at_x(
        &self,
        core: &crate::contexts::CoreCtx,
        local_x: i32,
    ) -> crate::types::BarPosition {
        use crate::bar::model::{build_fallback_hit_cache, hit_test};

        let is_selmon = core.globals().selected_monitor().num == self.num;

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
    current: MonitorId,
    direction: MonitorDirection,
) -> Option<MonitorId> {
    if monitors.is_empty() {
        return None;
    }
    if monitors.len() <= 1 {
        return Some(current);
    }

    let current = current.index();

    if direction.is_next() {
        if current + 1 >= monitors.len() {
            Some(MonitorId(0))
        } else {
            Some(MonitorId(current + 1))
        }
    } else if current == 0 {
        Some(MonitorId(monitors.len() - 1))
    } else {
        Some(MonitorId(current - 1))
    }
}

/// Find the monitor that contains the given rectangle (by maximum intersection area).
pub fn find_monitor_by_rect(monitors: &[Monitor], rect: &Rect) -> Option<MonitorId> {
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

    Some(MonitorId(best_idx))
}

/// Runtime state restored when a tag mask is revisited.
/// Initialized with hardcoded defaults on first visit.
#[derive(Debug, Clone)]
pub struct PertagState {
    pub nmaster: i32,
    pub mfact: f32,
    pub showbar: bool,
    pub layouts: TagLayouts,
}

impl Default for PertagState {
    fn default() -> Self {
        Self::new(true)
    }
}

impl PertagState {
    pub fn new(showbar: bool) -> Self {
        Self {
            nmaster: 1,
            mfact: 0.55,
            showbar,
            layouts: TagLayouts::default(),
        }
    }
}

/// Per-tag name data. No runtime layout state.
#[derive(Debug, Clone, Default)]
pub struct TagNames {
    pub name: String,
    pub alt_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_visible_client_prefers_topmost_visible_stack_entry() {
        let mut monitor = Monitor::default();
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        monitor.z_order.attach_top(WindowId(1));
        monitor.z_order.attach_top(WindowId(2));
        monitor.z_order.attach_top(WindowId(3));

        let mut clients = HashMap::new();
        for id in [WindowId(1), WindowId(2), WindowId(3)] {
            let mut client = Client {
                win: id,
                ..Client::default()
            };
            client.set_tag_mask(TagMask::single(1).unwrap());
            clients.insert(id, client);
        }

        assert_eq!(monitor.first_visible_client(&clients), Some(WindowId(3)));
    }

    #[test]
    fn client_z_order_raise_moves_existing_client_to_top() {
        let mut z_order = ClientZOrder::default();
        z_order.attach_top(WindowId(1));
        z_order.attach_top(WindowId(2));
        z_order.attach_top(WindowId(3));

        assert!(z_order.raise(WindowId(2)));
        assert_eq!(
            z_order.iter_bottom_to_top().collect::<Vec<_>>(),
            vec![WindowId(1), WindowId(3), WindowId(2)]
        );
    }

    #[test]
    fn client_z_order_raise_ignores_unknown_client() {
        let mut z_order = ClientZOrder::default();
        z_order.attach_top(WindowId(1));
        z_order.attach_top(WindowId(2));
        z_order.attach_top(WindowId(3));

        assert!(!z_order.raise(WindowId(4)));
        assert_eq!(
            z_order.iter_bottom_to_top().collect::<Vec<_>>(),
            vec![WindowId(1), WindowId(2), WindowId(3)]
        );
    }

    #[test]
    fn pertag_state_defaults_match_normal_tiling_defaults() {
        let state = PertagState::default();

        assert_eq!(state.nmaster, 1);
        assert_eq!(state.mfact, 0.55);
    }

    #[test]
    fn tiled_client_count_matches_collected_tiled_clients() {
        let mut monitor = Monitor::default();
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        monitor.clients = vec![WindowId(1), WindowId(2), WindowId(3), WindowId(4)];

        let mut normal = Client {
            win: WindowId(1),
            ..Client::default()
        };
        normal.set_tag_mask(TagMask::single(1).unwrap());

        let mut fullscreen = Client {
            win: WindowId(2),
            ..Client::default()
        };
        fullscreen.mode = fullscreen.mode.as_fullscreen();
        fullscreen.set_tag_mask(TagMask::single(1).unwrap());

        let mut floating = Client {
            win: WindowId(3),
            mode: crate::types::ClientMode::Floating,
            ..Client::default()
        };
        floating.set_tag_mask(TagMask::single(1).unwrap());

        let mut hidden = Client {
            win: WindowId(4),
            is_hidden: true,
            ..Client::default()
        };
        hidden.set_tag_mask(TagMask::single(1).unwrap());

        let clients = HashMap::from([
            (WindowId(1), normal),
            (WindowId(2), fullscreen),
            (WindowId(3), floating),
            (WindowId(4), hidden),
        ]);

        assert_eq!(monitor.tiled_client_count(&clients), 1);
        assert_eq!(monitor.collect_tiled(&clients).len(), 1);
    }
}
