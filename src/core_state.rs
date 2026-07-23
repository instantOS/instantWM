use crate::client::PendingLaunch;
use crate::config::ModeConfig;
use crate::config::commands::ExternalCommands;
use crate::model::WmModel;
use crate::types::*;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::env;

mod interactions;
mod keyboard_state;
pub use interactions::*;
pub use keyboard_state::*;

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
            border_width_px: 1,
            snap_threshold: 32,
            resize_hints: true,
            decor_hints: false,
            raise_floating_on_click: false,
        }
    }
}

/// Status bar settings.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BarConfig {
    pub show: bool,
    pub position: EdgeDirection,
    pub height: i32,
    pub startmenu_size: i32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            show: true,
            position: EdgeDirection::Top,
            height: 0,
            startmenu_size: 0,
        }
    }
}

/// Backend-derived runtime configuration / state.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BackendDerived {
    pub display: DisplayConfig,
    pub bar_height: i32,
    pub bar_horizontal_padding: i32,
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

/// Runtime configuration - composed from sub-structs.
pub struct RuntimeConfig {
    pub derived: BackendDerived,
    pub window: WindowConfig,
    pub bar: BarConfig,
    pub systray: SystrayConfig,
    pub layout: crate::config::config_toml::LayoutConfig,
    pub colors: ColorConfig,
    /// Active built-in colour theme (the base `colors` was derived from).
    pub theme: crate::config::config_toml::ColorTheme,
    pub bindings: BindingConfig,
    pub fonts: FontConfig,
    pub external_commands: ExternalCommands,
    /// Template tag list cloned into every new monitor.
    pub tag_template: Vec<crate::types::monitor::TagNames>,
    pub input: HashMap<String, crate::config::config_toml::InputConfig>,
    pub monitors: HashMap<String, crate::config::config_toml::MonitorConfig>,
    pub status_command: Option<String>,
    pub cursor: crate::config::config_toml::CursorConfig,
    pub exec_once: Vec<String>,
    pub exec: Vec<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            derived: BackendDerived::default(),
            window: WindowConfig::default(),
            bar: BarConfig::default(),
            systray: SystrayConfig::default(),
            layout: crate::config::config_toml::LayoutConfig::default(),
            colors: ColorConfig::default(),
            theme: crate::config::config_toml::ColorTheme::default(),
            bindings: BindingConfig::default(),
            fonts: FontConfig::default(),
            external_commands: crate::config::commands::default_commands(),
            tag_template: Vec::new(),
            input: HashMap::new(),
            monitors: HashMap::new(),
            status_command: None,
            cursor: crate::config::config_toml::CursorConfig::default(),
            exec_once: Vec::new(),
            exec: Vec::new(),
        }
    }
}

