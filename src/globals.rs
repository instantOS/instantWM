use crate::client::manager::ClientManager;
use crate::config::ModeConfig;
use crate::config::commands::ExternalCommands;
use crate::monitor::MonitorManager;
use crate::types::*;
use std::collections::BTreeSet;
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// Sub-structs for RuntimeConfig
// ---------------------------------------------------------------------------

/// Display/screen dimensions.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DisplayConfig {
    pub width: i32,
    pub height: i32,
}

/// Window behaviour settings.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WindowConfig {
    pub border_width_px: i32,
    pub snap_threshold: i32,
    pub resizehints: bool,
    pub decorhints: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            border_width_px: 1,
            snap_threshold: 32,
            resizehints: true,
            decorhints: false,
        }
    }
}

/// Status bar settings.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BarConfig {
    pub show: bool,
    pub top: bool,
    pub height: i32,
    pub horizontal_padding: i32,
    pub startmenu_size: i32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            show: true,
            top: true,
            height: 0,
            horizontal_padding: 0,
            startmenu_size: 0,
        }
    }
}

/// System tray settings.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SystrayConfig {
    pub show: bool,
    pub pinning: usize,
    pub spacing: i32,
}

impl Default for SystrayConfig {
    fn default() -> Self {
        Self {
            show: true,
            pinning: 0,
            spacing: 2,
        }
    }
}

/// Colour schemes for various UI elements.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ColorConfig {
    pub window: WindowColorConfigs,
    pub close_button: CloseButtonColorConfigs,
    pub border: BorderColorConfig,
    pub status_bar: StatusColorConfig,
}

/// Keybindings, mouse buttons, modes, and client rules.
#[derive(Clone, Default)]
pub struct BindingConfig {
    pub keys: Vec<Key>,
    pub desktop_keybinds: Vec<Key>,
    pub modes: std::collections::HashMap<String, ModeConfig>,
    pub buttons: Vec<Button>,
    pub rules: Vec<Rule>,
}

/// Font configuration.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FontConfig {
    pub fonts: Vec<String>,
    pub config_font: String,
}

/// Runtime configuration - composed from sub-structs.
pub struct RuntimeConfig {
    pub display: DisplayConfig,
    pub window: WindowConfig,
    pub bar: BarConfig,
    pub systray: SystrayConfig,
    pub layout: crate::config::config_toml::LayoutConfig,
    pub colors: ColorConfig,
    pub bindings: BindingConfig,
    pub fonts: FontConfig,
    pub external_commands: ExternalCommands,
    /// Template tag list cloned into every new monitor.
    pub tag_template: Vec<crate::types::monitor::TagNames>,
    pub input: std::collections::HashMap<String, crate::config::config_toml::InputConfig>,
    pub monitors: std::collections::HashMap<String, crate::config::config_toml::MonitorConfig>,
    pub status_command: Option<String>,
    pub cursor: crate::config::config_toml::CursorConfig,
    pub exec_once: Vec<String>,
    pub exec: Vec<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            display: DisplayConfig::default(),
            window: WindowConfig::default(),
            bar: BarConfig::default(),
            systray: SystrayConfig::default(),
            layout: crate::config::config_toml::LayoutConfig::default(),
            colors: ColorConfig::default(),
            bindings: BindingConfig::default(),
            fonts: FontConfig::default(),
            external_commands: crate::config::commands::default_commands(),
            tag_template: Vec::new(),
            input: std::collections::HashMap::new(),
            monitors: std::collections::HashMap::new(),
            status_command: None,
            cursor: crate::config::config_toml::CursorConfig::default(),
            exec_once: Vec::new(),
            exec: Vec::new(),
        }
    }
}

impl Clone for RuntimeConfig {
    fn clone(&self) -> Self {
        Self {
            display: self.display.clone(),
            window: self.window.clone(),
            bar: self.bar.clone(),
            systray: self.systray.clone(),
            layout: self.layout,
            colors: self.colors.clone(),
            bindings: self.bindings.clone(),
            fonts: self.fonts.clone(),
            external_commands: self.external_commands.clone(),
            tag_template: self.tag_template.clone(),
            input: self.input.clone(),
            monitors: self.monitors.clone(),
            status_command: self.status_command.clone(),
            cursor: self.cursor.clone(),
            exec_once: self.exec_once.clone(),
            exec: self.exec.clone(),
        }
    }
}

