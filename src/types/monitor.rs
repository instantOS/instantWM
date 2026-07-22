//! Monitor/screen types.
//!
//! Types for managing multiple monitors/screens.

use std::collections::HashMap;

use crate::layouts::PresentationMode;
use crate::types::MonitorId;
use crate::types::TagMask;
use crate::types::WindowId;
use crate::types::client::{Client, ClientListIter, ClientStackIter, TiledClientInfo};
use crate::types::geometry::{Point, Rect};
use crate::types::input::{EdgeDirection, StackDirection};

mod tag_state;
mod z_order;
pub use tag_state::{PerTagState, TagNames};
pub use z_order::ClientZOrder;

/// Internal state of a monitor (screen) in the window manager.
///
/// This struct holds all runtime state for a monitor, including
/// geometry, tag state, client lists, and UI configuration.
#[derive(Debug, Clone)]
pub struct Monitor {
    /// Stable identifier of this monitor, assigned by `MonitorManager` on
    /// insertion and never changed afterwards. Read via `Monitor::id()`.
    pub(crate) monitor_id: MonitorId,
    /// Monitor index number (0-based).
    pub num: i32,
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
    /// Portion of the monitor not consumed by exclusive layer-shell surfaces
    /// (waybar, quickshell, etc.). On X11 and when no exclusive layer surfaces
    /// are mapped this is identical to `monitor_rect`. The instantWM bar and
    /// the work area are positioned inside this rectangle.
    pub available_rect: Rect,
    /// Currently selected tag set index (0 or 1).
    pub sel_tags: bool,
    /// Tag sets (two sets for switching).
    pub tag_set: [TagMask; 2],
    /// Active offset for bar display.
    pub activeoffset: u32,
    /// Title offset for bar display.
    pub titleoffset: u32,
    /// Whether to show the bar.
    pub show_bar: bool,
    /// Position of the status bar on this monitor.
    pub bar_position: EdgeDirection,
    /// Bar window handle.
    pub bar_win: WindowId,
    /// Whether to hide empty inactive tags from the bar.
    pub showtags: bool,
    /// Previously selected single tag index.
    pub prev_tag: Option<usize>,
    /// Tags owned by this monitor.
    pub tags: Vec<TagNames>,
    /// Client list (focus order).
    pub clients: Vec<WindowId>,
    /// Currently selected client.
    pub selected: Option<WindowId>,
    /// Focus history per tag mask.
    pub tag_focus_history: HashMap<TagMask, WindowId>,
    /// Last tiled focus per tag mask.
    ///
    /// This is distinct from `sel`: a floating dialog can hold keyboard focus
    /// while maximized presentation keeps the previously focused tiled client
    /// visible below it.
    pub tag_tiled_focus_history: HashMap<TagMask, WindowId>,
    /// Per-tag runtime presentation, tree, preset cursor, and bar state.
    pub per_tag: HashMap<TagMask, PerTagState>,
    /// Overview mode state.
    pub overview_state: Option<crate::overview::OverviewState>,
    /// Persistent client z-order.
    pub z_order: ClientZOrder,
    /// Monitor name (e.g., "DP-1", "HDMI-1").
    pub name: String,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            monitor_id: MonitorId::default(),
            num: 0,
            ui_scale: 1.0,
            bar_height: 0,
            horizontal_padding: 0,
            startmenu_size: 0,
            bar_clients_width: 0,
            monitor_rect: Rect::default(),
            available_rect: Rect::default(),
            sel_tags: false,
            tag_set: [TagMask::EMPTY; 2],
            activeoffset: 0,
            titleoffset: 0,
            show_bar: true,
            bar_position: EdgeDirection::Top,
            bar_win: WindowId::default(),
            showtags: false,
            prev_tag: None,
            tags: Vec::new(),
            clients: Vec::new(),
            selected: None,
            tag_focus_history: HashMap::new(),
            tag_tiled_focus_history: HashMap::new(),
            per_tag: HashMap::new(),
            overview_state: None,
            z_order: ClientZOrder::default(),
            name: String::new(),
        }
    }
}

impl Monitor {
    /// Check whether a root-space y-coordinate falls within the bar's vertical span.
    /// Does not check bar visibility — caller must do that separately.
    pub fn y_in_bar(&self, root_y: i32) -> bool {
        let h = self.bar_height.max(1);
        root_y >= self.bar_y() && root_y < self.bar_y() + h
    }

    /// Check whether a root-space y-coordinate falls in the 4-pixel guard band
    /// immediately below the bar. Does not check bar visibility.
    pub fn y_in_guard_band(&self, root_y: i32) -> bool {
        let bar_bottom = self.bar_y() + self.bar_height.max(1);
        root_y >= bar_bottom && root_y < bar_bottom + 4
    }