/// Backend-neutral state owned by the window manager.
///
/// The authoritative client/monitor/tag graph lives in `model`; configuration
/// and transient interaction state are deliberately kept alongside it rather
/// than inside it. Keeping these categories in one aggregate gives `CoreCtx`
/// a single borrow boundary without mixing backend resources into core state.
#[derive(Default)]
pub struct CoreState {
    pub model: WmModel,
    pub config: RuntimeConfig,
    pub behavior: WmBehavior,
    pub drag: DragState,
    pub hot_corner: HotCornerState,
    pub keyboard_layout: KeyboardLayoutState,
    /// Backend-neutral outer rectangle of the currently previewed manual-tree
    /// placement. Both keyboard and pointer placement project this state.
    pub layout_preview: Option<Rect>,
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
                    && view.client.mode().is_tiling()
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

impl CoreState {
    pub fn selected_win(&self) -> Option<WindowId> {
        self.model.selected_win()
    }
    pub fn selected_monitor_id(&self) -> MonitorId {
        self.model.selected_monitor_id()
    }
    pub fn set_selected_monitor(&mut self, id: MonitorId) {
        self.model.set_selected_monitor(id);
    }
    pub fn selected_monitor(&self) -> Option<&Monitor> {
        self.model.selected_monitor()
    }
    pub fn expect_selected_monitor(&self) -> &Monitor {
        self.model.expect_selected_monitor()
    }
    pub fn expect_selected_monitor_mut(&mut self) -> &mut Monitor {
        self.model.expect_selected_monitor_mut()
    }
    pub fn selected_monitor_mut(&mut self) -> Option<&mut Monitor> {
        self.model.selected_monitor_mut()
    }
    pub fn monitor(&self, id: MonitorId) -> Option<&Monitor> {
        self.model.monitor(id)
    }
    pub fn monitor_mut(&mut self, id: MonitorId) -> Option<&mut Monitor> {
        self.model.monitor_mut(id)
    }
    pub fn monitors_iter(&self) -> impl Iterator<Item = (MonitorId, &Monitor)> {
        self.model.monitors_iter()
    }
    pub fn monitors_iter_all(&self) -> impl Iterator<Item = &Monitor> {
        self.model.monitors_iter_all()
    }
    pub fn monitors_iter_all_mut(&mut self) -> impl Iterator<Item = &mut Monitor> {
        self.model.monitors_iter_all_mut()
    }
    pub fn raise_client_in_z_order(&mut self, win: WindowId) {
        self.model.raise_client_in_z_order(win);
    }
}

impl Clone for RuntimeConfig {
    fn clone(&self) -> Self {
        Self {
            derived: self.derived.clone(),
            window: self.window.clone(),
            bar: self.bar.clone(),
            systray: self.systray.clone(),
            layout: self.layout,
            colors: self.colors.clone(),
            theme: self.theme,
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

/// Runtime behaviour toggles and transient WM mode state.
#[derive(Debug, Clone)]
pub struct WmBehavior {
    pub animated: bool,
    pub focus_follows_mouse: FocusFollowsMouseMode,
    pub focus_follows_float_mouse: bool,
    /// Last WM-owned cursor presentation requested through `set_cursor_style`.
    ///
    /// This is not interaction state. Hover-resize, move/resize drags, and
    /// other input modes must use their explicit state fields as the source of
    /// truth; this field only lets cursor application/reset code avoid treating
    /// the backend cursor as an implicit mode flag.
    pub requested_cursor: AltCursor,
    pub specialnext: SpecialNext,
    /// Current active mode (sway-like modes).
    pub current_mode: ActiveWmMode,
}

impl Default for WmBehavior {
    fn default() -> Self {
        Self {
            animated: true,
            focus_follows_mouse: FocusFollowsMouseMode::Normal,
            focus_follows_float_mouse: true,
            requested_cursor: AltCursor::Default,
            specialnext: SpecialNext::None,
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

    pub fn set_special_next(&mut self, value: SpecialNext) {
        self.specialnext = value;
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
        }
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

/// Build and atomically install runtime configuration.
///
/// The complete value is assembled off to the side so readers never observe
/// a partially-applied reload. Display geometry is backend-derived and is
/// therefore preserved across config replacement.
pub fn apply_config(state: &mut CoreState, cfg: &crate::config::Config) {
    let finite_clamp = |value: f64, minimum: f64, maximum: f64, fallback: f64| {
        if value.is_finite() {
            value.clamp(minimum, maximum)
        } else {
            fallback
        }
    };
    let config = &mut state.config;
    let derived = config.derived.clone();
    let mut next = RuntimeConfig {
        derived,
        ..RuntimeConfig::default()
    };
    next.window.border_width_px = cfg.border_px;
    next.input = cfg.input.clone();
    next.monitors = cfg.monitors.clone();
    next.window.snap_threshold = cfg.snap_threshold;
    next.bar.startmenu_size = cfg.startmenu_size;
    next.systray.pinning = cfg.systray_pinning;
    next.systray.spacing = cfg.systray_spacing;
    next.systray.show = cfg.show_systray;
    next.bar.show = cfg.show_bar;
    next.bar.position = cfg.bar_position;
    next.bar.height = cfg.bar_height;
    next.window.resize_hints = cfg.resize_hints;
    next.window.decor_hints = cfg.decor_hints;
    next.window.raise_floating_on_click = cfg.raise_floating_on_click;
    next.layout = crate::config::config_toml::LayoutConfig {
        inner_gap: cfg.layout.inner_gap.max(0),
        outer_gap: cfg.layout.outer_gap.max(0),
        smart_gaps: cfg.layout.smart_gaps,
        maximized_gaps: cfg.layout.maximized_gaps,
        keyboard_resize_step: finite_clamp(cfg.layout.keyboard_resize_step, 0.001, 0.5, 0.05),
        minimum_weight: finite_clamp(cfg.layout.minimum_weight, 0.001, 0.49, 0.15),
        pointer_edge_fraction: finite_clamp(cfg.layout.pointer_edge_fraction, 0.05, 0.49, 0.34),
        new_window_placement: cfg.layout.new_window_placement,
    };

    next.colors.window = cfg.window_colors.clone();
    next.colors.close_button = cfg.closebuttoncolors.clone();
    next.colors.border = cfg.border_colors;
    next.colors.status_bar = cfg.statusbarcolors;
    next.theme = cfg.theme;

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
        let layout = env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
        if !layout.is_empty() {
            let variant = env::var("XKB_DEFAULT_VARIANT").ok();
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
        .or_else(|| env::var("XKB_DEFAULT_OPTIONS").ok());
    let model = cfg
        .keyboard_model
        .clone()
        .or_else(|| env::var("XKB_DEFAULT_MODEL").ok());

    state.keyboard_layout = KeyboardLayoutState {
        layouts,
        options,
        model,
        swap_escape: cfg.keyboard_swapescape,
        current: 0,
    };

    // Rebuild tag template so monitor creation picks up any config changes.
    next.tag_template = build_tag_template(cfg);
    *config = next;
    apply_tags_config(&mut state.model, &mut state.config, cfg);
}

/// Build the canonical tag template from config.
///
/// Returns a `Vec<TagNames>` that every monitor should clone into its own
/// `tags` field via `Monitor::init_tags`.
pub fn build_tag_template(cfg: &crate::config::Config) -> Vec<crate::types::monitor::TagNames> {
    let num_tags = cfg.num_tags;
    let mut template = Vec::with_capacity(num_tags);
    for i in 0..num_tags {
        let name = cfg
            .tag_names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("{}", i + 1));
        let alt_name = cfg.tag_alt_names.get(i).cloned().unwrap_or_default();
        template.push(crate::types::monitor::TagNames { name, alt_name });
    }
    template
}

/// Apply tag configuration.
fn apply_tags_config(
    model: &mut crate::model::WmModel,
    config: &mut RuntimeConfig,
    cfg: &crate::config::Config,
) {
    let template = build_tag_template(cfg);
    model.tags.colors = cfg.tag_colors.clone();
    model.tags.num_tags = cfg.num_tags;
    config.tag_template = template.clone();
    // Initialise any monitors that already exist (re-init on config reload).
    for (_i, mon) in model.monitors_iter_mut() {
        mon.init_tags(&template);
    }
}