/// What kind of drag interaction is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragType {
    #[default]
    Move,
    Resize(ResizeDirection),
}

#[derive(Debug, Clone, Default)]
pub struct DragInteraction {
    pub active: bool,
    pub win: WindowId,
    pub button: MouseButton,
    /// Whether the pointer has exceeded the drag threshold and we are
    /// actively moving/resizing.  When `false`, releasing the button
    /// triggers the click action instead (focus/hide/zoom).
    pub dragging: bool,
    pub drag_type: DragType,
    pub win_start_geo: Rect,
    pub start_point: Point,
    pub last_root_point: Point,
    /// Geometry to restore when the window is re-tiled (e.g. dropped on
    /// the bar).  For windows that were already floating this equals
    /// `win_start_geo`; for tiled windows promoted during the drag it
    /// preserves the saved float dimensions.
    pub drop_restore_geo: Rect,
    pub was_focused: bool,
    pub was_hidden: bool,
    pub suppress_click_action: bool,
}

impl DragInteraction {
    /// Create a new Move drag interaction.
    ///
    /// Note: This constructor is used exclusively for immediate-start drag contexts
    /// (such as keyboard-driven moves or client/Wayland click-drags), and therefore
    /// initializes `dragging` as `true` immediately.
    pub fn new_move(win: WindowId, button: MouseButton, start: Point, geo: Rect) -> Self {
        Self {
            active: true,
            win,
            button,
            dragging: true,
            drag_type: DragType::Move,
            start_point: start,
            win_start_geo: geo,
            drop_restore_geo: geo,
            last_root_point: start,
            ..Default::default()
        }
    }

    /// Create a new Resize drag interaction.
    ///
    /// Note: This constructor is used exclusively for immediate-start resize contexts
    /// (such as keyboard-driven resizing or direct click-to-resize/Wayland client resize),
    /// and therefore initializes `dragging` as `true` immediately.
    pub fn new_resize(
        win: WindowId,
        button: MouseButton,
        dir: ResizeDirection,
        start: Point,
        geo: Rect,
    ) -> Self {
        Self {
            active: true,
            win,
            button,
            dragging: true,
            drag_type: DragType::Resize(dir),
            start_point: start,
            win_start_geo: geo,
            drop_restore_geo: geo,
            last_root_point: start,
            ..Default::default()
        }
    }
}

/// On X11, the synchronous grab loop drives this. On Wayland, the calloop
/// press/motion/release events drive it asynchronously.
#[derive(Debug, Clone, Default)]
pub struct TagDragState {
    /// Whether a tag drag is currently active.
    pub active: bool,
    /// The initial tag mask that was clicked.
    pub initial_tag: TagMask,
    /// Monitor ID where the drag started.
    pub monitor_id: MonitorId,
    /// Monitor X origin (for converting root coords to local).
    pub mon_mx: i32,
    /// Last seen tag gesture index (None = none).
    pub last_tag: Option<usize>,
    /// Whether cursor is still on the bar.
    pub cursor_on_bar: bool,
    /// Last motion coordinates + modifier state (for release handling).
    pub last_motion: Option<(i32, i32, u32)>,
    /// The mouse button that started the drag.
    pub button: MouseButton,
}

#[derive(Debug, Clone, Default)]
pub struct GestureInteraction {
    pub active: bool,
    pub button: MouseButton,
    pub monitor_id: MonitorId,
    pub last_y: i32,
}

/// The pointer-owned interaction currently being offered before a click commits it.
///
/// This is the source of truth for hover offers; the cursor icon is a
/// side-effect, not the other way around.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum HoverOffer {
    #[default]
    None,
    /// Cursor is in the resize border zone of a floating window.
    Resize { win: WindowId, dir: ResizeDirection },
    /// Cursor is on the sidebar drag edge.
    Sidebar(SidebarTarget),
}

