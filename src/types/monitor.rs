//! Monitor/screen types.
//!
//! Types for managing multiple monitors/screens.
//!
//! A [`Monitor`] owns the clients assigned to it. Ownership, rather than a
//! `monitor_id` field on the client, is what makes the relationship trustworthy:
//! there is exactly one place a client can live, so no assignment can name the
//! wrong monitor or go stale. Two orderings over those owned clients are kept
//! alongside them — [`Monitor::stack`] (focus order) and
//! [`Monitor::z_order`] (stacking order) — and a debug assertion keeps both
//! consistent with the owned set.

use std::collections::{HashMap, HashSet};

use crate::layouts::LayoutCommand;
use crate::layouts::PresentationMode;
use crate::types::MonitorId;
use crate::types::Tag;
use crate::types::TagMask;
use crate::types::WindowId;
use crate::types::client::{Client, OrderedClients, TiledClientInfo};
use crate::types::geometry::{Point, Rect};
use crate::types::input::StackDirection;

mod tag_state;
mod z_order;
pub use tag_state::PerTagState;
pub use z_order::ClientZOrder;

/// Bar UI metrics for one output, already scaled for that output's UI scale.
///
/// These three values always travel, compare, and apply together — they are
/// computed together from the unscaled [`DerivedState`](crate::core_state::DerivedState)
/// base, checked together for changes, and written together onto a
/// [`Monitor`]. Carrying them as one named type means the compiler rejects
/// transposing `horizontal_padding` and `startmenu_size` (both plain `i32`
/// when passed positionally), and readers see what a value is at every
/// boundary instead of decoding `let (bh, hp, sm) = ...`.
///
/// This is a *transport* type. A monitor still stores the three values as
/// individual fields; read them back via [`Monitor::ui_metrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorUiMetrics {
    /// Effective bar height for the output, scaled for its UI scale.
    pub bar_height: i32,
    /// Effective horizontal padding for the output's bar, scaled.
    pub horizontal_padding: i32,
    /// Effective start menu width for the output's bar, scaled.
    pub startmenu_size: i32,
}

/// Client state carried away from a disconnected output. The two orders are
/// independent: focus order drives cycling, while z-order drives overlap.
#[derive(Debug)]
pub(crate) struct OrphanedMonitorClients {
    pub clients_in_reverse_focus_order: Vec<(Client, bool)>,
    pub z_order: Vec<WindowId>,
}

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
    /// Height of the bottom gesture strip on this monitor (0 = none).
    pub bottom_bar_height: i32,
    /// Effective horizontal padding for this monitor's bar.
    pub horizontal_padding: i32,
    /// Effective start menu width for this monitor's bar.
    pub startmenu_size: i32,
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
    /// Configured bar visibility for this output, seeded from
    /// `bar.show` by [`TagBarPolicy::apply_to`](crate::bar::policy::TagBarPolicy).
    ///
    /// The fallback for tag masks without a session override in
    /// [`Self::per_tag`]; it is not itself toggled at runtime.
    pub bar_default_show: bool,
    /// Whether the bottom bar is shown (single global session setting).
    pub show_bottom_bar: bool,
    /// Bar window handle.
    pub bar_win: WindowId,
    /// X11 window backing the bottom bar strip (Wayland: default id).
    pub bottom_bar_win: WindowId,
    /// X11 child window for the bottom-bar indicator rectangle (Wayland: default id).
    pub bottom_bar_indicator_win: WindowId,
    /// Previously selected single tag index.
    pub prev_tag: Option<usize>,
    /// Tags owned by this monitor.
    pub tags: Vec<Tag>,
    /// Clients this monitor owns. Private because ownership changes must also
    /// update the focus stack and z-order; read it through
    /// [`Self::clients`] and change it through [`Self::adopt_client`] or
    /// [`Self::take_client`].
    clients: HashMap<WindowId, Client>,
    /// Client list (focus order). Private for the same reason as `clients`;
    /// read it through [`Self::focus_order`] and change it through
    /// [`Self::set_focus_order`], [`Self::move_client_in_stack`], or
    /// [`Self::swap_clients_in_stack`].
    stack: Vec<WindowId>,
    /// Currently selected client.
    pub selected: Option<WindowId>,
    /// Most-recently-used focus order per tag mask, oldest to newest.
    ///
    /// Keeping the complete order lets a run of short-lived windows unwind
    /// predictably when closed. Tiled focus is derived from this history rather
    /// than maintained as a second cache that can lose its predecessor.
    pub(crate) focus_history: HashMap<TagMask, Vec<WindowId>>,
    /// Per-tag runtime presentation, active layout slot, and bar state.
    pub per_tag: HashMap<TagMask, PerTagState>,
    /// Overview mode state.
    pub overview_state: Option<crate::overview::OverviewState>,
    /// Persistent client z-order. Private because a reorder must not be able
    /// to drop a window from the owned set; read it through [`Self::z_order`]
    /// and change it through [`Self::raise_client`] or the adopt/take pair.
    z_order: ClientZOrder,
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
            bottom_bar_height: 0,
            horizontal_padding: 0,
            startmenu_size: 0,
            monitor_rect: Rect::default(),
            available_rect: Rect::default(),
            sel_tags: false,
            tag_set: [TagMask::EMPTY; 2],
            bar_default_show: true,
            show_bottom_bar: false,
            bar_win: WindowId::default(),
            bottom_bar_win: WindowId::default(),
            bottom_bar_indicator_win: WindowId::default(),
            prev_tag: None,
            tags: Vec::new(),
            clients: HashMap::new(),
            stack: Vec::new(),
            selected: None,
            focus_history: HashMap::new(),
            per_tag: HashMap::new(),
            overview_state: None,
            z_order: ClientZOrder::default(),
            name: String::new(),
        }
    }
}