    /// Check whether the bar is visible on this monitor.
    pub fn bar_visible(&self, clients: &HashMap<WindowId, Client>) -> bool {
        self.shows_bar() && !self.has_real_fullscreen(clients)
    }

    /// Check whether the monitor has a client in true fullscreen mode.
    pub fn has_real_fullscreen(&self, clients: &HashMap<WindowId, Client>) -> bool {
        let selected_tags = self.selected_tags();
        self.iter_clients(clients).any(|(_, client)| {
            client.mode().is_true_fullscreen() && client.is_visible(selected_tags)
        })
    }

    /// Check whether the bar is visible on this monitor and `root_y` falls within it.
    pub fn bar_contains_y(&self, clients: &HashMap<WindowId, Client>, root_y: i32) -> bool {
        self.bar_visible(clients) && self.y_in_bar(root_y)
    }

    /// Create a new monitor with specific configuration values.
    ///
    /// Note: tags must be initialized separately via `init_tags()`.
    pub fn new_with_values(show_bar: bool, bar_position: EdgeDirection) -> Self {
        Self {
            show_bar,
            bar_position,
            per_tag: HashMap::new(),
            tag_set: [TagMask::single(1).unwrap(), TagMask::single(1).unwrap()],
            prev_tag: Some(1),
            tags: Vec::new(),
            monitor_id: MonitorId::default(),
            ..Default::default()
        }
    }

    /// Return the stable [`MonitorId`] of this monitor.
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

    /// Set the currently selected tags for this monitor, updating history.
    pub fn set_selected_tags_with_history(&mut self, new_mask: TagMask) -> bool {
        if self.selected_tags() == new_mask {
            return false;
        }

        let previous_current_tag = self.current_tag_number();
        self.sel_tags = !self.sel_tags;
        self.set_selected_tags(new_mask);
        if previous_current_tag != self.current_tag_number()
            && let Some(previous_current_tag) = previous_current_tag
        {
            self.prev_tag = Some(previous_current_tag);
        }
        true
    }

    /// Get or initialize state for the current tag mask.
    pub fn per_tag_state(&mut self) -> &mut PerTagState {
        let mask = self.selected_tags();
        let default_show_bar = self.show_bar;
        self.per_tag
            .entry(mask)
            .or_insert_with(|| PerTagState::new(default_show_bar))
    }

    /// Read the current pertag state, returning `None` if no entry exists yet.
    pub fn per_tag(&self) -> Option<&PerTagState> {
        self.per_tag.get(&self.selected_tags())
    }

    #[inline]
    pub fn current_tag_number(&self) -> Option<usize> {
        let selected = self.selected_tags();
        if selected.is_single() {
            selected.first_tag()
        } else {
            None
        }
    }

    #[inline]
    pub fn previous_tag_index(&self) -> Option<usize> {
        self.prev_tag
    }