impl HoverOffer {
    /// Whether any hover interaction is offered (not [`HoverOffer::None`]).
    #[inline]
    pub fn is_active(self) -> bool {
        !matches!(self, HoverOffer::None)
    }

    #[inline]
    pub fn is_sidebar(self) -> bool {
        matches!(self, HoverOffer::Sidebar(_))
    }

    /// The resize target and direction when this is a border-resize offer.
    #[inline]
    pub fn resize_target(self) -> Option<(WindowId, ResizeDirection)> {
        match self {
            HoverOffer::Resize { win, dir } => Some((win, dir)),
            _ => None,
        }
    }
}

/// Consolidated state for mouse/touch interactions.
#[derive(Debug, Clone, Default)]
pub struct DragState {
    pub tag: TagDragState,
    pub interactive: DragInteraction,
    pub gesture: GestureInteraction,
    pub bar_active: bool,
    pub hover_offer: HoverOffer,
}

impl DragState {
    #[inline]
    pub fn any_drag_active(&self) -> bool {
        self.interactive.active || self.tag.active || self.gesture.active
    }

    #[inline]
    pub fn set_hover_offer(&mut self, offer: HoverOffer) {
        self.hover_offer = offer;
    }

    /// Clears an active hover offer. Returns `true` if the state changed.
    pub fn clear_hover_offer(&mut self) -> bool {
        if !self.hover_offer.is_active() {
            return false;
        }
        self.hover_offer = HoverOffer::None;
        true
    }
}

/// A single keyboard layout with optional variant.
#[derive(Debug, Clone, Default)]
pub struct KeyboardLayout {
    /// XKB layout name (e.g., "us", "de", "fr").
    pub name: String,
    /// XKB variant for this layout (e.g., "nodeadkeys", "colemak").
    pub variant: Option<String>,
}

impl KeyboardLayout {
    /// Create a new keyboard layout.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variant: None,
        }
    }

    /// Create a new keyboard layout with a variant.
    pub fn with_variant(name: impl Into<String>, variant: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variant: Some(variant.into()),
        }
    }
}

impl From<&str> for KeyboardLayout {
    fn from(s: &str) -> Self {
        // Parse "layout(variant)" syntax
        if let Some((name, variant)) = s.strip_suffix(')').and_then(|s| s.rsplit_once('(')) {
            Self::with_variant(name, variant)
        } else {
            Self::new(s)
        }
    }
}

impl From<String> for KeyboardLayout {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

/// Keyboard (XKB) layout runtime state.
#[derive(Debug, Clone, Default)]
pub struct KeyboardLayoutState {
    /// Configured XKB layouts with optional variants.
    pub layouts: Vec<KeyboardLayout>,
    /// XKB options string.
    pub options: Option<String>,
    /// XKB model string.
    pub model: Option<String>,
    /// Swap Caps Lock and Escape.
    pub swap_escape: bool,
    /// Index of the currently active layout in `layouts`.
    pub current: usize,
}

impl KeyboardLayoutState {
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    pub fn layout(&self, index: usize) -> Option<&KeyboardLayout> {
        self.layouts.get(index)
    }

    pub fn find_layout_index(&self, name: &str) -> Option<usize> {
        self.layouts.iter().position(|layout| layout.name == name)
    }

    pub fn reset_layouts(&mut self, layouts: Vec<KeyboardLayout>) {
        self.layouts = layouts;
        self.current = 0;
    }

    pub fn add_layout(&mut self, layout: KeyboardLayout) -> Result<usize, String> {
        if self.find_layout_index(&layout.name).is_some() {
            return Err(format!("layout '{}' already exists", layout.name));
        }

        let new_index = self.layouts.len();
        self.layouts.push(layout);
        Ok(new_index)
    }

    pub fn remove_layout(&mut self, index: usize) -> Result<(), String> {
        if self.layouts.len() == 1 {
            return Err("cannot remove the last layout".to_string());
        }

        self.layouts.remove(index);

        if index < self.current {
            self.current -= 1;
        } else if index == self.current && self.current >= self.layouts.len() {
            self.current = self.layouts.len() - 1;
        }

        Ok(())
    }

