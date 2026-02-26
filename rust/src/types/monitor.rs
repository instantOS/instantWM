//! Monitor/screen types.
//!
//! Types for managing multiple monitors/screens.

use std::collections::HashMap;

use crate::layouts::LayoutKind;
use crate::types::client::{Client, ClientListIter, ClientStackIter};
use crate::types::geometry::Rect;
use crate::types::input::Gesture;
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
    pub by: i32,
    /// Width reserved for client title display in the bar.
    pub bar_clients_width: i32,
    /// Bar thickness/height in pixels.
    pub bt: i32,
    /// Full monitor geometry (including bar).
    pub monitor_rect: Rect,
    /// Work area geometry (excluding bar).
    pub work_rect: Rect,
    /// Currently selected tag set index.
    pub seltags: u32,
    /// Tag sets (two sets for switching).
    pub tagset: [u32; 2],
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
    pub gesture: Gesture,
    /// Bar window handle.
    pub barwin: WindowId,
    /// Which tags to show.
    pub showtags: u32,
    /// Current tag index (1-based).
    pub current_tag: usize,
    /// Previous tag index (1-based).
    pub prev_tag: usize,
    /// Tags owned by this monitor.
    pub tags: Vec<Tag>,
    /// Head of client list (focus order).
    pub clients: Option<WindowId>,
    /// Currently selected client.
    pub sel: Option<WindowId>,
    /// Overlay window.
    pub overlay: Option<WindowId>,
    /// Head of stack list (stacking order).
    pub stack: Option<WindowId>,
    /// Currently fullscreen client.
    pub fullscreen: Option<WindowId>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            monitor_id: 0,
            mfact: 0.55,
            nmaster: 1,
            num: 0,
            by: 0,
            bar_clients_width: 0,
            bt: 0,
            monitor_rect: Rect::default(),
            work_rect: Rect::default(),
            seltags: 0,
            tagset: [0; 2],
            activeoffset: 0,
            titleoffset: 0,
            clientcount: 0,
            showbar: true,
            topbar: true,
            overlaystatus: 0,
            overlaymode: OverlayMode::default(),
            gesture: Gesture::default(),
            barwin: WindowId::default(),
            showtags: 0,
            current_tag: 0,
            prev_tag: 0,
            tags: Vec::new(),
            clients: None,
            sel: None,
            overlay: None,
            stack: None,
            fullscreen: None,
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
            tagset: [1, 1],
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
        self.tagset[self.seltags as usize]
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
        ClientListIter::new(self.clients, clients)
    }

    /// Iterate the monitor's stack list (stacking order).
    #[inline]
    pub fn iter_stack<'a>(&'a self, clients: &'a HashMap<WindowId, Client>) -> ClientStackIter<'a> {
        ClientStackIter::new(self.stack, clients)
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
            if c.is_visible_on_tags(selected) && !c.isfloating && !c.is_hidden {
                count += 1;
            }
        }
        count
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
        if self.showbar {
            self.work_rect.y = if self.topbar {
                self.monitor_rect.y + bar_height
            } else {
                self.monitor_rect.y
            };
            self.work_rect.h = self.monitor_rect.h - bar_height;
            self.by = if self.topbar {
                self.monitor_rect.y
            } else {
                self.monitor_rect.y + self.monitor_rect.h - bar_height
            };
        } else {
            self.work_rect.y = self.monitor_rect.y;
            self.work_rect.h = self.monitor_rect.h;
            self.by = if self.topbar {
                -bar_height
            } else {
                self.monitor_rect.h
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