impl Monitor {
    /// Record `win` as the most recently focused client on `tags`.
    pub(crate) fn record_focus(&mut self, tags: TagMask, win: WindowId) {
        let history = self.focus_history.entry(tags).or_default();
        history.retain(|candidate| *candidate != win);
        history.push(win);
    }

    /// Return the most recently focused client on `tags` accepted by `eligible`.
    pub(crate) fn most_recent_focus(
        &self,
        tags: TagMask,
        mut eligible: impl FnMut(WindowId) -> bool,
    ) -> Option<WindowId> {
        self.focus_history
            .get(&tags)?
            .iter()
            .rev()
            .copied()
            .find(|win| eligible(*win))
    }

    /// Forget every focus-history occurrence of a removed or transferred client.
    pub(crate) fn forget_focus(&mut self, win: WindowId) {
        self.focus_history.retain(|_, history| {
            history.retain(|candidate| *candidate != win);
            !history.is_empty()
        });
    }

    pub(crate) fn focus_history_windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.focus_history
            .values()
            .flat_map(|history| history.iter().copied())
    }

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

    /// Bottom bar Y position (root space), bottom-aligned inside
    /// `available_rect`. When hidden, the strip sits just below the monitor so
    /// the X11 window can stay mapped without overlapping any content.
    pub fn bottom_bar_y(&self) -> i32 {
        let safe_bh = self.bottom_bar_height.min(self.available_rect.h.max(0));
        if self.shows_bottom_bar() {
            self.available_rect.bottom() - safe_bh
        } else {
            self.available_rect.bottom()
        }
    }

    /// Check whether a root-space y-coordinate falls within the bottom bar's
    /// vertical span. Does not check bar visibility — caller must do that.
    pub fn y_in_bottom_bar(&self, root_y: i32) -> bool {
        let h = self
            .bottom_bar_height
            .min(self.available_rect.h.max(0))
            .max(1);
        root_y >= self.bottom_bar_y() && root_y < self.bottom_bar_y() + h
    }

    /// Centered "grab handle" rectangle inside the bottom bar, local to the
    /// strip's top-left corner. Wide and thin so it reads as an interactive
    /// indicator without dominating the bar.
    pub fn bottom_bar_indicator_rect(&self) -> Rect {
        let bar_w = self.work_rect().w;
        let bar_h = self.bottom_bar_height;
        let w = (bar_w / 6).max(40).min(bar_w);
        let h = (bar_h / 4).max(3).min(bar_h);
        let x = (bar_w - w) / 2;
        let y = (bar_h - h) / 2;
        Rect::new(x, y, w, h)
    }

    /// Check whether the bar is visible on this monitor.
    pub fn bar_visible(&self) -> bool {
        self.shows_bar() && !self.has_real_fullscreen()
    }

    /// Whether this monitor draws the bottom bar. The state is global and
    /// deliberately not per-tag: toggling it applies to every tag.
    pub fn shows_bottom_bar(&self) -> bool {
        self.show_bottom_bar
    }

    /// Check whether the bottom bar is visible on this monitor.
    pub fn bottom_bar_visible(&self) -> bool {
        self.shows_bottom_bar() && !self.has_real_fullscreen()
    }

    /// Check whether the bottom bar is visible on this monitor and `root_y`
    /// falls within it.
    pub fn bottom_bar_contains_y(&self, root_y: i32) -> bool {
        self.bottom_bar_visible() && self.y_in_bottom_bar(root_y)
    }

    /// Check whether the monitor has a client in true fullscreen mode.
    pub fn has_real_fullscreen(&self) -> bool {
        let selected_tags = self.visible_tags();
        self.iter_clients().any(|(_, client)| {
            client.mode().is_true_fullscreen() && client.is_visible(selected_tags)
        })
    }

    /// Check whether the bar is visible on this monitor and `root_y` falls within it.
    pub fn bar_contains_y(&self, root_y: i32) -> bool {
        self.bar_visible() && self.y_in_bar(root_y)
    }

    /// Create a new monitor with its initial tag selection.
    ///
    /// Note: tags must be initialized separately via `init_tags()`, and the
    /// configured bar visibility is seeded by
    /// [`TagBarPolicy::apply_to`](crate::bar::policy::TagBarPolicy).
    pub fn new_with_values() -> Self {
        Self {
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
    pub fn init_tags(&mut self, template: &[Tag]) {
        self.tags = template.to_vec();
    }

    /// Discard stored views that reference tags removed by a config reload.
    /// The active view is checked before this method is called.
    pub(crate) fn retain_tag_count(&mut self, count: usize) {
        let allowed = TagMask::all(count);
        let active = self.selected_tags();
        for view in &mut self.tag_set {
            *view = *view & allowed;
            if view.is_empty() {
                *view = active;
            }
        }
        self.prev_tag = self.prev_tag.filter(|tag| *tag <= count);
        self.per_tag.retain(|mask, _| (*mask & !allowed).is_empty());
        self.focus_history
            .retain(|mask, _| (*mask & !allowed).is_empty());
        if let Some(overview) = &mut self.overview_state {
            overview.retain_tags(allowed);
        }
    }

    /// Get the currently selected tags for this monitor.
    #[inline]
    pub fn selected_tags(&self) -> TagMask {
        self.tag_set[self.sel_tags as usize]
    }

    /// Tags currently projected on screen.
    ///
    /// Overview is a presentation of every tag, not a workspace selection.
    /// Keeping this separate from [`Self::selected_tags`] prevents temporary
    /// overview state from leaking into launch placement, tag history, IPC,
    /// and per-tag layout storage.
    #[inline]
    pub fn visible_tags(&self) -> TagMask {
        self.overview_state.as_ref().map_or_else(
            || self.selected_tags(),
            crate::overview::OverviewState::projected_tags,
        )
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
        self.per_tag.entry(mask).or_default()
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
    pub fn iter_clients(&self) -> OrderedClients<'_> {
        OrderedClients::new(&self.stack, &self.clients)
    }

    /// Iterate the monitor's persistent z-order.
    #[inline]
    pub fn iter_stack(&self) -> OrderedClients<'_> {
        OrderedClients::new(self.z_order.as_slice(), &self.clients)
    }

    /// Return a client this monitor owns.
    #[inline]
    pub fn client(&self, win: WindowId) -> Option<&Client> {
        self.clients.get(&win)
    }

    /// The clients this monitor owns, keyed by window.
    ///
    /// Read-only by design: the owned set, focus stack, and z-order are one
    /// graph, so inserting or removing here directly would leave the other two
    /// stale. Use [`Self::adopt_client`] / [`Self::take_client`] to change
    /// ownership, or [`Self::client_mut`] to edit a client in place.
    #[inline]
    pub fn clients(&self) -> &HashMap<WindowId, Client> {
        &self.clients
    }

    /// This monitor's focus order, oldest first.
    ///
    /// Read-only for the same reason as [`Self::clients`]. Runtime reordering
    /// goes through [`Self::move_client_in_stack`] or
    /// [`Self::swap_clients_in_stack`].
    #[inline]
    pub fn focus_order(&self) -> &[WindowId] {
        &self.stack
    }

    /// This monitor's persistent stacking order, bottom to top.
    ///
    /// Read-only for the same reason as [`Self::clients`]. Reordering goes
    /// through [`Self::raise_client`] or the adopt/take pair.
    #[inline]
    pub fn z_order(&self) -> &ClientZOrder {
        &self.z_order
    }

    /// Return a client this monitor owns, mutably.
    #[inline]
    pub fn client_mut(&mut self, win: WindowId) -> Option<&mut Client> {
        self.clients.get_mut(&win)
    }

    /// Whether this monitor owns `win`.
    #[inline]
    pub fn has_client(&self, win: WindowId) -> bool {
        self.clients.contains_key(&win)
    }

    /// Build a deliberate focus order in a fixture without admitting stale or
    /// duplicate windows. Runtime reordering uses the monitor's move methods.
    #[cfg(test)]
    pub(crate) fn set_focus_order(&mut self, order: Vec<WindowId>) -> bool {
        if order.len() != self.clients.len()
            || order.iter().copied().collect::<HashSet<_>>().len() != order.len()
            || order.iter().any(|win| !self.has_client(*win))
        {
            return false;
        }
        self.stack = order;
        true
    }

    /// Raise a client in this monitor's persistent overlap order.
    pub(crate) fn raise_client(&mut self, win: WindowId) -> bool {
        self.has_client(win) && self.z_order.raise(win)
    }

    /// Adopt a client already checked for global uniqueness by `WmModel`.
    pub(crate) fn adopt_client(&mut self, client: Client, selected: bool) {
        let win = client.win;
        assert!(
            !self.has_client(win),
            "client already owned by this monitor"
        );
        self.clients.insert(win, client);
        self.stack.insert(0, win);
        self.z_order.attach_top(win);
        if selected {
            self.selected = Some(win);
        }
    }

    /// Remove a client and all monitor-local references to it.
    pub(crate) fn take_client(&mut self, win: WindowId) -> Option<(Client, bool)> {
        let client = self.clients.remove(&win)?;
        self.stack.retain(|candidate| *candidate != win);
        self.z_order.remove(win);
        let was_selected = self.selected == Some(win);
        if was_selected {
            self.selected = None;
        }
        self.forget_focus(win);
        Some((client, was_selected))
    }

    /// Consume a removed output's clients in focus order for rehoming.
    pub(crate) fn into_orphaned_clients(mut self) -> OrphanedMonitorClients {
        let selected = self.selected;
        let z_order = self.z_order.as_slice().to_vec();
        let owned: HashSet<_> = self.clients.keys().copied().collect();
        assert_eq!(
            self.stack.iter().copied().collect::<HashSet<_>>(),
            owned,
            "removed monitor's focus order must contain every owned client"
        );
        assert_eq!(
            z_order.iter().copied().collect::<HashSet<_>>(),
            owned,
            "removed monitor's z-order must contain every owned client"
        );
        assert_eq!(self.stack.len(), owned.len(), "duplicate focus entry");
        assert_eq!(z_order.len(), owned.len(), "duplicate z-order entry");
        let mut orphaned = Vec::with_capacity(self.clients.len());
        // Adoption prepends. Walk the old stack backwards to preserve its
        // order after every client has been placed on the survivor.
        for win in std::mem::take(&mut self.stack).into_iter().rev() {
            if let Some(client) = self.clients.remove(&win) {
                orphaned.push((client, selected == Some(win)));
            }
        }
        assert!(
            self.clients.is_empty(),
            "owned client missing from focus order"
        );
        OrphanedMonitorClients {
            clients_in_reverse_focus_order: orphaned,
            z_order,
        }
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
    pub fn client_count(&self) -> usize {
        let selected = self.visible_tags();
        let mut count = 0;
        for (_win, c) in self.iter_clients() {
            if c.is_visible(selected) {
                count += 1;
            }
        }
        count
    }

    /// Count the number of tiled clients on this monitor.
    pub fn tiled_client_count(&self) -> usize {
        let selected = self.visible_tags();
        let mut count = 0;
        for (_win, c) in self.iter_clients() {
            if c.is_tiled(selected) {
                count += 1;
            }
        }
        count
    }

    /// Collect tiled clients into lightweight info snapshots for layout use.
    ///
    /// This replaces the per-layout boilerplate of filtering + snapshotting.
    pub fn collect_tiled(&self) -> Vec<TiledClientInfo> {
        let selected_tags = self.visible_tags();
        self.collect_client_info(|client| client.is_tiled(selected_tags))
    }

    /// Collect persistent tiling-tree members, including clients temporarily
    /// presented as fullscreen or maximized.
    pub fn collect_tiling_tree_members(&self) -> Vec<TiledClientInfo> {
        let selected_tags = self.visible_tags();
        self.collect_client_info(|client| client.is_tiling_tree_member(selected_tags))
    }

    /// Collect clients that hold a leaf position in the persistent tree for
    /// order maintenance.
    ///
    /// Unlike [`Self::collect_tiling_tree_members`], this includes hidden
    /// (minimized) tiled clients: in maximized presentation the tree is the
    /// tab/cycle order, so a minimized client must not lose its position.
    pub fn collect_tree_order_members(&self) -> Vec<TiledClientInfo> {
        let selected_tags = self.visible_tags();
        self.collect_client_info(|client| client.is_tree_order_member(selected_tags))
    }

    fn collect_client_info(&self, include: impl Fn(&Client) -> bool) -> Vec<TiledClientInfo> {
        self.iter_clients()
            .filter(|(_, client)| include(client))
            .map(|(win, client)| TiledClientInfo {
                win,
                border_width: client.border_width,
            })
            .collect()
    }

    /// Tiled clients in the stable order represented by the current manual
    /// tree. A newly managed client is appended defensively if reconciliation
    /// has not reached the tree yet.
    ///
    /// This is the order role of the tree: hidden (minimized) tiled clients
    /// keep their position so their bar title and cycle slot stay in place.
    /// Tiling geometry instead uses [`Self::collect_tiling_tree_members`].
    pub fn tiled_tree_order(&self) -> Vec<WindowId> {
        let selected = self.visible_tags();
        let mut ordered = self
            .per_tag()
            .map(|state| state.layout_tree.leaves())
            .unwrap_or_default()
            .into_iter()
            .filter(|win| {
                self.clients
                    .get(win)
                    .is_some_and(|client| client.is_tree_order_member(selected))
            })
            .collect::<Vec<_>>();
        let mut seen: HashSet<WindowId> = ordered.iter().copied().collect();

        for &win in self.stack.iter() {
            if self
                .clients
                .get(&win)
                .is_some_and(|client| client.is_tree_order_member(selected))
                && seen.insert(win)
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
    pub fn bar_client_order(&self) -> Vec<WindowId> {
        let selected = self.visible_tags();
        let mut ordered = if self.is_maximized_layout() {
            self.tiled_tree_order()
        } else {
            Vec::new()
        };
        let mut seen: HashSet<WindowId> = ordered.iter().copied().collect();

        for &win in self.stack.iter() {
            if self
                .clients
                .get(&win)
                .is_some_and(|client| client.shows_in_bar(selected))
                && seen.insert(win)
            {
                ordered.push(win);
            }
        }
        ordered
    }

    /// The windows keyboard focus cycling walks, in cycling order.
    ///
    /// This is [`Self::bar_client_order`] minus every entry that cannot take
    /// focus. The filter is [`Client::is_visible`] rather than
    /// [`Client::shows_in_bar`], so a minimized client keeps its bar title but
    /// drops out of the cycle.
    ///
    /// Maximized presentation overrides the source order: tiled clients cycle
    /// in [`Self::tiled_tree_order`] so the cycle follows the visible tab strip,
    /// and floating overlays are excluded. If that yields nothing — every tiled
    /// client is minimized — the bar order is used instead, which keeps
    /// floating clients cyclable on a monitor with no focusable tile. In
    /// maximized presentation the result is therefore a prefix of
    /// [`Self::bar_client_order`].
    pub fn focus_cycle_order(&self) -> Vec<WindowId> {
        let selected = self.visible_tags();

        if self.is_maximized_layout() {
            // The persistent tree is a stable, user-controlled order. Unlike
            // z-order it does not change merely because a window was focused,
            // and minimized entries keep their tree position so their bar title
            // stays put. They cannot receive focus until explicitly restored,
            // so the cycle skips them.
            let tiled_cycle: Vec<WindowId> = self
                .tiled_tree_order()
                .into_iter()
                .filter(|win| {
                    self.clients
                        .get(win)
                        .is_some_and(|client| client.is_visible(selected))
                })
                .collect();
            if !tiled_cycle.is_empty() {
                return tiled_cycle;
            }
        }

        // Outside maximized presentation — and as the fallback above — the
        // cycle follows the exact title order exposed by the bar.
        // Hidden/minimized entries retain a title but cannot receive focus
        // until explicitly restored, so skip them.
        self.bar_client_order()
            .into_iter()
            .filter(|win| {
                self.clients
                    .get(win)
                    .is_some_and(|client| client.is_visible(selected))
            })
            .collect()
    }

    /// Move a client within this monitor's focus list (stack order).
    ///
    /// Returns true if the position changed, false otherwise (e.g., if the client
    /// is floating, not found, or there are fewer than 2 tiled clients).
    pub fn move_client_in_stack(&mut self, win: WindowId, direction: StackDirection) -> bool {
        // Check if client exists and is tiled
        let is_floating = self
            .clients
            .get(&win)
            .map(|c| c.placement() == super::ClientPlacement::Floating)
            .unwrap_or(false);
        if is_floating {
            return false;
        }

        let tiled_count = self.tiled_client_count();
        if tiled_count < 2 {
            return false;
        }

        if let Some(pos) = self.stack.iter().position(|&w| w == win) {
            match direction {
                StackDirection::Previous => {
                    if pos > 0 {
                        self.stack.swap(pos, pos - 1);
                        return true;
                    } else {
                        // Wrap to end: move first element to end
                        if self.stack.len() > 1 {
                            let first = self.stack.remove(0);
                            self.stack.push(first);
                            return true;
                        }
                    }
                }
                StackDirection::Next => {
                    if pos + 1 < self.stack.len() {
                        self.stack.swap(pos, pos + 1);
                        return true;
                    } else {
                        // Wrap to beginning: move last element to front
                        if self.stack.len() > 1 {
                            let last = self.stack.pop();
                            if let Some(last) = last {
                                self.stack.insert(0, last);
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

    /// Exchange the focus-list positions of two clients on this monitor.
    ///
    /// Unlike [`Self::move_client_in_stack`] this has no placement
    /// restrictions: the bar title strip includes floating windows, and a
    /// bar-title drag reorders exactly the order the strip presents. Returns
    /// `false` when either window is absent or both share a position.
    pub fn swap_clients_in_stack(&mut self, first: WindowId, second: WindowId) -> bool {
        let Some(a) = self.stack.iter().position(|&w| w == first) else {
            return false;
        };
        let Some(b) = self.stack.iter().position(|&w| w == second) else {
            return false;
        };
        if a == b {
            return false;
        }
        self.stack.swap(a, b);
        true
    }

    /// Walk the persistent z-order and return the topmost visible, non-hidden
    /// client on the currently selected tags.
    ///
    /// `z_order` is bottom-to-top. Focus recovery walks it from the top so
    /// closing an overlapping window selects the window immediately below it.
    pub fn first_visible_client(&self) -> Option<WindowId> {
        let tags = self.visible_tags();
        self.z_order.iter_top_to_bottom().find_map(|w| {
            self.clients
                .get(&w)
                .filter(|c| c.is_visible(tags))
                .map(|_| w)
        })
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
    pub fn next_tiled(&self, start_win: Option<WindowId>) -> Option<WindowId> {
        let selected = self.visible_tags();

        let start_idx = if let Some(win) = start_win {
            self.stack.iter().position(|&w| w == win)
        } else {
            None
        };

        let iter_start = start_idx.map(|i| i + 1).unwrap_or(0);

        for &win in self.stack.iter().skip(iter_start) {
            if let Some(c) = self.clients.get(&win)
                && c.mode().is_normal_tiling()
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

    /// Effective bar visibility for the given tag mask: the per-view
    /// session override when one is set, otherwise the configured default
    /// ([`Self::bar_default_show`]).
    pub fn show_bar_for_mask(&self, mask: TagMask) -> bool {
        self.per_tag
            .get(&mask)
            .and_then(|state| state.show_bar)
            .unwrap_or(self.bar_default_show)
    }

    /// Returns true when an exclusive layer-shell surface reserves space on the
    /// top edge where instantWM would place its own bar.
    pub fn has_external_bar_on_internal_bar_edge(&self) -> bool {
        self.available_rect.y > self.monitor_rect.y
    }

    /// Returns presentation state for the given tag mask.
    pub fn presentation_for_mask(&self, mask: TagMask) -> PresentationMode {
        self.per_tag
            .get(&mask)
            .map(|state| state.presentation)
            .unwrap_or_default()
    }

    /// Get the name data for a given tag index (1-based).
    pub fn tag_name(&self, tag_index: usize) -> Option<&Tag> {
        tag_index.checked_sub(1).and_then(|i| self.tags.get(i))
    }

    /// Get the current tag name data for this monitor.
    pub fn current_tag(&self) -> Option<&Tag> {
        let idx = self.current_tag_number()?;
        if idx > 0 && idx <= self.tags.len() {
            Some(&self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Get a mutable reference to the current tag name data.
    pub fn current_tag_mut(&mut self) -> Option<&mut Tag> {
        let idx = self.current_tag_number()?;
        if idx > 0 && idx <= self.tags.len() {
            Some(&mut self.tags[idx - 1])
        } else {
            None
        }
    }

    /// Active layout slot identity for the given tag mask.
    ///
    /// Defaults to master/stack: a tag that never received a layout command
    /// grows its tree organically, which is the tile layout.
    pub fn active_preset_for_mask(&self, mask: TagMask) -> crate::layouts::tree::Preset {
        self.per_tag
            .get(&mask)
            .map_or(crate::layouts::tree::Preset::MasterStack, |state| {
                state.active_preset
            })
    }

    /// Get the layout symbol shown in the bar for the given tag mask.
    ///
    /// Lens presentations show their own symbol; tiled tags show the symbol
    /// of the active layout slot.
    pub fn layout_symbol_for_mask(&self, mask: TagMask) -> &'static str {
        match self.presentation_for_mask(mask) {
            PresentationMode::Tiled => {
                LayoutCommand::from_tree_preset(self.active_preset_for_mask(mask))
                    .unwrap_or(LayoutCommand::Tile)
                    .symbol()
            }
            presentation => presentation.symbol(),
        }
    }

    /// Get the current layout symbol for this monitor.
    pub fn layout_symbol(&self) -> String {
        self.layout_symbol_for_mask(self.selected_tags())
            .to_string()
    }

    /// The layout-cycle entry matching this monitor's full presentation
    /// state: the active lens while lensed, the active slot otherwise.
    ///
    /// This is the single source of truth for "which layout is the user in"
    /// shared by layout cycling and layout IPC queries.
    pub fn current_layout_command(&self) -> LayoutCommand {
        match self.current_layout() {
            PresentationMode::Floating => LayoutCommand::Floating,
            PresentationMode::Maximized => LayoutCommand::Maximized,
            PresentationMode::Tiled => self
                .per_tag()
                .and_then(|state| LayoutCommand::from_tree_preset(state.active_preset))
                .unwrap_or(LayoutCommand::Tile),
        }
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

    /// Bar Y position (vertical position of the status bar).
    ///
    /// Derived from `available_rect`, `bar_height` and `shows_bar()`
    /// so it can never fall out of sync with the monitor's real geometry.
    pub fn bar_y(&self) -> i32 {
        let safe_bh = self.bar_height.min(self.available_rect.h.max(0));
        if self.shows_bar() {
            self.available_rect.y
        } else {
            self.available_rect.y - safe_bh
        }
    }

    /// Work area geometry (excluding bar and exclusive layer surfaces).
    ///
    /// Derived from `available_rect`, `bar_height`, the bottom bar height and
    /// the bar visibility flags, so it can never fall out of sync with the
    /// monitor's real geometry.
    pub fn work_rect(&self) -> Rect {
        self.rect_excluding_internal_bars(self.shows_bar(), self.shows_bottom_bar())
    }

    /// Area not occupied by exclusive layer surfaces or the currently visible
    /// built-in bars.
    ///
    /// Unlike [`Self::work_rect`], this accounts for a true-fullscreen client
    /// temporarily hiding the built-in bars. It is intended for WM-owned UI
    /// such as edge scratchpads that must avoid every visible bar.
    pub fn visible_content_rect(&self) -> Rect {
        self.rect_excluding_internal_bars(self.bar_visible(), self.bottom_bar_visible())
    }

    fn rect_excluding_internal_bars(
        &self,
        top_bar_visible: bool,
        bottom_bar_visible: bool,
    ) -> Rect {
        let safe_bh = if top_bar_visible {
            self.bar_height.min(self.available_rect.h.max(0))
        } else {
            0
        };
        let safe_bbh = if bottom_bar_visible {
            self.bottom_bar_height
                .min((self.available_rect.h - safe_bh).max(0))
        } else {
            0
        };
        Rect::new(
            self.available_rect.x,
            self.available_rect.y + safe_bh,
            self.available_rect.w.max(1),
            (self.available_rect.h - safe_bh - safe_bbh).max(1),
        )
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
        metrics: MonitorUiMetrics,
    ) {
        self.num = index as i32;
        self.monitor_rect = rect;
        // Reset the available rect to the full output. The Wayland backend
        // re-applies layer-shell exclusive zones on top of this. The work area
        // and bar position are derived from this rectangle on access.
        self.available_rect = rect;
        self.name = name;
        self.set_ui_metrics(scale, metrics);
    }

    /// Set effective UI metrics for this monitor.
    pub fn set_ui_metrics(&mut self, ui_scale: f64, metrics: MonitorUiMetrics) {
        self.ui_scale = if ui_scale.is_finite() && ui_scale > 0.0 {
            ui_scale
        } else {
            1.0
        };
        // This is the single place a UI metric's minimum is decided. Callers
        // hand over already-scaled values; `scaled_px` clamps at the source
        // and the remaining floors live here, so no layer re-clamps a value
        // another layer already settled.
        let bar_height = metrics.bar_height.max(0);
        self.bar_height = bar_height;
        // The bottom strip is just a gesture handle, so it stays thinner than
        // the status bar. Half the top bar height (with a small minimum so the
        // indicator still has room to breathe) keeps it subtle but reachable.
        self.bottom_bar_height = (bar_height / 2).max(6);
        self.horizontal_padding = metrics.horizontal_padding.max(0);
        self.startmenu_size = metrics.startmenu_size.max(0);
    }

    /// This monitor's bar UI metrics as a single transport value.
    #[inline]
    pub fn ui_metrics(&self) -> MonitorUiMetrics {
        MonitorUiMetrics {
            bar_height: self.bar_height,
            horizontal_padding: self.horizontal_padding,
            startmenu_size: self.startmenu_size,
        }
    }

    /// Get the width of the monitor's work area.
    pub fn width(&self) -> i32 {
        self.work_rect().w
    }

    /// Get the height of the monitor's work area.
    pub fn height(&self) -> i32 {
        self.work_rect().h
    }

    /// Compute a bitmask of tags that have at least one client on this monitor.
    ///
    /// Excludes the scratchpad tag from the result.
    pub fn occupied_tags(&self) -> TagMask {
        let mut occupied = TagMask::EMPTY;
        for (_win, c) in self.iter_clients() {
            occupied = occupied | c.tags;
        }
        occupied.without_scratchpad()
    }
}

#[cfg(test)]
mod tests;