    /// The currently active layout name, or `None` if no layouts are configured.
    pub fn current_layout(&self) -> Option<&str> {
        self.layouts.get(self.current).map(|l| l.name.as_str())
    }

    /// The variant for the currently active layout, or empty string.
    pub fn current_variant(&self) -> &str {
        self.layouts
            .get(self.current)
            .and_then(|l| l.variant.as_deref())
            .unwrap_or("")
    }
}

/// Runtime behaviour toggles and transient WM mode state.
#[derive(Debug, Clone)]
pub struct WmBehavior {
    pub animated: bool,
    pub focus_follows_mouse: bool,
    pub focus_follows_float_mouse: bool,
    /// Last WM-owned cursor presentation requested through `set_cursor_style`.
    ///
    /// This is not interaction state. Hover-resize, move/resize drags, and
    /// other input modes must use their explicit state fields as the source of
    /// truth; this field only lets cursor application/reset code avoid treating
    /// the backend cursor as an implicit mode flag.
    pub requested_cursor: AltCursor,
    pub double_draw: bool,
    pub specialnext: SpecialNext,
    /// Current active mode (sway-like modes).
    pub current_mode: String,
}

impl Default for WmBehavior {
    fn default() -> Self {
        Self {
            animated: true,
            focus_follows_mouse: true,
            focus_follows_float_mouse: true,
            requested_cursor: AltCursor::Default,
            double_draw: false,
            specialnext: SpecialNext::None,
            current_mode: "default".to_string(),
        }
    }
}

/// Batched layout targets waiting to be arranged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutWorkTargets {
    AllMonitors,
    Monitors(Vec<MonitorId>),
}

/// Pending layout invalidation with per-monitor granularity.
#[derive(Debug, Clone, Default)]
pub struct PendingLayoutWork {
    all_monitors: bool,
    monitors: BTreeSet<MonitorId>,
    urgent: bool,
}

impl PendingLayoutWork {
    pub fn mark_all(&mut self) {
        self.all_monitors = true;
        self.monitors.clear();
    }

    pub fn mark_all_urgent(&mut self) {
        self.mark_all();
        self.urgent = true;
    }

    pub fn mark_monitor(&mut self, monitor_id: MonitorId) {
        if !self.all_monitors {
            self.monitors.insert(monitor_id);
        }
    }

    pub fn mark_monitor_urgent(&mut self, monitor_id: MonitorId) {
        self.mark_monitor(monitor_id);
        self.urgent = true;
    }

    pub fn mark_monitor_opt(&mut self, monitor_id: Option<MonitorId>) {
        if let Some(monitor_id) = monitor_id {
            self.mark_monitor(monitor_id);
        } else {
            self.mark_all();
        }
    }

    pub fn is_pending(&self) -> bool {
        self.all_monitors || !self.monitors.is_empty()
    }

    pub fn is_urgent(&self) -> bool {
        self.urgent
    }

    pub fn clear(&mut self) {
        self.all_monitors = false;
        self.monitors.clear();
        self.urgent = false;
    }

    /// Consume and return pending layout targets.
    pub fn take_targets(&mut self) -> Option<LayoutWorkTargets> {
        if self.all_monitors {
            self.clear();
            return Some(LayoutWorkTargets::AllMonitors);
        }
        if self.monitors.is_empty() {
            self.urgent = false;
            return None;
        }
        let monitors = self.monitors.iter().copied().collect();
        self.clear();
        Some(LayoutWorkTargets::Monitors(monitors))
    }
}

/// Work queue consumed by runtime ticks.
#[derive(Debug, Clone)]
pub struct PendingWork {
    /// Whether input configuration has changed and needs to be re-applied.
    pub input_config: bool,
    /// Whether monitor configuration has changed and needs to be re-applied.
    pub monitor_config: bool,
    /// Pending layout work.
    pub layout: PendingLayoutWork,
}

impl Default for PendingWork {
    fn default() -> Self {
        let mut layout = PendingLayoutWork::default();
        layout.mark_all();
        Self {
            input_config: false,
            monitor_config: false,
            layout,
        }
    }
}

pub struct Globals {
    // Runtime configuration (loaded from config files)
    pub(crate) cfg: RuntimeConfig,

