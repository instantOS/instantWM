use crate::client::PendingLaunch;
use crate::config::ModeConfig;
use crate::config::commands::ExternalCommands;
use crate::model::WmModel;
use crate::types::*;
use std::collections::{BTreeSet, HashMap, VecDeque};

mod hot_corner;
mod interactions;
mod keyboard_state;
mod mode;
pub use hot_corner::*;
pub use interactions::*;
pub use keyboard_state::*;
pub use mode::*;

// ---------------------------------------------------------------------------
// Effective configuration
// ---------------------------------------------------------------------------

/// Display/screen dimensions.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DisplayConfig {
    pub width: i32,
    pub height: i32,
}

/// Window behaviour settings.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowConfig {
    pub border_width_px: i32,
    pub snap_threshold: i32,
    pub resize_hints: bool,
    pub decor_hints: bool,
    /// Raise a floating window when its client area is left-clicked.
    ///
    /// Focus and stacking are otherwise independent; move/resize and bar-title
    /// interactions always raise explicitly.
    pub raise_floating_on_click: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            border_width_px: crate::config::mod_consts::BORDER_PX,
            snap_threshold: 32,
            resize_hints: true,
            decor_hints: true,
            raise_floating_on_click: false,
        }
    }
}

/// Measurements derived from the active backend and rendering resources.
///
/// These values are runtime state rather than user configuration, so config
/// replacement must not copy or preserve them specially.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DerivedState {
    pub display: DisplayConfig,
    pub bar_height: i32,
    pub bar_horizontal_padding: i32,
}

/// Backend presenting the hosted StatusNotifier context menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrayMenuBackend {
    /// Prefer instantMENU when the binary is available, otherwise the bar.
    #[default]
    Auto,
    /// Render the menu inline in the status bar.
    StatusBar,
    /// Delegate the menu to an external instantMENU process.
    InstantMenu,
}

impl TrayMenuBackend {
    /// Lowercase name for serialization and IPC round-trips.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::StatusBar => "statusbar",
            Self::InstantMenu => "instantmenu",
        }
    }
}

/// System tray settings.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SystrayConfig {
    pub show: bool,
    pub pinning: usize,
    pub spacing: i32,
    /// How the tray item context menu is presented.
    pub menu_backend: TrayMenuBackend,
}