    #[inline]
    pub fn is_all_tags_view(&self) -> bool {
        self.selected_tags() == TagMask::all(self.tags.len())
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
    pub fn contains_point(&self, point: Point) -> bool {
        self.work_rect().contains_point(point)
    }

    /// Calculate the intersection area between a rectangle and this monitor's work area.
    pub fn intersect_area(&self, rect: &Rect) -> i32 {
        self.work_rect()
            .intersection(rect)
            .map_or(0, |intersection| intersection.w * intersection.h)
    }

    /// Get the center point of this monitor's work area.
    pub fn center(&self) -> crate::types::Point {
        self.work_rect().center()
    }

    /// Translate a root-coordinate point into this monitor's work-area space.
    #[inline]
    pub fn local_work_point(&self, point: Point) -> Point {
        self.work_rect().local_point(point)
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
                    border_width: c.border_width,
                })
            })
            .collect()
    }

    /// Tiled clients in the stable order represented by the current manual
    /// tree. A newly managed client is appended defensively if reconciliation
    /// has not reached the tree yet.
    pub fn tiled_tree_order(&self, clients: &HashMap<WindowId, Client>) -> Vec<WindowId> {
        let selected = self.selected_tags();
        let mut ordered = self
            .per_tag()
            .map(|state| state.layout_tree.leaves())
            .unwrap_or_default()
            .into_iter()
            .filter(|win| {
                clients
                    .get(win)
                    .is_some_and(|client| client.is_tiled(selected))
            })
            .collect::<Vec<_>>();

        for &win in &self.clients {
            if !ordered.contains(&win)
                && clients
                    .get(&win)
                    .is_some_and(|client| client.is_tiled(selected))
            {
                ordered.push(win);
            }
        }
        ordered
    }

    /// Client-title order presented by the bar.
    ///
    /// In maximized presentation, tiled titles are tabs for the overlapping
    /// stack and therefore use the same tree order as keyboard focus cycling.
    /// Floating overlays follow that sequence in ordinary monitor client order.
    pub fn bar_client_order(&self, clients: &HashMap<WindowId, Client>) -> Vec<WindowId> {
        let selected = self.selected_tags();
        let mut ordered = if self.is_maximized_layout() {
            self.tiled_tree_order(clients)
        } else {
            Vec::new()
        };

        for &win in &self.clients {
            if !ordered.contains(&win)
                && clients
                    .get(&win)
                    .is_some_and(|client| client.shows_in_bar(selected))
            {
                ordered.push(win);
            }
        }
        ordered
    }

    /// Move a client within this monitor's focus list (stack order).
    ///
    /// Returns true if the position changed, false otherwise (e.g., if the client
    /// is floating, not found, or there are fewer than 2 tiled clients).
    pub fn move_client_in_stack(
        &mut self,
        win: WindowId,
        direction: StackDirection,
        clients: &HashMap<WindowId, Client>,
    ) -> bool {
        // Check if client exists and is tiled
        let is_floating = clients
            .get(&win)
            .map(|c| c.mode().is_floating())
            .unwrap_or(false);
        if is_floating {
            return false;
        }

        let tiled_count = self.tiled_client_count(clients);
        if tiled_count < 2 {
            return false;
        }

        if let Some(pos) = self.clients.iter().position(|&w| w == win) {
            match direction {
                StackDirection::Previous => {
                    if pos > 0 {
                        self.clients.swap(pos, pos - 1);
                        return true;
                    } else {
                        // Wrap to end: move first element to end
                        if self.clients.len() > 1 {
                            let first = self.clients.remove(0);
                            self.clients.push(first);
                            return true;
                        }
                    }
                }
                StackDirection::Next => {
                    if pos + 1 < self.clients.len() {
                        self.clients.swap(pos, pos + 1);
                        return true;
                    } else {
                        // Wrap to beginning: move last element to front
                        if self.clients.len() > 1 {
                            let last = self.clients.pop();
                            if let Some(last) = last {
                                self.clients.insert(0, last);
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Get the currently selected client window, if any.
    pub fn selected_client(&self) -> Option<WindowId> {
        self.selected
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
        self.selected.is_some()
    }

    /// Set the selected client for this monitor.
    pub fn set_selected(&mut self, win: Option<WindowId>) {
        self.selected = win;
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
                && c.mode().is_tiling()
                && c.is_visible(selected)
            {
                return Some(win);
            }
        }
        None
    }

    /// Check if this monitor shows the bar.
    pub fn shows_bar(&self) -> bool {
        self.show_bar_for_mask(self.selected_tags())
            && !self.has_external_bar_on_internal_bar_edge()
    }

    /// Returns showbar state for the given tag mask.
    pub fn show_bar_for_mask(&self, mask: TagMask) -> bool {
        self.per_tag
            .get(&mask)
            .map(|s| s.show_bar)
            .unwrap_or(self.show_bar)
    }

    /// Returns true when an exclusive layer-shell surface reserves space on the
    /// same edge where instantWM would place its own bar.
    pub fn has_external_bar_on_internal_bar_edge(&self) -> bool {
        if self.bar_position == EdgeDirection::Top {
            self.available_rect.y > self.monitor_rect.y
        } else {
            self.available_rect.y + self.available_rect.h
                < self.monitor_rect.y + self.monitor_rect.h
        }
    }

    /// Returns presentation state for the given tag mask.
    pub fn presentation_for_mask(&self, mask: TagMask) -> PresentationMode {
        self.per_tag
            .get(&mask)
            .map(|state| state.presentation)
            .unwrap_or_default()
    }

    /// Get the name data for a given tag index (1-based).
    pub fn tag_name(&self, tag_index: usize) -> Option<&TagNames> {
        tag_index.checked_sub(1).and_then(|i| self.tags.get(i))
    }

    /// Get the current tag name data for this monitor.
    pub fn current_tag(&self) -> Option<&TagNames> {
        let idx = self.current_tag_number()?;
        if idx > 0 && idx <= self.tags.len() {
            Some(&self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Get a mutable reference to the current tag name data.
    pub fn current_tag_mut(&mut self) -> Option<&mut TagNames> {
        let idx = self.current_tag_number()?;
        if idx > 0 && idx <= self.tags.len() {
            Some(&mut self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Get the current layout symbol for this monitor.
    pub fn layout_symbol(&self) -> String {
        self.presentation_for_mask(self.selected_tags())
            .symbol()
            .to_string()
    }

    /// Check if the current layout is a tiling layout.
    pub fn is_tiling_layout(&self) -> bool {
        self.presentation_for_mask(self.selected_tags()).is_tiling()
    }

    /// Check if tiled clients use maximized-stack presentation.
    pub fn is_maximized_layout(&self) -> bool {
        self.presentation_for_mask(self.selected_tags())
            .is_maximized()
    }

    /// Get the current persistent presentation mode.
    pub fn current_layout(&self) -> PresentationMode {
        self.presentation_for_mask(self.selected_tags())
    }

    /// Set the effective bar height.
    ///
    /// The work area (`work_rect`) and bar Y (`bar_y`) are derived on access
    /// from `available_rect` and `bar_height`, so storing the height is all
    /// that is needed to keep them in sync.
    pub fn set_bar_height(&mut self, bar_height: i32) {
        self.bar_height = bar_height.max(0);
    }

    /// Bar Y position (vertical position of the status bar).
    ///
    /// Derived from `available_rect`, `bar_height`, `top_bar` and `shows_bar()`
    /// so it can never fall out of sync with the monitor's real geometry.
    pub fn bar_y(&self) -> i32 {
        let safe_bh = self.bar_height.min(self.available_rect.h.max(0));
        if self.shows_bar() {
            if self.bar_position == EdgeDirection::Top {
                self.available_rect.y
            } else {
                self.available_rect.y + self.available_rect.h - safe_bh
            }
        } else if self.bar_position == EdgeDirection::Top {
            self.available_rect.y - safe_bh
        } else {
            self.available_rect.y + self.available_rect.h
        }
    }

    /// Work area geometry (excluding bar and exclusive layer surfaces).
    ///
    /// Derived from `available_rect`, `bar_height`, `top_bar` and `shows_bar()`
    /// so it can never fall out of sync with the monitor's real geometry.
    pub fn work_rect(&self) -> Rect {
        self.rect_excluding_internal_bar(self.shows_bar())
    }

    /// Area not occupied by exclusive layer surfaces or the currently visible
    /// built-in bar.
    ///
    /// Unlike [`Self::work_rect`], this accounts for a true-fullscreen client
    /// temporarily hiding the built-in bar. It is intended for WM-owned UI
    /// such as edge scratchpads that must avoid every visible bar.
    pub fn visible_content_rect(&self, clients: &HashMap<WindowId, Client>) -> Rect {
        self.rect_excluding_internal_bar(self.bar_visible(clients))
    }

    fn rect_excluding_internal_bar(&self, bar_visible: bool) -> Rect {
        let safe_bh = self.bar_height.min(self.available_rect.h.max(0));
        let mut rect = Rect::new(self.available_rect.x, 0, self.available_rect.w.max(1), 0);
        if bar_visible {
            rect.y = if self.bar_position == EdgeDirection::Top {
                self.available_rect.y + safe_bh
            } else {
                self.available_rect.y
            };
            rect.h = (self.available_rect.h - safe_bh).max(1);
        } else {
            rect.y = self.available_rect.y;
            rect.h = self.available_rect.h.max(1);
        }
        rect
    }

    /// The currently maximized client on this monitor, if any.
    ///
    /// Derived by scanning the monitor's clients for one in maximized mode, so
    /// it can never disagree with the actual client modes.
    pub fn maximized_client(&self, clients: &HashMap<WindowId, Client>) -> Option<WindowId> {
        self.clients.iter().find_map(|&win| {
            clients
                .get(&win)
                .filter(|c| c.mode().is_maximized())
                .map(|_| win)
        })
    }

    /// Set the rectangle that is not consumed by exclusive layer-shell
    /// surfaces. The work area and bar position are derived automatically from
    /// this rectangle whenever they are accessed.
    pub fn set_available_rect(&mut self, rect: Rect) {
        self.available_rect = rect;
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
        // Reset the available rect to the full output. The Wayland backend
        // re-applies layer-shell exclusive zones on top of this. The work area
        // and bar position are derived from this rectangle on access.
        self.available_rect = rect;
        self.name = name;
        self.set_ui_metrics(scale, bar_height, horizontal_padding, startmenu_size);
        self.set_bar_height(bar_height);
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
        self.work_rect().w
    }

    /// Get the height of the monitor's work area.
    pub fn height(&self) -> i32 {
        self.work_rect().h
    }

    /// Get the monitor's work area.
    pub fn work_area(&self) -> Rect {
        self.work_rect()
    }

    /// Get the monitor's full geometry.
    pub fn monitor_area(&self) -> Rect {
        self.monitor_rect
    }

    /// Return true if the tag at `tag_index` should be hidden.
    ///
    /// A tag is hidden when compact tag display is enabled and it is neither occupied nor selected.
    pub fn should_hide_tag(&self, tag_index: usize, occupied: TagMask) -> bool {
        if !self.showtags {
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
            && let Some(current_tag) = self.current_tag_number()
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

        let is_selmon = core.model().expect_selected_monitor().num == self.num;

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

        hit_test(hit, self, core.config().systray.show, is_selmon, local_x)
    }
}

#[cfg(test)]
mod tests;