    // Runtime state (changes during WM operation)
    pub(crate) monitors: MonitorManager,
    pub(crate) clients: ClientManager,
    pub(crate) tags: TagSet,

    // Grouped subsystems
    pub(crate) behavior: WmBehavior,
    pub(crate) drag: DragState,

    /// XKB keyboard layout state.
    pub(crate) keyboard_layout: KeyboardLayoutState,
    /// Recently spawned processes awaiting their first managed window.
    pub(crate) pending_launches: VecDeque<crate::client::PendingLaunch>,
}

impl Globals {
    // -------------------------------------------------------------------------
    // Selected-monitor convenience helpers
    // -------------------------------------------------------------------------

    /// Return the window currently selected on the selected monitor, if any.
    #[inline]
    pub fn selected_win(&self) -> Option<WindowId> {
        self.monitors.sel().and_then(|m| m.sel)
    }

    /// Return the ID of the currently selected monitor.
    pub fn selected_monitor_id(&self) -> MonitorId {
        self.monitors.sel_idx()
    }

    /// Change the currently selected monitor.
    pub fn set_selected_monitor(&mut self, id: MonitorId) {
        self.monitors.set_sel_idx(id);
    }

    /// Shorthand to get the selected monitor.
    pub fn selected_monitor(&self) -> &Monitor {
        self.monitors.sel_unchecked()
    }

    /// Shorthand to get the selected monitor mutably.
    pub fn selected_monitor_mut(&mut self) -> &mut Monitor {
        self.monitors.sel_mut_unchecked()
    }

    /// Shorthand to get the selected monitor (Option version for cases that need it).
    pub fn selected_monitor_opt(&self) -> Option<&Monitor> {
        self.monitors.sel()
    }

    /// Shorthand to get the selected monitor mutably (Option version).
    pub fn selected_monitor_mut_opt(&mut self) -> Option<&mut Monitor> {
        self.monitors.sel_mut()
    }

    /// Delegation to get a monitor by index.
    pub fn monitor(&self, id: MonitorId) -> Option<&Monitor> {
        self.monitors.get(id)
    }

    /// Delegation to get a mutable monitor by index.
    pub fn monitor_mut(&mut self, id: MonitorId) -> Option<&mut Monitor> {
        self.monitors.get_mut(id)
    }

    /// Delegation to iterate over monitors.
    pub fn monitors_iter(&self) -> impl Iterator<Item = (MonitorId, &Monitor)> {
        self.monitors.iter()
    }

    /// Iterate over all monitors (without index).
    pub fn monitors_iter_all(&self) -> impl Iterator<Item = &Monitor> {
        self.monitors.iter_all()
    }

    /// Delegation to iterate over monitors mutably.
    pub fn monitors_iter_mut(&mut self) -> impl Iterator<Item = (MonitorId, &mut Monitor)> {
        self.monitors.iter_mut()
    }

    /// Iterate over all monitors mutably (without index).
    pub fn monitors_iter_all_mut(&mut self) -> impl Iterator<Item = &mut Monitor> {
        self.monitors.iter_all_mut()
    }

    /// Clear the maximized reference on any monitor that holds `win`.
    pub fn clear_maximized_for(&mut self, win: WindowId) {
        for mon in self.monitors_iter_all_mut() {
            if mon.maximized == Some(win) {
                mon.maximized = None;
            }
        }
    }

    // -------------------------------------------------------------------------
    // Client List Management (Attach/Detach)
    // -------------------------------------------------------------------------

    /// Attach `win` to its assigned monitor's focus list.
    pub fn attach(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.monitor_id(win)
            && let Some(mon) = self.monitors.get_mut(mid)
        {
            mon.clients.insert(0, win);
        }
    }

    /// Detach `win` from its assigned monitor's focus list.
    pub fn detach(&mut self, win: WindowId) {
        let monitor_id = self.clients.monitor_id(win);
        if let Some(mid) = monitor_id
            && let Some(mon) = self.monitors.get_mut(mid)
            && mon.clients.contains(&win)
        {
            mon.clients.retain(|&w| w != win);
            return;
        }

        // Fallback: search all monitors if not found on the assigned one.
        for mon in self.monitors.iter_all_mut() {
            if mon.clients.contains(&win) {
                mon.clients.retain(|&w| w != win);
            }
        }
    }

