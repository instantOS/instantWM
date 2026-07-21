use crate::client::PendingLaunch;
use crate::config::ModeConfig;
use crate::config::commands::ExternalCommands;
use crate::model::WmModel;
use crate::types::*;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::env;

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
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            border_width_px: 1,
            snap_threshold: 32,
            resize_hints: true,
            decor_hints: false,
        }
    }
}

/// Status bar settings.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BarConfig {
    pub show: bool,
    pub top: bool,
    pub height: i32,
    pub startmenu_size: i32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            show: true,
            top: true,
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

    pub fn targets(&self) -> &[crate::layouts::tree::PlacementTarget] {
        &self.targets
    }

    pub fn selected_target(&self) -> crate::layouts::tree::PlacementTarget {
        // Only the validating constructor and replacement method can create
        // this state, so a selected target always exists.
        self.targets[self.selected]
    }

    pub fn selected_index(&self) -> usize {
        self.selected
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
    pub fn selected_monitor(&self) -> &Monitor {
        self.model.selected_monitor()
    }
    pub fn selected_monitor_mut(&mut self) -> &mut Monitor {
        self.model.selected_monitor_mut()
    }
    pub fn selected_monitor_mut_opt(&mut self) -> Option<&mut Monitor> {
        self.model.selected_monitor_mut_opt()
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
    pub fn attach(&mut self, win: WindowId) {
        self.model.attach(win);
    }
    pub fn detach(&mut self, win: WindowId) {
        self.model.detach(win);
    }
    pub fn attach_z_order_top(&mut self, win: WindowId) {
        self.model.attach_z_order_top(win);
    }
    pub fn detach_z_order(&mut self, win: WindowId) {
        self.model.detach_z_order(win);
    }
    pub fn raise_client_in_z_order(&mut self, win: WindowId) {
        self.model.raise_client_in_z_order(win);
    }

    pub fn normalize_current_mode(&mut self) {
        let mode_exists = match &self.behavior.current_mode {
            ActiveWmMode::Named(name) => self.config.bindings.modes.contains_key(name),
            ActiveWmMode::Default | ActiveWmMode::Overview | ActiveWmMode::TreePlacement(_) => true,
        };
        if !mode_exists {
            self.behavior.current_mode = ActiveWmMode::Default;
        }
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

/// What kind of drag interaction is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragType {
    #[default]
    Move,
    Resize(ResizeDirection),
}

#[derive(Debug, Clone)]
pub struct DragInteraction {
    win: WindowId,
    button: MouseButton,
    drag_type: DragType,
    win_start_geo: Rect,
    start_point: Point,
    last_root_point: Point,
    /// Geometry to restore when the window is re-tiled (e.g. dropped on
    /// the bar).  For windows that were already floating this equals
    /// `win_start_geo`; for tiled windows promoted during the drag it
    /// preserves the saved float dimensions.
    drop_restore_geo: Rect,
    was_focused: bool,
    was_hidden: bool,
    suppress_click_action: bool,
}

impl DragInteraction {
    fn immediate(
        win: WindowId,
        button: MouseButton,
        drag_type: DragType,
        start: Point,
        geo: Rect,
    ) -> Self {
        Self {
            win,
            button,
            drag_type,
            start_point: start,
            win_start_geo: geo,
            drop_restore_geo: geo,
            last_root_point: start,
            was_focused: false,
            was_hidden: false,
            suppress_click_action: false,
        }
    }

    fn armed(params: ArmedDragParams) -> Self {
        Self {
            win: params.win,
            button: params.button,
            drag_type: DragType::Move,
            start_point: params.start,
            win_start_geo: params.geometry,
            drop_restore_geo: params.restore_geometry,
            last_root_point: params.start,
            was_focused: params.was_focused,
            was_hidden: params.was_hidden,
            suppress_click_action: params.suppress_click_action,
        }
    }

    pub fn win(&self) -> WindowId {
        self.win
    }
    pub fn button(&self) -> MouseButton {
        self.button
    }
    pub fn drag_type(&self) -> DragType {
        self.drag_type
    }
    pub fn win_start_geo(&self) -> Rect {
        self.win_start_geo
    }
    pub fn start_point(&self) -> Point {
        self.start_point
    }
    pub fn last_root_point(&self) -> Point {
        self.last_root_point
    }
    pub fn drop_restore_geo(&self) -> Rect {
        self.drop_restore_geo
    }
    pub fn was_focused(&self) -> bool {
        self.was_focused
    }
    pub fn was_hidden(&self) -> bool {
        self.was_hidden
    }
    pub fn suppress_click_action(&self) -> bool {
        self.suppress_click_action
    }

    fn record_motion(&mut self, point: Point) {
        self.last_root_point = point;
    }

    fn activate_as(&mut self, drag_type: DragType, start: Point, geo: Rect) {
        self.drag_type = drag_type;
        self.start_point = start;
        self.last_root_point = start;
        self.win_start_geo = geo;
    }
}

#[derive(Debug, Clone, Default)]
pub enum InteractiveDrag {
    #[default]
    Idle,
    Armed(DragInteraction),
    Active(DragInteraction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragAlreadyActive;

impl std::fmt::Display for DragAlreadyActive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another interactive drag is already armed or active")
    }
}

impl std::error::Error for DragAlreadyActive {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragNotArmed;

impl std::fmt::Display for DragNotArmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no armed drag is available to activate")
    }
}

impl std::error::Error for DragNotArmed {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragCancelReason {
    WindowDestroyed,
    SessionLocked,
    InputDeviceRemoved,
}

#[derive(Debug, Clone, Copy)]
pub struct ArmedDragParams {
    pub win: WindowId,
    pub button: MouseButton,
    pub start: Point,
    pub geometry: Rect,
    pub restore_geometry: Rect,
    pub was_focused: bool,
    pub was_hidden: bool,
    pub suppress_click_action: bool,
}

impl InteractiveDrag {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
    pub fn armed(&self) -> Option<&DragInteraction> {
        match self {
            Self::Armed(drag) => Some(drag),
            _ => None,
        }
    }
    pub fn active(&self) -> Option<&DragInteraction> {
        match self {
            Self::Active(drag) => Some(drag),
            _ => None,
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
    pub last_motion: Option<(Point, u32)>,
    /// The mouse button that started the drag.
    pub button: MouseButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarVolumeDrag {
    button: MouseButton,
    monitor_id: MonitorId,
    anchor_y: i32,
    threshold: i32,
}

impl SidebarVolumeDrag {
    pub fn new(button: MouseButton, monitor_id: MonitorId, anchor_y: i32, threshold: i32) -> Self {
        Self {
            button,
            monitor_id,
            anchor_y,
            threshold: threshold.max(1),
        }
    }

    pub fn button(self) -> MouseButton {
        self.button
    }

    pub fn monitor_id(self) -> MonitorId {
        self.monitor_id
    }

    /// Consume pointer distance and return signed volume steps.
    ///
    /// Positive values mean volume-up. Advancing the anchor only by complete
    /// thresholds preserves sub-threshold movement across input events and
    /// makes the result independent of backend motion-event compression.
    pub fn update(&mut self, root_y: i32) -> i32 {
        let delta = self.anchor_y - root_y;
        let steps = delta / self.threshold;
        self.anchor_y -= steps * self.threshold;
        steps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HotCornerState {
    #[default]
    Ready,
    Latched {
        monitor_id: MonitorId,
    },
}

impl HotCornerState {
    /// Update the hot-corner latch. Returns the monitor whose corner was
    /// entered when the caller should toggle the edge scratchpad.
    pub fn update(
        &mut self,
        monitor_id: Option<MonitorId>,
        inside_activation_zone: bool,
        inside_keep_zone: bool,
    ) -> Option<MonitorId> {
        match *self {
            Self::Ready => {
                let monitor_id = monitor_id.filter(|_| inside_activation_zone)?;
                *self = Self::Latched { monitor_id };
                Some(monitor_id)
            }
            Self::Latched {
                monitor_id: latched_monitor,
            } if monitor_id == Some(latched_monitor) && inside_keep_zone => None,
            Self::Latched { .. } => {
                // Rearm only. Requiring a subsequent sample before activation
                // prevents a single jump between monitor corners from firing
                // two actions at once.
                *self = Self::Ready;
                None
            }
        }
    }
}

#[cfg(test)]
mod pointer_interaction_tests {
    use super::{DragState, HotCornerState, SidebarVolumeDrag};
    use crate::types::{MonitorId, MouseButton};

    #[test]
    fn hot_corner_fires_once_until_pointer_leaves_keep_zone() {
        let monitor = MonitorId::from_raw(7);
        let mut state = HotCornerState::default();

        assert_eq!(state.update(Some(monitor), true, true), Some(monitor));
        assert_eq!(state.update(Some(monitor), true, true), None);
        assert_eq!(state.update(Some(monitor), false, true), None);
        assert_eq!(state.update(Some(monitor), false, false), None);
        assert_eq!(state.update(Some(monitor), true, true), Some(monitor));
    }

    #[test]
    fn hot_corner_rearms_without_firing_when_pointer_jumps_between_monitors() {
        let first = MonitorId::from_raw(1);
        let second = MonitorId::from_raw(2);
        let mut state = HotCornerState::default();

        assert_eq!(state.update(Some(first), true, true), Some(first));
        assert_eq!(state.update(Some(second), true, true), None);
        assert_eq!(state.update(Some(second), true, true), Some(second));
    }

    #[test]
    fn volume_drag_preserves_distance_across_compressed_motion() {
        let mut drag = SidebarVolumeDrag::new(MouseButton::Left, MonitorId::from_raw(3), 500, 30);

        assert_eq!(drag.update(395), 3);
        assert_eq!(drag.update(381), 0);
        assert_eq!(drag.update(379), 1);
    }

    #[test]
    fn volume_drag_handles_direction_reversal_with_residual_distance() {
        let mut drag = SidebarVolumeDrag::new(MouseButton::Left, MonitorId::from_raw(3), 500, 30);

        assert_eq!(drag.update(475), 0);
        assert_eq!(drag.update(510), 0);
        assert_eq!(drag.update(531), -1);
        assert_eq!(drag.update(469), 2);
    }

    #[test]
    fn sidebar_volume_lifecycle_rejects_overlap_and_wrong_button_release() {
        let drag = SidebarVolumeDrag::new(MouseButton::Left, MonitorId::from_raw(3), 500, 30);
        let mut interactions = DragState::default();
        interactions.tag.active = true;
        assert!(interactions.begin_sidebar_volume(drag).is_err());
        assert!(!interactions.sidebar_volume_active());

        interactions.tag.active = false;
        interactions.begin_sidebar_volume(drag).unwrap();
        assert!(!interactions.finish_sidebar_volume(MouseButton::Right));
        assert!(interactions.sidebar_volume_active());
        assert!(interactions.finish_sidebar_volume(MouseButton::Left));
        assert!(!interactions.sidebar_volume_active());
    }
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
    interactive: InteractiveDrag,
    sidebar_volume: Option<SidebarVolumeDrag>,
    pub hover_offer: HoverOffer,
}

impl DragState {
    pub fn interactive(&self) -> &InteractiveDrag {
        &self.interactive
    }

    pub fn active_interaction(&self) -> Option<&DragInteraction> {
        self.interactive.active()
    }

    pub fn armed_interaction(&self) -> Option<&DragInteraction> {
        self.interactive.armed()
    }

    pub fn interaction_button(&self) -> Option<MouseButton> {
        self.active_interaction()
            .or_else(|| self.armed_interaction())
            .map(DragInteraction::button)
    }

    pub fn sidebar_volume_active(&self) -> bool {
        self.sidebar_volume.is_some()
    }

    pub fn sidebar_volume_button(&self) -> Option<MouseButton> {
        self.sidebar_volume.map(SidebarVolumeDrag::button)
    }

    pub fn sidebar_volume_monitor(&self) -> Option<MonitorId> {
        self.sidebar_volume.map(SidebarVolumeDrag::monitor_id)
    }

    pub fn begin_sidebar_volume(
        &mut self,
        drag: SidebarVolumeDrag,
    ) -> Result<(), DragAlreadyActive> {
        if self.any_drag_active() {
            return Err(DragAlreadyActive);
        }
        self.sidebar_volume = Some(drag);
        Ok(())
    }

    pub fn update_sidebar_volume(&mut self, root_y: i32) -> Option<i32> {
        self.sidebar_volume.as_mut().map(|drag| drag.update(root_y))
    }

    pub fn finish_sidebar_volume(&mut self, button: MouseButton) -> bool {
        if self.sidebar_volume_button() != Some(button) {
            return false;
        }
        self.sidebar_volume = None;
        true
    }

    pub fn cancel_sidebar_volume(&mut self) -> bool {
        self.sidebar_volume.take().is_some()
    }

    pub fn begin_move(
        &mut self,
        win: WindowId,
        button: MouseButton,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragAlreadyActive> {
        self.begin_active(DragInteraction::immediate(
            win,
            button,
            DragType::Move,
            start,
            geo,
        ))
    }

    pub fn begin_resize(
        &mut self,
        win: WindowId,
        button: MouseButton,
        dir: ResizeDirection,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragAlreadyActive> {
        self.begin_active(DragInteraction::immediate(
            win,
            button,
            DragType::Resize(dir),
            start,
            geo,
        ))
    }

    fn begin_active(&mut self, drag: DragInteraction) -> Result<(), DragAlreadyActive> {
        if !self.interactive.is_idle() {
            return Err(DragAlreadyActive);
        }
        self.interactive = InteractiveDrag::Active(drag);
        Ok(())
    }

    pub fn arm_title_drag(&mut self, params: ArmedDragParams) -> Result<(), DragAlreadyActive> {
        if !self.interactive.is_idle() {
            return Err(DragAlreadyActive);
        }
        self.interactive = InteractiveDrag::Armed(DragInteraction::armed(params));
        Ok(())
    }

    pub fn activate_armed(
        &mut self,
        drag_type: DragType,
        start: Point,
        geo: Rect,
    ) -> Result<(), DragNotArmed> {
        let mut drag = match std::mem::take(&mut self.interactive) {
            InteractiveDrag::Armed(drag) => drag,
            other => {
                self.interactive = other;
                return Err(DragNotArmed);
            }
        };
        drag.activate_as(drag_type, start, geo);
        self.interactive = InteractiveDrag::Active(drag);
        Ok(())
    }

    pub fn record_interactive_motion(&mut self, point: Point) {
        match &mut self.interactive {
            InteractiveDrag::Armed(drag) | InteractiveDrag::Active(drag) => {
                drag.record_motion(point)
            }
            InteractiveDrag::Idle => {}
        }
    }

    pub fn finish_active(&mut self, button: MouseButton) -> Option<DragInteraction> {
        if !self
            .active_interaction()
            .is_some_and(|drag| drag.button() == button)
        {
            return None;
        }
        match std::mem::take(&mut self.interactive) {
            InteractiveDrag::Active(drag) => Some(drag),
            _ => unreachable!(),
        }
    }

    pub fn finish_armed(&mut self) -> Option<DragInteraction> {
        match std::mem::take(&mut self.interactive) {
            InteractiveDrag::Armed(drag) => Some(drag),
            other => {
                self.interactive = other;
                None
            }
        }
    }

    pub fn cancel_interactive(&mut self) -> Option<DragInteraction> {
        match std::mem::take(&mut self.interactive) {
            InteractiveDrag::Idle => None,
            InteractiveDrag::Armed(drag) | InteractiveDrag::Active(drag) => Some(drag),
        }
    }

    #[inline]
    pub fn any_drag_active(&self) -> bool {
        !self.interactive.is_idle() || self.tag.active || self.sidebar_volume.is_some()
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

    /// Format the currently active layout for status and IPC output.
    pub fn status(&self) -> String {
        if self.is_empty() {
            return "no layouts configured".to_string();
        }
        let current_name = self.current_layout().unwrap_or("unknown");
        let variant = self.current_variant();
        let variant = if variant.is_empty() {
            String::new()
        } else {
            format!(" ({variant})")
        };
        format!(
            "{}/{}: {}{}",
            self.current + 1,
            self.len(),
            current_name,
            variant
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveWmMode {
    Default,
    Overview,
    /// Compositor-owned keyboard placement. Keeping the interaction payload
    /// in the mode makes it impossible for modal input and the advertised WM
    /// mode to disagree.
    TreePlacement(KeyboardTreePlacement),
    Named(String),
}

pub const TREE_PLACEMENT_MODE_NAME: &str = "placement";

impl ActiveWmMode {
    pub fn from_name(name: impl Into<String>) -> Self {
        let name = name.into();
        match name.as_str() {
            "" | "default" => Self::Default,
            crate::overview::OVERVIEW_MODE_NAME => Self::Overview,
            _ => Self::Named(name),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Overview => crate::overview::OVERVIEW_MODE_NAME,
            Self::TreePlacement(_) => TREE_PLACEMENT_MODE_NAME,
            Self::Named(name) => name,
        }
    }

    pub fn tree_placement(&self) -> Option<&KeyboardTreePlacement> {
        match self {
            Self::TreePlacement(state) => Some(state),
            _ => None,
        }
    }

    pub fn tree_placement_mut(&mut self) -> Option<&mut KeyboardTreePlacement> {
        match self {
            Self::TreePlacement(state) => Some(state),
            _ => None,
        }
    }
}

impl From<&str> for ActiveWmMode {
    fn from(name: &str) -> Self {
        Self::from_name(name)
    }
}

impl From<String> for ActiveWmMode {
    fn from(name: String) -> Self {
        Self::from_name(name)
    }
}

#[cfg(test)]
mod active_wm_mode_tests {
    use super::{ActiveWmMode, KeyboardTreePlacement};
    use crate::layouts::tree::{PlacementTarget, Side};
    use crate::types::{MonitorId, Point, TagMask, WindowId};

    #[test]
    fn external_mode_names_are_normalized_into_explicit_states() {
        assert_eq!(ActiveWmMode::from_name(""), ActiveWmMode::Default);
        assert_eq!(ActiveWmMode::from_name("default"), ActiveWmMode::Default);
        assert_eq!(ActiveWmMode::from_name("overview"), ActiveWmMode::Overview);
        assert_eq!(
            ActiveWmMode::from_name("resize"),
            ActiveWmMode::Named("resize".to_string())
        );
    }

    #[test]
    fn keyboard_placement_rejects_an_invalid_selection() {
        let target = PlacementTarget {
            target: WindowId(2),
            side: Some(Side::Left),
            candidate_index: 0,
            position: Point::new(10, 20),
        };
        assert!(
            KeyboardTreePlacement::new(
                WindowId(1),
                MonitorId::default(),
                TagMask::EMPTY,
                vec![target],
                1,
            )
            .is_none()
        );

        let mut placement = KeyboardTreePlacement::new(
            WindowId(1),
            MonitorId::default(),
            TagMask::EMPTY,
            vec![target],
            0,
        )
        .expect("valid selection");
        assert_eq!(placement.selected_target(), target);
        assert!(!placement.select(1));
        assert_eq!(placement.selected_target(), target);
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
    pub specialnext: SpecialNext,
    /// Current active mode (sway-like modes).
    pub current_mode: ActiveWmMode,
}

impl Default for WmBehavior {
    fn default() -> Self {
        Self {
            animated: true,
            focus_follows_mouse: true,
            focus_follows_float_mouse: true,
            requested_cursor: AltCursor::Default,
            specialnext: SpecialNext::None,
            current_mode: ActiveWmMode::Default,
        }
    }
}

impl WmBehavior {
    pub fn toggle_animated(&mut self, action: ToggleAction) {
        action.apply(&mut self.animated);
    }

    pub fn set_special_next(&mut self, value: SpecialNext) {
        self.specialnext = value;
    }

    pub fn toggle_focus_follows_mouse(&mut self, action: ToggleAction) {
        action.apply(&mut self.focus_follows_mouse);
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

impl PendingWork {
    /// Queue applying the monitor configuration.
    pub fn queue_monitor_config_apply(&mut self) {
        self.monitor_config = true;
    }

    /// Queue applying the input configuration.
    pub fn queue_input_config_apply(&mut self) {
        self.input_config = true;
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
    next.bar.top = cfg.top_bar;
    next.bar.height = cfg.bar_height;
    next.window.resize_hints = cfg.resize_hints;
    next.window.decor_hints = cfg.decor_hints;
    next.layout = crate::config::config_toml::LayoutConfig {
        inner_gap: cfg.layout.inner_gap.max(0),
        outer_gap: cfg.layout.outer_gap.max(0),
        smart_gaps: cfg.layout.smart_gaps,
        maximized_gaps: cfg.layout.maximized_gaps,
        keyboard_resize_step: finite_clamp(cfg.layout.keyboard_resize_step, 0.001, 0.5, 0.05),
        minimum_weight: finite_clamp(cfg.layout.minimum_weight, 0.001, 0.49, 0.15),
        pointer_edge_fraction: finite_clamp(cfg.layout.pointer_edge_fraction, 0.05, 0.49, 0.34),
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