impl Default for SystrayConfig {
    fn default() -> Self {
        Self {
            show: true,
            pinning: 0,
            spacing: 0,
            menu_backend: TrayMenuBackend::default(),
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
    pub modes: HashMap<String, ModeConfig>,
    pub buttons: Vec<Button>,
    pub rules: Vec<Rule>,
}

/// Font configuration.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FontConfig {
    pub fonts: Vec<String>,
    pub config_font: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarMetrics {
    pub height: i32,
    pub horizontal_padding: i32,
}

impl FontConfig {
    /// Extract the first positive `size=N` value, falling back to 14 pixels.
    pub fn size(&self) -> f32 {
        self.fonts
            .iter()
            .find_map(|font| {
                let idx = font.find("size=")?;
                let tail = &font[idx + 5..];
                let number: String = tail
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                number.parse::<f32>().ok().filter(|size| *size > 0.0)
            })
            .unwrap_or(14.0)
    }

    /// Return family names stripped of Fontconfig size and style fragments.
    pub fn families(&self) -> Vec<String> {
        self.fonts
            .iter()
            .filter_map(|font| {
                let mut family = font.split(':').next()?.trim();
                for suffix in ["-Regular", "-Medium", "-Bold", "-Light", "-Thin"] {
                    if let Some(stripped) = family.strip_suffix(suffix) {
                        family = stripped;
                        break;
                    }
                }
                (!family.is_empty()).then(|| family.to_string())
            })
            .collect()
    }

    /// Calculate a comfortable line/cell height for the configured font size.
    pub fn line_height(&self) -> i32 {
        let size = self.size();
        ((size * 1.3).ceil() as i32).max(size.ceil() as i32 + 2)
    }

    /// Resolve backend-independent bar geometry from the configured font.
    pub fn bar_metrics(&self, configured_height: i32) -> BarMetrics {
        let font_height = self.line_height();
        let min_height = crate::types::CLOSE_BUTTON_WIDTH + crate::types::CLOSE_BUTTON_DETAIL + 2;
        let height = if configured_height > 0 {
            configured_height.max(min_height)
        } else {
            (font_height + 12).max(min_height)
        };
        BarMetrics {
            height,
            horizontal_padding: font_height,
        }
    }

    /// Xft interprets `size` as points, whereas the shared config defines it
    /// in pixels. Convert only the size property and preserve every other
    /// Fontconfig pattern fragment.
    pub fn xft_pixel_patterns(&self) -> Vec<String> {
        self.fonts
            .iter()
            .map(|font| {
                font.split(':')
                    .map(|part| {
                        part.strip_prefix("size=")
                            .map_or_else(|| part.to_string(), |size| format!("pixelsize={size}"))
                    })
                    .collect::<Vec<_>>()
                    .join(":")
            })
            .collect()
    }
}

#[cfg(test)]
mod tray_menu_backend_tests {
    use super::{SystrayConfig, TrayMenuBackend};

    #[test]
    fn backend_names_round_trip_through_serde() {
        for (backend, name) in [
            (TrayMenuBackend::Auto, "auto"),
            (TrayMenuBackend::StatusBar, "statusbar"),
            (TrayMenuBackend::InstantMenu, "instantmenu"),
        ] {
            assert_eq!(backend.as_str(), name);
            assert_eq!(
                serde_json::to_string(&backend).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(
                serde_json::from_str::<TrayMenuBackend>(&format!("\"{name}\"")).unwrap(),
                backend
            );
        }
    }

    #[test]
    fn default_backend_is_auto_and_partial_sections_keep_defaults() {
        assert_eq!(SystrayConfig::default().menu_backend, TrayMenuBackend::Auto);
        let config: SystrayConfig = toml::from_str("menu_backend = \"statusbar\"\n").unwrap();
        assert_eq!(config.menu_backend, TrayMenuBackend::StatusBar);
        assert!(config.show, "show keeps its non-derive default");
    }
}

#[cfg(test)]
mod font_config_tests {
    use super::FontConfig;
    #[test]
    fn bar_metrics_are_shared_and_respect_the_visual_minimum() {
        let fonts = FontConfig {
            fonts: vec!["Inter-Regular:size=12".to_string()],
            ..FontConfig::default()
        };

        let automatic = fonts.bar_metrics(0);
        assert_eq!(automatic.horizontal_padding, fonts.line_height());
        assert_eq!(automatic.height, fonts.line_height() + 12);

        let too_small = fonts.bar_metrics(1);
        assert_eq!(
            too_small.height,
            crate::types::CLOSE_BUTTON_WIDTH + crate::types::CLOSE_BUTTON_DETAIL + 2
        );
    }

    #[test]
    fn xft_patterns_preserve_pixel_sized_shared_font_semantics() {
        let fonts = FontConfig {
            fonts: vec![
                "Inter-Regular:size=12:style=Bold".to_string(),
                "Symbols Nerd Font:pixelsize=15".to_string(),
            ],
            ..FontConfig::default()
        };

        assert_eq!(
            fonts.xft_pixel_patterns(),
            [
                "Inter-Regular:pixelsize=12:style=Bold",
                "Symbols Nerd Font:pixelsize=15"
            ]
        );
    }
}

/// Fully resolved, validated configuration consumed by core and backends.
#[derive(Clone)]
pub struct EffectiveConfig {
    pub window: WindowConfig,
    pub bar: crate::config::config_toml::BarConfig,
    pub systray: SystrayConfig,
    pub layout: crate::config::config_toml::LayoutConfig,
    pub animations: crate::config::config_toml::AnimationConfig,
    pub colors: ColorConfig,
    /// Active built-in colour theme (the base `colors` was derived from).
    pub theme: crate::config::config_toml::ColorTheme,
    pub bindings: BindingConfig,
    pub fonts: FontConfig,
    pub external_commands: ExternalCommands,
    /// Template tag list cloned into every new monitor.
    pub tag_template: Vec<crate::types::Tag>,
    pub tag_colors: TagColorConfigs,
    /// Resolved keyboard settings. The current layout index remains runtime
    /// interaction state in [`KeyboardLayoutState`].
    pub keyboard: crate::config::config_toml::KeyboardConfig,
    pub input: HashMap<String, crate::config::config_toml::InputConfig>,
    pub monitors: HashMap<String, crate::config::config_toml::MonitorConfig>,
    pub status_command: Option<String>,
    pub cursor: crate::config::config_toml::CursorConfig,
    pub exec_once: Vec<String>,
    pub exec: Vec<String>,
}

impl Default for EffectiveConfig {
    fn default() -> Self {
        crate::config::default_config(crate::backend::BackendKind::Wayland)
    }
}

/// Backend-neutral state owned by the window manager.
///
/// The authoritative client/monitor/tag graph lives in `model`; configuration
/// and transient interaction state are deliberately kept alongside it rather
/// than inside it. Keeping these categories in one aggregate gives `CoreCtx`
/// a single borrow boundary without mixing backend resources into core state.
/// Ephemeral pointer/keyboard/outline state that changes at input frequency.
///
/// Grouping it separately from `model`/`config`/`derived` makes the
/// god-object boundary explicit: `model` is the persistent client graph,
/// `interaction` is per-frame input state. New transient fields (e.g. future
/// gesture previews) belong here, not as flat `CoreState` fields.
#[derive(Default, Debug, Clone)]
pub struct InteractionState {
    pub drag: DragState,
    pub hot_corner: HotCornerState,
    pub keyboard_layout: KeyboardLayoutState,
    /// Backend-neutral outer rectangle of the active compositor interaction
    /// outline (manual-tree placement or destructive overview gesture).
    pub layout_preview: Option<Rect>,
    pub layout_preview_style: InteractionOutlineStyle,
    /// Lazily solved trigger zones for the active pointer tree-placement drag.
    /// Authoritative arrange passes invalidate the cached layout/constraints;
    /// source, monitor, tag view, and edge policy are checked before reuse.
    pub(crate) pointer_placement_cache:
        Option<crate::layouts::manager::PointerPlacementPreviewCache>,
}

#[derive(Default)]
pub struct CoreState {
    pub model: WmModel,
    pub config: EffectiveConfig,
    pub derived: DerivedState,
    pub behavior: WmBehavior,
    pub interaction: InteractionState,
    pub pending_launches: VecDeque<PendingLaunch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardTreePlacement {
    pub source: WindowId,
    /// The tree is owned by this exact monitor/tag view. Capturing both keeps
    /// a modal placement from being applied to a different tree after a view
    /// or monitor change.
    pub monitor_id: MonitorId,
    pub tags: TagMask,
    targets: Vec<crate::layouts::tree::PlacementTarget>,
    selected: usize,
}

impl KeyboardTreePlacement {
    pub fn new(
        source: WindowId,
        monitor_id: MonitorId,
        tags: TagMask,
        targets: Vec<crate::layouts::tree::PlacementTarget>,
        selected: usize,
    ) -> Option<Self> {
        targets.get(selected)?;
        Some(Self {
            source,
            monitor_id,
            tags,
            targets,
            selected,
        })
    }

    pub fn new_nearest(
        source: WindowId,
        monitor_id: MonitorId,
        tags: TagMask,
        targets: Vec<crate::layouts::tree::PlacementTarget>,
        point: Point,
    ) -> Option<Self> {
        let selected = Self::nearest_target_index(&targets, point);
        Self::new(source, monitor_id, tags, targets, selected)
    }

    pub fn targets(&self) -> &[crate::layouts::tree::PlacementTarget] {
        &self.targets
    }

    pub fn selected_target(&self) -> crate::layouts::tree::PlacementTarget {
        // Only the validating constructor and replacement method can create
        // this state, so a selected target always exists.
        self.targets[self.selected]
    }

    /// Whether the monitor/tag/tree context captured at entry is still the
    /// authoritative context in which this session may operate.
    pub fn is_current_for(&self, model: &WmModel) -> bool {
        if model.selected_monitor_id() != self.monitor_id {
            return false;
        }
        let monitor = model.expect_selected_monitor();
        monitor.selected_tags() == self.tags
            && model.client_view(self.source).is_some_and(|view| {
                view.monitor.id() == self.monitor_id
                    && view.client.mode().is_normal_tiling()
                    && view.client.is_visible(self.tags)
            })
            && monitor
                .per_tag()
                .is_some_and(|tag| tag.layout_tree.leaves().contains(&self.source))
    }

    fn nearest_target_index(
        targets: &[crate::layouts::tree::PlacementTarget],
        point: Point,
    ) -> usize {
        targets
            .iter()
            .enumerate()
            .min_by_key(|(_, target)| {
                let dx = i64::from(target.position.x - point.x);
                let dy = i64::from(target.position.y - point.y);
                dx * dx + dy * dy
            })
            .map_or(0, |(index, _)| index)
    }

    /// Select the best candidate lying visually in `side` from the current
    /// candidate. At a visual edge, wrap to the opposite edge and use
    /// cross-axis alignment to break ties. This keeps a
    /// directional key productive without requiring users to understand the
    /// exact placement-target topology.
    pub fn select_direction(&mut self, side: crate::layouts::tree::Side) -> bool {
        let current = self.selected_target().position;
        let selected = self.selected;
        let candidates = || {
            self.targets
                .iter()
                .enumerate()
                .filter(move |(index, _)| *index != selected)
        };
        let next = candidates()
            .filter_map(|(index, target)| {
                let (primary, cross) = directional_distances(current, target.position, side);
                if primary <= 0 {
                    return None;
                }
                let score = primary
                    .saturating_add(cross.saturating_mul(2))
                    .saturating_add(cross.saturating_mul(cross) / (primary + 1));
                Some((index, score))
            })
            .min_by_key(|(index, score)| (*score, *index))
            .map(|(index, _)| index)
            .or_else(|| {
                // No target lies farther in the requested direction. Treat
                // that as an edge and wrap to the far side. Candidates on the
                // opposite edge are preferred first, then the one closest to
                // the current cross-axis lane, so repeated presses traverse
                // the complete spatial ordering instead of getting trapped.
                let opposite_edge = candidates()
                    .map(|(_, target)| directional_coordinate(target.position, side))
                    .min()?;
                candidates()
                    .map(|(index, target)| {
                        let coordinate = directional_coordinate(target.position, side);
                        let cross = cross_axis_distance(current, target.position, side);
                        let depth = coordinate - opposite_edge;
                        (index, cross, depth)
                    })
                    .min_by_key(|(index, cross, depth)| (*depth, *cross, *index))
                    .map(|(index, _, _)| index)
            });
        next.is_some_and(|index| self.select(index))
    }

    pub fn select_center_of_current_window(&mut self) -> bool {
        let window = self.selected_target().target;
        let Some(index) = self
            .targets
            .iter()
            .position(|target| target.target == window && target.side.is_none())
        else {
            return false;
        };
        self.select(index)
    }

    pub fn select(&mut self, selected: usize) -> bool {
        if selected >= self.targets.len() {
            return false;
        }
        self.selected = selected;
        true
    }

    pub fn cycle(&mut self, backwards: bool) {
        let len = self.targets.len();
        self.selected = if backwards {
            (self.selected + len - 1) % len
        } else {
            (self.selected + 1) % len
        };
    }

    pub fn replace_targets(
        &mut self,
        targets: Vec<crate::layouts::tree::PlacementTarget>,
        selected: usize,
    ) -> bool {
        if targets.get(selected).is_none() {
            return false;
        }
        self.targets = targets;
        self.selected = selected;
        true
    }

    pub fn replace_targets_near(
        &mut self,
        targets: Vec<crate::layouts::tree::PlacementTarget>,
        point: Point,
    ) -> bool {
        let selected = Self::nearest_target_index(&targets, point);
        self.replace_targets(targets, selected)
    }
}

fn directional_distances(
    current: Point,
    candidate: Point,
    side: crate::layouts::tree::Side,
) -> (i64, i64) {
    let dx = i64::from(candidate.x) - i64::from(current.x);
    let dy = i64::from(candidate.y) - i64::from(current.y);
    match side {
        crate::layouts::tree::Side::Left => (-dx, dy.abs()),
        crate::layouts::tree::Side::Right => (dx, dy.abs()),
        crate::layouts::tree::Side::Top => (-dy, dx.abs()),
        crate::layouts::tree::Side::Bottom => (dy, dx.abs()),
    }
}

fn directional_coordinate(point: Point, side: crate::layouts::tree::Side) -> i64 {
    match side {
        crate::layouts::tree::Side::Left => -i64::from(point.x),
        crate::layouts::tree::Side::Right => i64::from(point.x),
        crate::layouts::tree::Side::Top => -i64::from(point.y),
        crate::layouts::tree::Side::Bottom => i64::from(point.y),
    }
}

fn cross_axis_distance(current: Point, candidate: Point, side: crate::layouts::tree::Side) -> i64 {
    match side {
        crate::layouts::tree::Side::Left | crate::layouts::tree::Side::Right => {
            (i64::from(candidate.y) - i64::from(current.y)).abs()
        }
        crate::layouts::tree::Side::Top | crate::layouts::tree::Side::Bottom => {
            (i64::from(candidate.x) - i64::from(current.x)).abs()
        }
    }
}

/// Runtime behaviour toggles and transient WM mode state.
#[derive(Debug, Clone)]
pub struct WmBehavior {
    pub animated: bool,
    pub focus_follows_mouse: FocusFollowsMouseMode,
    pub focus_follows_float_mouse: bool,
    /// Runtime-added one-shot rules waiting to match the next matching window.
    /// Each entry has an absolute deadline; see [`crate::client::rules`] for
    /// the matching/consumption path.
    pub pending_tmp_rules: Vec<crate::types::PendingTmpRule>,
    /// Current active mode (sway-like modes).
    pub current_mode: ActiveWmMode,
}

impl Default for WmBehavior {
    fn default() -> Self {
        Self {
            animated: true,
            focus_follows_mouse: FocusFollowsMouseMode::Normal,
            focus_follows_float_mouse: true,
            pending_tmp_rules: Vec::new(),
            current_mode: ActiveWmMode::Default,
        }
    }
}

impl WmBehavior {
    pub fn normalize_current_mode(&mut self, modes: &HashMap<String, ModeConfig>) {
        let mode_exists = match &self.current_mode {
            ActiveWmMode::Named(name) => modes.contains_key(name),
            ActiveWmMode::Default | ActiveWmMode::Overview | ActiveWmMode::TreePlacement(_) => true,
        };
        if !mode_exists {
            self.current_mode = ActiveWmMode::Default;
        }
    }

    pub fn toggle_animated(&mut self, action: ToggleAction) {
        action.apply(&mut self.animated);
    }

    pub fn set_focus_follows_mouse(&mut self, mode: FocusFollowsMouseMode) {
        self.focus_follows_mouse = mode;
    }

    pub fn toggle_focus_follows_float_mouse(&mut self, action: ToggleAction) {
        action.apply(&mut self.focus_follows_float_mouse);
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
    /// Whether the Wayland cursor theme or size needs to be reloaded.
    pub cursor_config: bool,
    /// Pending layout work.
    pub layout: PendingLayoutWork,
    /// Newly managed windows waiting for their first authoritative arrange
    /// before the one-time spawn transition can be started.
    pub(crate) spawn_animations: BTreeSet<WindowId>,
    /// Edge scratchpads whose slide-out animation is still playing.
    ///
    /// The logical hide (conceal, focus hand-off, layout) is deferred until
    /// the backend reports the exit animation finished, so the overlay slides
    /// out instead of vanishing on the first frame.
    pending_scratchpad_hides: BTreeSet<WindowId>,
}

impl Default for PendingWork {
    fn default() -> Self {
        let mut layout = PendingLayoutWork::default();
        layout.mark_all();
        Self {
            input_config: false,
            monitor_config: false,
            cursor_config: false,
            layout,
            spawn_animations: BTreeSet::new(),
            pending_scratchpad_hides: BTreeSet::new(),
        }
    }
}

impl PendingWork {
    /// Track an edge scratchpad whose logical hide waits for its exit animation.
    pub fn queue_pending_scratchpad_hide(&mut self, win: WindowId) {
        self.pending_scratchpad_hides.insert(win);
    }

    /// Whether a deferred hide is waiting on this window's exit animation.
    pub fn has_pending_scratchpad_hide(&self, win: WindowId) -> bool {
        self.pending_scratchpad_hides.contains(&win)
    }

    /// Cancel a tracked pending hide (the scratchpad is being shown again).
    pub fn cancel_pending_scratchpad_hide(&mut self, win: WindowId) {
        self.pending_scratchpad_hides.remove(&win);
    }

    /// Snapshot the windows with a deferred hide awaiting their animation.
    pub fn pending_scratchpad_hide_windows(&self) -> Vec<WindowId> {
        self.pending_scratchpad_hides.iter().copied().collect()
    }
}

impl PendingWork {
    /// Queue applying the monitor configuration.
    pub fn queue_monitor_config_apply(&mut self) {
        self.monitor_config = true;
    }

    /// Queue applying the input configuration.
    pub fn queue_input_config_apply(&mut self) {
        self.input_config = true;
    }

    /// Queue reloading the Wayland cursor theme and size.
    pub fn queue_cursor_config_apply(&mut self) {
        self.cursor_config = true;
    }
}

/// Atomically install a fully resolved configuration.
///
/// Parsing, default resolution and validation happen before this boundary.
/// Installation only replaces policy and synchronizes model/interaction state
/// that intentionally mirrors part of that policy.
pub fn apply_config(state: &mut CoreState, next: EffectiveConfig) {
    let keyboard_layout = KeyboardLayoutState {
        layouts: next.keyboard.layouts.clone(),
        options: next.keyboard.options.clone(),
        model: next.keyboard.model.clone(),
        swap_escape: next.keyboard.swapescape,
        current: 0,
    };
    let show_bottom_bar = next.bar.show_bottom;
    let tag_template = next.tag_template.clone();
    let tag_colors = next.tag_colors.clone();

    state.config = next;
    state.interaction.keyboard_layout = keyboard_layout;
    state.model.tags.colors = tag_colors;
    state.model.tags.num_tags = tag_template.len();

    // The file setting is global. Reloading it resets interactive per-tag
    // overrides so existing outputs immediately match newly created outputs.
    for (_id, monitor) in state.model.monitors_iter_mut() {
        monitor.show_bottom_bar = show_bottom_bar;
        for per_tag in monitor.per_tag.values_mut() {
            per_tag.show_bottom_bar = show_bottom_bar;
        }
        monitor.init_tags(&tag_template);
    }
}