    /// Attach `win` to the top of its assigned monitor's persistent z-order.
    ///
    /// Does **not** modify `mon.sel` — callers are responsible for
    /// deciding selection via `focus_soft` or explicit assignment.
    pub fn attach_z_order_top(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.monitor_id(win)
            && let Some(mon) = self.monitors.get_mut(mid)
        {
            mon.z_order.attach_top(win);
        }
    }

    /// Detach `win` from its assigned monitor's persistent z-order.
    ///
    /// Pure list operation — does **not** modify `mon.sel`.
    /// Callers are responsible for focus recovery (e.g. `focus_soft`,
    /// `restore_focus_after_overlay`, or explicit `mon.sel` assignment).
    pub fn detach_z_order(&mut self, win: WindowId) {
        let monitor_id = self.clients.monitor_id(win);

        let handle_monitor = |mon: &mut crate::types::Monitor| -> bool { mon.z_order.remove(win) };

        if let Some(mid) = monitor_id
            && let Some(mon) = self.monitors.get_mut(mid)
            && handle_monitor(mon)
        {
            return;
        }

        // Fallback: search all monitors if not found on the assigned one.
        for mon in self.monitors.iter_all_mut() {
            if handle_monitor(mon) {
                return;
            }
        }
    }

    /// Move `win` to the top of its monitor's persistent z-order.
    ///
    /// The backend may also have its own native stacking structure (X11 server
    /// order, Smithay `Space`, etc.). This keeps the backend-agnostic model in
    /// sync so later z-order syncs, focus recovery, and focus-stack traversal do not
    /// rebuild z-order from stale state.
    pub fn raise_client_in_z_order(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.monitor_id(win)
            && let Some(mon) = self.monitors.get_mut(mid)
            && mon.z_order.raise(win)
        {
            return;
        }

        // Fallback: search all monitors if the client's monitor assignment is
        // stale during a transfer or teardown path.
        for mon in self.monitors.iter_all_mut() {
            if mon.z_order.raise(win) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LayoutWorkTargets, PendingLayoutWork};
    use crate::types::MonitorId;

    #[test]
    fn pending_layout_returns_sorted_monitor_targets() {
        let mut pending = PendingLayoutWork::default();
        pending.mark_monitor(MonitorId(3));
        pending.mark_monitor(MonitorId(1));
        pending.mark_monitor(MonitorId(2));

        let targets = pending.take_targets();
        assert_eq!(
            targets,
            Some(LayoutWorkTargets::Monitors(vec![
                MonitorId(1),
                MonitorId(2),
                MonitorId(3),
            ]))
        );
        assert!(!pending.is_pending());
    }

    #[test]
    fn pending_layout_all_overrides_specific_targets() {
        let mut pending = PendingLayoutWork::default();
        pending.mark_monitor(MonitorId(2));
        pending.mark_all();
        pending.mark_monitor(MonitorId(1)); // ignored while all-monitors is set

        assert_eq!(pending.take_targets(), Some(LayoutWorkTargets::AllMonitors));
        assert!(!pending.is_pending());
    }

    #[test]
    fn pending_layout_none_monitor_marks_all() {
        let mut pending = PendingLayoutWork::default();
        pending.mark_monitor_opt(None);
        assert_eq!(pending.take_targets(), Some(LayoutWorkTargets::AllMonitors));
    }
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            cfg: RuntimeConfig::default(),
            monitors: MonitorManager::new(),
            clients: ClientManager::new(),
            tags: TagSet::default(),
            behavior: WmBehavior::default(),
            drag: DragState::default(),
            keyboard_layout: KeyboardLayoutState::default(),
            pending_launches: VecDeque::new(),
        }
    }
}

/// Build and atomically install runtime configuration.
///
/// The complete value is assembled off to the side so readers never observe
/// a partially-applied reload. Display geometry is backend-derived and is
/// therefore preserved across config replacement.
pub fn apply_config(g: &mut Globals, cfg: &crate::config::Config) {
    let display = g.cfg.display.clone();
    let mut next = RuntimeConfig {
        display,
        ..RuntimeConfig::default()
    };
    next.window.border_width_px = cfg.borderpx;
    next.input = cfg.input.clone();
    next.monitors = cfg.monitors.clone();
    next.window.snap_threshold = cfg.snap_threshold;
    next.bar.startmenu_size = cfg.startmenu_size;
    next.systray.pinning = cfg.systraypinning;
    next.systray.spacing = cfg.systrayspacing;
    next.systray.show = cfg.show_systray;
    next.bar.show = cfg.showbar;
    next.bar.top = cfg.topbar;
    next.bar.height = cfg.bar_height;
    next.window.resizehints = cfg.resize_hints;
    next.window.decorhints = cfg.decorhints;
    next.layout = crate::config::config_toml::LayoutConfig {
        inner_gap: cfg.layout.inner_gap.max(0),
        outer_gap: cfg.layout.outer_gap.max(0),
        smart_gaps: cfg.layout.smart_gaps,
        monocle_gaps: cfg.layout.monocle_gaps,
    };

    next.colors.window = cfg.window_colors.clone();
    next.colors.close_button = cfg.closebuttoncolors.clone();
    next.colors.border = cfg.border_colors;
    next.colors.status_bar = cfg.statusbarcolors;

    next.bindings.keys = cfg.keys.clone();
    next.bindings.desktop_keybinds = cfg.desktop_keybinds.clone();
    next.bindings.modes = cfg.modes.clone();
    next.bindings.buttons = cfg.buttons.clone();
    next.bindings.rules = cfg.rules.clone();
    next.fonts.fonts = cfg.fonts.clone();
    next.external_commands = cfg.external_commands.clone();
    next.status_command = cfg.status_command.clone();
    next.cursor = cfg.cursor.clone();
    next.exec_once = cfg.exec_once.clone();
    next.exec = cfg.exec.clone();

    // Initialize keyboard layout state from config
    let mut layouts: Vec<KeyboardLayout> = cfg
        .keyboard_layouts
        .iter()
        .map(|c| KeyboardLayout {
            name: c.name.clone(),
            variant: c.variant.clone(),
        })
        .collect();

    if layouts.is_empty() {
        // Fallback to environment variables (standard Wayland convention)
        let layout = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
        if !layout.is_empty() {
            let variant = std::env::var("XKB_DEFAULT_VARIANT").ok();
            layouts.push(KeyboardLayout {
                name: layout,
                variant,
            });
        } else {
            // Last resort: standard US layout
            layouts.push(KeyboardLayout::new("us"));
        }
    }

    let options = cfg
        .keyboard_options
        .clone()
        .or_else(|| std::env::var("XKB_DEFAULT_OPTIONS").ok());
    let model = cfg
        .keyboard_model
        .clone()
        .or_else(|| std::env::var("XKB_DEFAULT_MODEL").ok());

    g.keyboard_layout = KeyboardLayoutState {
        layouts,
        options,
        model,
        swap_escape: cfg.keyboard_swapescape,
        current: 0,
    };

    // Rebuild tag template so monitor creation picks up any config changes.
    next.tag_template = build_tag_template(cfg);
    g.cfg = next;
}

/// Build the canonical tag template from config.
///
/// Returns a `Vec<TagNames>` that every monitor should clone into its own
/// `tags` field via `Monitor::init_tags`.
pub fn build_tag_template(cfg: &crate::config::Config) -> Vec<crate::types::monitor::TagNames> {
    let num_tags = cfg.num_tags;
    let mut template = Vec::with_capacity(num_tags);
    for i in 0..num_tags {
        let name = if i < cfg.tag_names.len() {
            cfg.tag_names[i].clone()
        } else {
            format!("{}", i + 1)
        };
        let alt_name = if i < cfg.tag_alt_names.len() {
            cfg.tag_alt_names[i].clone()
        } else {
            String::new()
        };
        template.push(crate::types::monitor::TagNames { name, alt_name });
    }
    template
}

/// Apply tag configuration to the given `Globals` instance.
pub fn apply_tags_config(g: &mut Globals, cfg: &crate::config::Config) {
    let template = build_tag_template(cfg);
    g.tags.colors = cfg.tag_colors.clone();
    g.tags.num_tags = cfg.num_tags;
    g.cfg.tag_template = template.clone();
    // Initialise any monitors that already exist (re-init on config reload).
    for (_i, mon) in g.monitors.iter_mut() {
        mon.init_tags(&template);
    }
}

impl Globals {
    /// Get the status bar color scheme.
    pub fn status_scheme(&self) -> crate::bar::paint::BarScheme {
        let c = &self.cfg.colors.status_bar;
        crate::bar::paint::BarScheme {
            fg: c.fg,
            bg: c.bg,
            detail: c.detail,
        }
    }

    /// Get the tag hover fill scheme.
    pub fn tag_hover_fill_scheme(&self) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeHover, SchemeTag};

        let colors = self
            .tags
            .colors
            .colors_for(SchemeHover::Hover, SchemeTag::Filled);
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Get the color scheme for a tag.
    pub fn tag_scheme(
        &self,
        m: &Monitor,
        tag_index: u32,
        occupied_tags: TagMask,
        urgent_tags: TagMask,
        is_hover: bool,
    ) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeHover, SchemeTag};

        let tag_num = tag_index as usize + 1;
        let tag_role = if urgent_tags.contains(tag_num) {
            SchemeTag::Urgent
        } else if occupied_tags.contains(tag_num) {
            let selmon = self.selected_monitor();
            let sel_has_tag = selmon
                .sel
                .and_then(|selected_window| {
                    self.clients
                        .get(&selected_window)
                        .map(|c| c.tags.contains(tag_num))
                })
                .unwrap_or(false);

            let is_selected = selmon.num == m.num;

            if is_selected && sel_has_tag {
                SchemeTag::Focus
            } else if m.selected_tags().contains(tag_num) {
                SchemeTag::NoFocus
            } else if !m.showtags {
                SchemeTag::Filled
            } else {
                SchemeTag::Inactive
            }
        } else if m.selected_tags().contains(tag_num) {
            SchemeTag::Empty
        } else {
            SchemeTag::Inactive
        };

        let colors = self.tags.colors.colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            tag_role,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Get the color scheme for a client window.
    pub fn window_scheme(&self, c: &Client, is_hover: bool) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeHover, SchemeWin};

        let selmon = self.selected_monitor();
        let is_selected = selmon.sel == Some(c.win);
        let is_edge_scratchpad = c.is_edge_scratchpad();

        let window_role = if is_selected {
            if is_edge_scratchpad {
                SchemeWin::EdgeScratchpadFocus
            } else if c.is_sticky {
                SchemeWin::StickyFocus
            } else {
                SchemeWin::Focus
            }
        } else if is_edge_scratchpad {
            SchemeWin::EdgeScratchpad
        } else if c.is_sticky {
            SchemeWin::Sticky
        } else if c.is_minimized() {
            SchemeWin::Minimized
        } else if c.is_urgent {
            SchemeWin::Urgent
        } else {
            SchemeWin::Normal
        };

        let colors = self.cfg.colors.window.colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            window_role,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Get the close button color scheme.
    pub fn close_button_scheme(
        &self,
        is_hover: bool,
        is_locked: bool,
        is_fullscreen: bool,
    ) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeClose, SchemeHover};

        let close_role = if is_locked {
            SchemeClose::Locked
        } else if is_fullscreen {
            SchemeClose::Fullscreen
        } else {
            SchemeClose::Normal
        };

        let colors = self.cfg.colors.close_button.colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            close_role,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Reset `current_mode` to `"default"` if it refers to a mode that no longer exists.
    pub fn normalize_current_mode(&mut self) {
        if self.behavior.current_mode == "default"
            || self.behavior.current_mode == crate::overview::OVERVIEW_MODE_NAME
        {
            return;
        }

        if !self
            .cfg
            .bindings
            .modes
            .contains_key(&self.behavior.current_mode)
        {
            self.behavior.current_mode = "default".to_string();
        }
    }
}
