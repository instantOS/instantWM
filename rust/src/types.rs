use std::fmt::Debug;
use std::rc::Rc;

use x11rb::protocol::xproto::Window;

use crate::contexts::WmCtx;
use crate::drw::Clr;

/// X11 atom identifier (protocol type is CARDINAL / 32-bit).
pub type Atom = u32;

pub const MAX_TAGS: usize = 21;
pub const SCRATCHPAD_TAG: usize = 20;
pub const SCRATCHPAD_MASK: u32 = 1 << SCRATCHPAD_TAG;
pub const SCRATCHPAD_NAME_LEN: usize = 64;

pub const BUTTONMASK: u32 = 1 << 2 | 1 << 3;
pub const MOUSEMASK: u32 = BUTTONMASK | 1 << 6;

pub const CLOSE_BUTTON_WIDTH: i32 = 20;
pub const CLOSE_BUTTON_HEIGHT: i32 = 16;
pub const CLOSE_BUTTON_DETAIL: i32 = 4;
pub const CLOSE_BUTTON_HIT_WIDTH: i32 = 32;
pub const RESIZE_WIDGET_WIDTH: i32 = 30;

pub const SIDEBAR_WIDTH: i32 = 50;
pub const OVERLAY_ACTIVATION_ZONE: i32 = 20;
pub const OVERLAY_KEEP_ZONE_X: i32 = 40;
pub const OVERLAY_KEEP_ZONE_Y: i32 = 30;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorScheme {
    pub fg: Clr,
    pub bg: Clr,
    pub detail: Clr,
}

impl ColorScheme {
    pub fn new(fg: Clr, bg: Clr, detail: Clr) -> Self {
        Self { fg, bg, detail }
    }

    pub fn from_vec(vec: Vec<Clr>) -> Option<Self> {
        if vec.len() >= 3 {
            Some(Self {
                fg: vec[0].clone(),
                bg: vec[1].clone(),
                detail: vec[2].clone(),
            })
        } else {
            None
        }
    }

    pub fn as_vec(&self) -> Vec<Clr> {
        vec![self.fg.clone(), self.bg.clone(), self.detail.clone()]
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BorderScheme {
    pub normal: ColorScheme,
    pub tile_focus: ColorScheme,
    pub float_focus: ColorScheme,
    pub snap: ColorScheme,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatusScheme {
    pub fg: Clr,
    pub bg: Clr,
    pub detail: Clr,
}

impl StatusScheme {
    pub fn new(fg: Clr, bg: Clr, detail: Clr) -> Self {
        Self { fg, bg, detail }
    }

    pub fn as_color_scheme(&self) -> ColorScheme {
        ColorScheme {
            fg: self.fg.clone(),
            bg: self.bg.clone(),
            detail: self.detail.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagSchemes {
    pub no_hover: Vec<ColorScheme>,
    pub hover: Vec<ColorScheme>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowSchemes {
    pub no_hover: Vec<ColorScheme>,
    pub hover: Vec<ColorScheme>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonSchemes {
    pub no_hover: Vec<ColorScheme>,
    pub hover: Vec<ColorScheme>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorSchemeStrings {
    pub fg: &'static str,
    pub bg: &'static str,
    pub detail: &'static str,
}

impl ColorSchemeStrings {
    pub fn new(fg: &'static str, bg: &'static str, detail: &'static str) -> Self {
        Self { fg, bg, detail }
    }

    pub fn to_vec(&self) -> Vec<&'static str> {
        vec![self.fg, self.bg, self.detail]
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TagColorConfigs {
    pub no_hover: Vec<ColorSchemeStrings>,
    pub hover: Vec<ColorSchemeStrings>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowColorConfigs {
    pub no_hover: Vec<ColorSchemeStrings>,
    pub hover: Vec<ColorSchemeStrings>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CloseButtonColorConfigs {
    pub no_hover: Vec<ColorSchemeStrings>,
    pub hover: Vec<ColorSchemeStrings>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BorderColorConfig {
    pub normal: ColorSchemeStrings,
    pub tile_focus: ColorSchemeStrings,
    pub float_focus: ColorSchemeStrings,
    pub snap: ColorSchemeStrings,
}

impl Default for BorderColorConfig {
    fn default() -> Self {
        Self {
            normal: ColorSchemeStrings::new("", "", ""),
            tile_focus: ColorSchemeStrings::new("", "", ""),
            float_focus: ColorSchemeStrings::new("", "", ""),
            snap: ColorSchemeStrings::new("", "", ""),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusColorConfig {
    pub colors: ColorSchemeStrings,
}

impl Default for StatusColorConfig {
    fn default() -> Self {
        Self {
            colors: ColorSchemeStrings::new("", "", ""),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Normal,
    Resize,
    Move,
    Click,
    Hor,
    Vert,
    TL,
    TR,
    BL,
    BR,
}

/// Named struct for WM protocol atoms (replaces `wmatom: [u32; 4]`)
#[derive(Debug, Clone, Copy, Default)]
pub struct WmAtoms {
    pub protocols: u32,
    pub delete: u32,
    pub state: u32,
    pub take_focus: u32,
}

/// Named struct for EWMH / NET atoms (replaces `netatom: [u32; 14]`)
#[derive(Debug, Clone, Copy, Default)]
pub struct NetAtoms {
    pub active_window: u32,
    pub supported: u32,
    pub system_tray: u32,
    pub system_tray_op: u32,
    pub system_tray_orientation: u32,
    pub system_tray_orientation_horz: u32,
    pub wm_name: u32,
    pub wm_state: u32,
    pub wm_check: u32,
    pub wm_fullscreen: u32,
    pub wm_window_type: u32,
    pub wm_window_type_dialog: u32,
    pub client_list: u32,
    pub client_info: u32,
}

/// Named struct for XEmbed / ICCCM atoms (replaces `xatom: [u32; 3]`)
#[derive(Debug, Clone, Copy, Default)]
pub struct XAtoms {
    pub manager: u32,
    pub xembed: u32,
    pub xembed_info: u32,
}

/// All tag-related configuration and runtime state, grouped in one place.
#[derive(Debug, Clone, Default)]
pub struct TagSet {
    /// List of tags with their properties.
    pub tags: Vec<Tag>,
    /// Raw colour strings from config/xresources, indexed [hover_state][type][colour_index].
    pub colors: Vec<Vec<Vec<&'static str>>>,
    /// Compiled colour objects derived from `colors`.
    pub schemes: TagSchemes,
    /// Whether to display `alt_names` instead of `names`.
    pub show_alt: bool,
    /// Prefix-key mode: next tag key toggles rather than views.
    pub prefix: bool,
    /// Cached pixel width of the tag strip in the bar.
    pub width: i32,
}

impl TagSet {
    /// Bitmask covering all active tags: `(1 << count) - 1`.
    #[inline]
    pub fn mask(&self) -> u32 {
        (1u32 << self.tags.len()).wrapping_sub(1)
    }

    /// Number of active tags.
    #[inline]
    pub fn count(&self) -> usize {
        self.tags.len()
    }
}

/// Stores layout state for a tag with last-used tracking.
///
/// Each tag maintains its current layout and remembers the previously used layout,
/// enabling `restore_last_layout()` functionality. The primary/secondary slot system
/// is internal and managed automatically.
#[derive(Debug, Clone, Copy)]
pub struct TagLayouts {
    primary: crate::layouts::LayoutKind,
    secondary: crate::layouts::LayoutKind,
    active_slot: LayoutSlot,
    last_layout: Option<crate::layouts::LayoutKind>,
}

impl Default for TagLayouts {
    fn default() -> Self {
        use crate::layouts::LayoutKind;
        Self {
            primary: LayoutKind::Tile,
            secondary: LayoutKind::Floating,
            active_slot: LayoutSlot::default(),
            last_layout: None,
        }
    }
}

impl TagLayouts {
    /// Get the currently active layout.
    pub fn get_layout(self) -> crate::layouts::LayoutKind {
        match self.active_slot {
            LayoutSlot::Primary => self.primary,
            LayoutSlot::Secondary => self.secondary,
        }
    }

    /// Set a new layout on the active slot, saving the current one to `last_layout`.
    /// If the new layout matches the current one, this is a no-op.
    pub fn set_layout(&mut self, layout: crate::layouts::LayoutKind) {
        let current = self.get_layout();
        if current == layout {
            return;
        }
        self.last_layout = Some(current);
        match self.active_slot {
            LayoutSlot::Primary => self.primary = layout,
            LayoutSlot::Secondary => self.secondary = layout,
        }
    }

    /// Swap the current layout with the last used layout.
    /// Returns true if a swap occurred, false if no last layout was stored.
    pub fn restore_last_layout(&mut self) -> bool {
        let current = self.get_layout();
        let last = self.last_layout.take();

        match last {
            Some(last) => {
                self.last_layout = Some(current);
                match self.active_slot {
                    LayoutSlot::Primary => self.primary = last,
                    LayoutSlot::Secondary => self.secondary = last,
                }
                true
            }
            None => false,
        }
    }

    /// Returns true if the current layout is a tiling layout.
    pub fn is_tiling(self) -> bool {
        self.get_layout().is_tiling()
    }

    /// Returns true if the current layout is a monocle layout.
    pub fn is_monocle(self) -> bool {
        self.get_layout().is_monocle()
    }

    /// Returns true if the current layout is an overview layout.
    pub fn is_overview(self) -> bool {
        self.get_layout().is_overview()
    }

    /// Get the symbol of the current layout.
    pub fn symbol(self) -> &'static str {
        self.get_layout().symbol()
    }

    /// Toggle between primary and secondary slots.
    /// Saves current layout to last_layout before toggling.
    pub fn toggle_slot(&mut self) {
        self.last_layout = Some(self.get_layout());
        self.active_slot = self.active_slot.toggle();
    }
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub alt_name: &'static str,
    /// Number of clients in the master area for tiling layouts.
    pub nmaster: i32,
    /// Master factor for tiling layouts (0.0 to 1.0).
    pub mfact: f32,
    /// Whether to show the bar on this tag.
    pub showbar: bool,
    /// The layouts for this tag (primary and secondary).
    pub layouts: TagLayouts,
}

impl Default for Tag {
    fn default() -> Self {
        Self {
            name: String::new(),
            alt_name: "",
            nmaster: 1,
            mfact: 0.55,
            showbar: true,
            layouts: TagLayouts::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    TagBar,
    LtSymbol,
    StatusText,
    WinTitle,
    ClientWin,
    RootWin,
    CloseButton,
    ShutDown,
    SideBar,
    StartMenu,
    ResizeWidget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left = 1,
    Middle = 2,
    Right = 3,
    ScrollUp = 4,
    ScrollDown = 5,
}

impl MouseButton {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Left),
            2 => Some(Self::Middle),
            3 => Some(Self::Right),
            4 => Some(Self::ScrollUp),
            5 => Some(Self::ScrollDown),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AltCursor {
    #[default]
    None,
    Resize,
    //TODO: Port over sidebar from C codebase
    Sidebar,
}

/// Identifies which layout slot (primary or secondary) is currently active for a tag.
///
/// Each tag maintains two layout slots that can be toggled between. This allows users
/// to quickly switch between two different layouts (e.g., tiling and floating) without
/// cycling through all available layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutSlot {
    /// The primary layout slot (index 0).
    #[default]
    Primary,
    /// The secondary layout slot (index 1).
    Secondary,
}

impl LayoutSlot {
    /// Convert to a usize index (0 for Primary, 1 for Secondary).
    pub const fn as_index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Secondary => 1,
        }
    }

    /// Create a LayoutSlot from a usize index.
    /// Returns None if the index is not 0 or 1.
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Primary),
            1 => Some(Self::Secondary),
            _ => None,
        }
    }

    /// Toggle between Primary and Secondary.
    pub const fn toggle(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Primary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapPosition {
    #[default]
    None,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
    Maximized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeDirection {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl ResizeDirection {
    pub fn cursor_index(self) -> usize {
        match self {
            Self::TopLeft => 8,
            Self::Top => 4,
            Self::TopRight => 9,
            Self::Right => 5,
            Self::BottomRight => 7,
            Self::Bottom => 4,
            Self::BottomLeft => 6,
            Self::Left => 5,
        }
    }

    pub fn affected_edges(self) -> (bool, bool, bool, bool) {
        match self {
            Self::TopLeft => (true, false, true, false),
            Self::Top => (false, false, true, false),
            Self::TopRight => (false, true, true, false),
            Self::Right => (false, true, false, false),
            Self::BottomRight => (false, true, false, true),
            Self::Bottom => (false, false, false, true),
            Self::BottomLeft => (true, false, false, true),
            Self::Left => (true, false, false, false),
        }
    }

    pub fn warp_offset(self, w: i32, h: i32, bw: i32) -> (i32, i32) {
        match self {
            Self::TopLeft => (-bw, -bw),
            Self::Top => ((w + bw - 1) / 2, -bw),
            Self::TopRight => (w + bw - 1, -bw),
            Self::Right => (w + bw - 1, (h + bw - 1) / 2),
            Self::BottomRight => (w + bw - 1, h + bw - 1),
            Self::Bottom => ((w + bw - 1) / 2, h + bw - 1),
            Self::BottomLeft => (-bw, h + bw - 1),
            Self::Left => (-bw, (h + bw - 1) / 2),
        }
    }
}

pub fn get_resize_direction(w: i32, h: i32, hit_x: i32, hit_y: i32) -> ResizeDirection {
    if hit_y > h / 2 {
        if hit_x < w / 3 {
            if hit_y < 2 * h / 3 {
                ResizeDirection::Left
            } else {
                ResizeDirection::BottomLeft
            }
        } else if hit_x > 2 * w / 3 {
            if hit_y < 2 * h / 3 {
                ResizeDirection::Right
            } else {
                ResizeDirection::BottomRight
            }
        } else {
            ResizeDirection::Bottom
        }
    } else if hit_x < w / 3 {
        if hit_y > h / 3 {
            ResizeDirection::Left
        } else {
            ResizeDirection::TopLeft
        }
    } else if hit_x > 2 * w / 3 {
        if hit_y > h / 3 {
            ResizeDirection::Right
        } else {
            ResizeDirection::TopRight
        }
    } else {
        ResizeDirection::Top
    }
}

/// The side of the screen from which the overlay window slides in/out.
///
/// Mirrors the `OverlayTop` / `OverlayRight` / `OverlayBottom` / `OverlayLeft`
/// constants from the C codebase and is stored on [`Monitor::overlaymode`].
/// The numeric values are preserved so that external commands (e.g.
/// `setoverlaymode`) that pass a raw integer continue to work. The command
/// handler parses the integer and uses [`OverlayMode::from_i32`] to convert it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayMode {
    /// Overlay slides down from the top edge (default).
    #[default]
    Top,
    /// Overlay slides in from the right edge.
    Right,
    /// Overlay slides up from the bottom edge.
    Bottom,
    /// Overlay slides in from the left edge.
    Left,
}

impl OverlayMode {
    /// Convert a raw `i32` to an [`OverlayMode`].
    /// Returns `None` for any value outside `0..=3`.
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Top),
            1 => Some(Self::Right),
            2 => Some(Self::Bottom),
            3 => Some(Self::Left),
            _ => None,
        }
    }

    /// Return the canonical `i32` representation of this mode (matches the C
    /// `OverlayTop` … `OverlayLeft` enum values).
    pub fn to_i32(self) -> i32 {
        match self {
            Self::Top => 0,
            Self::Right => 1,
            Self::Bottom => 2,
            Self::Left => 3,
        }
    }

    /// Returns `true` for the two modes where the overlay is sized/animated
    /// along the vertical axis (top / bottom).
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Describes which interactive bar region the cursor is currently hovering over.
///
/// Used to drive hover highlighting and drag-gesture detection without scattering
/// raw integer comparisons throughout the codebase.
///
/// The canonical way to produce a `Gesture` from a cursor position is via
/// `crate::bar::model::bar_position_at_x(...).to_gesture()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gesture {
    /// No actionable hover target (cursor is over a neutral area or outside the bar).
    #[default]
    None,
    /// Cursor is over a tag button.  The inner value is the **0-based** tag index.
    Tag(usize),
    /// Cursor is over the overlay activation zone.
    Overlay,
    /// Cursor is over the close button of the selected client.
    CloseButton,
    /// Cursor is over the start-menu icon.
    StartMenu,
}

impl Gesture {
    /// Construct a `Tag` gesture from a 0-based tag index.
    ///
    /// Returns `None` only if the index is unreasonably large (> 63), which
    /// should never occur in practice given the `MAX_TAGS` constant.
    pub fn from_tag_index(tag_index: usize) -> Option<Self> {
        if tag_index < 64 {
            Some(Self::Tag(tag_index))
        } else {
            None
        }
    }

    /// Returns `true` if this gesture represents a tag hover.
    #[allow(dead_code)]
    pub fn is_tag(self) -> bool {
        matches!(self, Self::Tag(_))
    }

    /// Returns the tag index if this is a `Tag` gesture, otherwise `None`.
    #[allow(dead_code)]
    pub fn tag_index(self) -> Option<usize> {
        if let Self::Tag(idx) = self {
            Some(idx)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleFloat {
    Tiled,
    Float,
    FloatCenter,
    FloatFullscreen,
    Scratchpad,
}

/// Monitor selection in rules.
///
/// Replaces the old `i32` field where `-1` meant "any monitor" and `0+` meant
/// a specific monitor index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorRule {
    /// Place on any available monitor (was -1).
    Any,
    /// Place on specific monitor by index.
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpecialNext {
    #[default]
    None,
    Float,
}

/// Action to perform on a boolean toggle setting.
///
/// Replaces the old C pattern where `arg: u32` encoded toggle behavior:
/// - 0 or 2: toggle the value
/// - 1: set to false
/// - else: set to true
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToggleAction {
    /// Toggle the current value (true → false, false → true).
    #[default]
    Toggle,
    /// Set the value to `false`.
    SetFalse,
    /// Set the value to `true`.
    SetTrue,
}

impl ToggleAction {
    /// Parse from a raw u32 value (for compatibility with external commands).
    /// Returns Toggle for invalid values.
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 | 2 => Self::Toggle,
            1 => Self::SetFalse,
            _ => Self::SetTrue,
        }
    }

    /// Parse from command argument string.
    /// Empty string defaults to Toggle, otherwise parses as u32.
    pub fn from_arg(arg: &str) -> Self {
        if arg.is_empty() {
            Self::Toggle
        } else {
            arg.parse().ok().map(Self::from_u32).unwrap_or_default()
        }
    }

    /// Apply this action to a boolean value.
    pub fn apply(self, value: &mut bool) {
        match self {
            Self::Toggle => *value = !*value,
            Self::SetFalse => *value = false,
            Self::SetTrue => *value = true,
        }
    }
}

/// Direction for focus movement and similar operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Direction for stack-based focus movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackDirection {
    /// Move to the next (forward) item in the stack.
    #[default]
    Next,
    /// Move to the previous (backward) item in the stack.
    Previous,
}

impl StackDirection {
    /// Returns true if this is the Next direction.
    pub fn is_forward(self) -> bool {
        matches!(self, Self::Next)
    }

    /// Parse from i32 (for command compatibility): positive = Next, negative/zero = Previous.
    pub fn from_i32(v: i32) -> Self {
        if v > 0 {
            Self::Next
        } else {
            Self::Previous
        }
    }
}

/// Cardinal direction for keyboard-driven window movement/resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardinalDirection {
    #[default]
    Down,
    Up,
    Right,
    Left,
}

impl CardinalDirection {
    /// Convert from integer (for backward compat with config files if needed).
    pub fn from_i32(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::Down),
            1 => Some(Self::Up),
            2 => Some(Self::Right),
            3 => Some(Self::Left),
            _ => None,
        }
    }

    /// Delta for movement (dx, dy).
    pub fn move_delta(self, step: i32) -> (i32, i32) {
        match self {
            Self::Down => (0, step),
            Self::Up => (0, -step),
            Self::Right => (step, 0),
            Self::Left => (-step, 0),
        }
    }

    /// Delta for resize (dw, dh) - grow direction.
    pub fn resize_delta(self, step: i32) -> (i32, i32) {
        match self {
            Self::Down => (0, step),
            Self::Up => (0, -step),
            Self::Right => (step, 0),
            Self::Left => (-step, 0),
        }
    }
}

pub type ClientId = usize;
pub type MonitorId = usize;

/// Size hints for a client window (from WM_NORMAL_HINTS).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizeHints {
    pub basew: i32,
    pub baseh: i32,
    pub incw: i32,
    pub inch: i32,
    pub maxw: i32,
    pub maxh: i32,
    pub minw: i32,
    pub minh: i32,
    pub min_aspect_n: i32,
    pub min_aspect_d: i32,
    pub max_aspect_n: i32,
    pub max_aspect_d: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn area(&self) -> i32 {
        self.w * self.h
    }

    pub fn contains_point(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub fn intersects_other(&self, other: &Rect) -> bool {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.w).min(other.x + other.w);
        let y2 = (self.y + self.h).min(other.y + other.h);
        x1 < x2 && y1 < y2
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn total_width(&self, border_width: i32) -> i32 {
        self.w + 2 * border_width
    }

    pub fn total_height(&self, border_width: i32) -> i32 {
        self.h + 2 * border_width
    }

    /// Convert to a 4-tuple (x, y, w, h) for compatibility with legacy code.
    #[inline]
    pub fn as_tuple(&self) -> (i32, i32, i32, i32) {
        (self.x, self.y, self.w, self.h)
    }

    /// Create a Rect from a 4-tuple.
    #[inline]
    pub fn from_tuple((x, y, w, h): (i32, i32, i32, i32)) -> Self {
        Self { x, y, w, h }
    }

    /// Create a new Rect with adjusted position.
    #[inline]
    pub fn with_pos(&self, x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            w: self.w,
            h: self.h,
        }
    }

    /// Create a new Rect with adjusted size.
    #[inline]
    pub fn with_size(&self, w: i32, h: i32) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w,
            h,
        }
    }

    /// Create a new Rect with borders subtracted.
    #[inline]
    pub fn without_borders(&self, border_width: i32) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w: self.w - 2 * border_width,
            h: self.h - 2 * border_width,
        }
    }

    /// Create a new Rect with borders added.
    #[inline]
    pub fn with_borders(&self, border_width: i32) -> Self {
        Self {
            x: self.x,
            y: self.y,
            w: self.w + 2 * border_width,
            h: self.h + 2 * border_width,
        }
    }

    /// Check if this rect has valid positive dimensions.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0
    }
}

/// Represents a managed client window in the window manager.
///
/// This struct contains all state for a window managed by instantWM,
/// including geometry, tags, flags, and relationships to other clients.
#[derive(Debug, Clone, Default)]
pub struct Client {
    /// Window title/name displayed in the bar.
    pub name: String,
    /// Minimum aspect ratio constraint from WM_NORMAL_HINTS.
    /// Used for size hint calculations when resizing.
    pub mina: f32,
    /// Maximum aspect ratio constraint from WM_NORMAL_HINTS.
    /// Used for size hint calculations when resizing.
    pub maxa: f32,
    pub geo: Rect,
    pub float_geo: Rect,
    pub old_geo: Rect,
    /// Size hints from WM_NORMAL_HINTS property
    pub size_hints: SizeHints,

    /// Size hint fields for backward compatibility (access via size_hints)
    pub base_width: i32,
    pub base_height: i32,
    pub min_width: i32,
    pub min_height: i32,
    pub max_width: i32,
    pub max_height: i32,
    pub inc_width: i32,
    pub inc_height: i32,
    pub base_aspect_n: i32,
    pub base_aspect_d: i32,
    pub min_aspect_n: i32,
    pub min_aspect_d: i32,
    pub max_aspect_n: i32,
    pub max_aspect_d: i32,

    pub hintsvalid: i32,
    pub border_width: i32,
    pub old_border_width: i32,
    pub tags: u32,
    pub isfixed: bool,
    pub isfloating: bool,
    pub isurgent: bool,
    pub neverfocus: bool,
    pub oldstate: i32,
    pub is_fullscreen: bool,
    pub isfakefullscreen: bool,
    pub islocked: bool,
    pub issticky: bool,
    /// Cached minimized/iconic state.
    ///
    /// Set to `true` by [`crate::client::hide`] and back to `false` by
    /// [`crate::client::show`].  Initialised from the live `WM_STATE`
    /// property during [`crate::client::manage`] so that windows that were
    /// already iconic before the WM started are handled correctly.
    ///
    /// Using a cached field avoids an X11 roundtrip on every bar redraw
    /// (the previous `is_hidden(win)` call queried `WM_STATE` each time).
    pub is_hidden: bool,
    pub snapstatus: SnapPosition,
    pub scratchpad_name: String,
    pub scratchpad_restore_tags: u32,
    pub mon_id: Option<MonitorId>,
    /// X11 window id for this client.
    ///
    /// Kept for backward-compatibility during refactors; in most call-sites the
    /// `Window` key used to look up the `Client` is the same value.
    pub win: Window,
    pub next: Option<Window>,
    pub snext: Option<Window>,
}

impl Client {
    pub fn total_width(&self) -> i32 {
        self.geo.total_width(self.border_width)
    }

    pub fn total_height(&self) -> i32 {
        self.geo.total_height(self.border_width)
    }

    pub fn is_scratchpad(&self) -> bool {
        !self.scratchpad_name.is_empty()
    }

    /// True when the client should be treated as visible for a given tag-set.
    ///
    /// This is intentionally pure: callers provide the currently selected
    /// tag-mask for the monitor the client is on.
    #[inline]
    pub fn is_visible_on_tags(&self, selected_tags: u32) -> bool {
        self.issticky || (self.tags & selected_tags) != 0
    }

    /// Backward-compatible convenience wrapper.
    ///
    /// Prefer `is_visible_on_tags` when you can supply the tag-set explicitly.
    pub fn is_visible(&self) -> bool {
        if self.issticky {
            return true;
        }
        if let Some(mon_id) = self.mon_id {
            let globals = crate::globals::get_globals();
            if let Some(mon) = globals.monitors.get(mon_id) {
                return (self.tags & mon.selected_tags()) != 0;
            }
        }
        false
    }
}

/// Internal state of a monitor (screen) in the window manager.
///
/// This struct holds all runtime state for a monitor, including
/// geometry, tag state, client lists, and UI configuration.
#[derive(Debug, Clone)]
pub struct Monitor {
    /// Master factor for tiling layouts (0.0 to 1.0).
    /// Controls the proportion of screen given to the master area.
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
    /// This is the actual rendered height of the bar window.
    pub bt: i32,
    pub monitor_rect: Rect,
    pub work_rect: Rect,
    pub seltags: u32,
    pub tagset: [u32; 2],
    pub activeoffset: u32,
    pub titleoffset: u32,
    pub clientcount: u32,
    pub showbar: bool,
    pub topbar: bool,
    pub overlaystatus: i32,
    pub overlaymode: OverlayMode,
    pub gesture: Gesture,
    pub barwin: Window,
    pub showtags: u32,
    pub current_tag: usize,
    pub prev_tag: usize,
    /// Tags owned by this monitor - each monitor has its own independent tag set.
    pub tags: Vec<Tag>,
    pub clients: Option<Window>,
    pub sel: Option<Window>,
    pub overlay: Option<Window>,
    pub stack: Option<Window>,
    pub fullscreen: Option<Window>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
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
            barwin: 0,
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
            ..Default::default()
        }
    }

    /// Initialize tags from a template (e.g., from global config).
    pub fn init_tags(&mut self, template: &[Tag]) {
        self.tags = template.to_vec();
    }

    /// Get the currently selected tags for this monitor.
    #[inline]
    pub fn selected_tags(&self) -> u32 {
        self.tagset[self.seltags as usize]
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
    pub fn client_count(&self, clients: &std::collections::HashMap<Window, Client>) -> usize {
        let mut count = 0;
        let mut current = self.clients;
        while let Some(c_win) = current {
            if let Some(c) = clients.get(&c_win) {
                if c.is_visible() {
                    count += 1;
                }
                current = c.next;
            } else {
                break;
            }
        }
        count
    }

    /// Count the number of tiled (non-floating, non-hidden) clients on this monitor.
    pub fn tiled_client_count(&self, clients: &std::collections::HashMap<Window, Client>) -> usize {
        let mut count = 0;
        let mut current = self.clients;
        while let Some(c_win) = current {
            if let Some(c) = clients.get(&c_win) {
                if c.is_visible() && !c.isfloating && !c.is_hidden {
                    count += 1;
                }
                current = c.next;
            } else {
                break;
            }
        }
        count
    }

    /// Get the currently selected client window, if any.
    pub fn selected_client(&self) -> Option<Window> {
        self.sel
    }

    /// Check if this monitor has a selected client.
    pub fn has_selection(&self) -> bool {
        self.sel.is_some()
    }

    /// Set the selected client for this monitor.
    pub fn set_selected(&mut self, win: Option<Window>) {
        self.sel = win;
    }

    /// Get the next client in the client list after the given window.
    pub fn next_client(
        &self,
        clients: &std::collections::HashMap<Window, Client>,
        win: Window,
    ) -> Option<Window> {
        clients.get(&win).and_then(|c| c.next)
    }

    /// Get the previous client in the client list before the given window.
    pub fn prev_client(
        &self,
        clients: &std::collections::HashMap<Window, Client>,
        win: Window,
    ) -> Option<Window> {
        let mut current = self.clients;
        let mut prev = None;
        while let Some(c_win) = current {
            if c_win == win {
                return prev;
            }
            prev = current;
            current = clients.get(&c_win).and_then(|c| c.next);
        }
        None
    }

    /// Check if this monitor shows the bar (considering both monitor and tag settings).
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

    /// Get a mutable reference to the current tag for this monitor.
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

    /// Get the current layout kind for this monitor.
    pub fn current_layout(&self) -> crate::layouts::LayoutKind {
        self.current_tag()
            .map(|t| t.layouts.get_layout())
            .unwrap_or(crate::layouts::LayoutKind::Tile)
    }

    /// Toggle between primary and secondary layout slots for the current tag.
    pub fn toggle_layout_slot(&mut self) {
        if let Some(tag) = self.current_tag_mut() {
            tag.layouts.toggle_slot();
        }
    }

    /// Update the bar position based on monitor geometry and configuration.
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

    /// Get the monitor's work area as a rectangle.
    pub fn work_area(&self) -> Rect {
        self.work_rect
    }

    /// Get the monitor's full geometry (including bar).
    pub fn monitor_area(&self) -> Rect {
        self.monitor_rect
    }
}

/// Find a monitor in a given direction from the current one.
pub fn find_monitor_by_direction(
    monitors: &[Monitor],
    current: MonitorId,
    dir: i32,
) -> Option<MonitorId> {
    if monitors.is_empty() {
        return None;
    }
    if monitors.len() <= 1 {
        return Some(current);
    }

    if dir > 0 {
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

    Some(best_idx)
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub class: Option<&'static str>,
    pub instance: Option<&'static str>,
    pub title: Option<&'static str>,
    pub tags: u32,
    pub isfloating: RuleFloat,
    pub monitor: MonitorRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    String,
    Integer,
    Float,
}

#[derive(Debug, Clone)]
pub struct ResourcePref {
    pub name: &'static str,
    pub rtype: ResourceType,
}

#[derive(Debug, Clone)]
pub struct Systray {
    pub win: Window,
    pub icons: Vec<Window>,
}

pub struct Key {
    pub mod_mask: u32,
    pub keysym: u32,
    pub action: Rc<Box<dyn Fn(&mut WmCtx)>>,
}

impl Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key")
            .field("mod_mask", &self.mod_mask)
            .field("keysym", &self.keysym)
            .field("action", &"<closure>")
            .finish()
    }
}

impl Clone for Key {
    fn clone(&self) -> Self {
        Self {
            mod_mask: self.mod_mask,
            keysym: self.keysym,
            action: Rc::clone(&self.action),
        }
    }
}

pub struct Button {
    pub click: Click,
    pub mask: u32,
    pub button: MouseButton,
    pub action: Rc<Box<dyn Fn(&mut WmCtx)>>,
}

impl Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("click", &self.click)
            .field("mask", &self.mask)
            .field("button", &self.button)
            .field("action", &"<closure>")
            .finish()
    }
}

impl Clone for Button {
    fn clone(&self) -> Self {
        Self {
            click: self.click,
            mask: self.mask,
            button: self.button,
            action: Rc::clone(&self.action),
        }
    }
}

#[derive(Debug, Clone)]
pub struct XCommand {
    pub cmd: &'static str,
    pub action: fn(&mut WmCtx, &str),
}

pub fn intersect(r: &Rect, m: &Monitor) -> i32 {
    let x1 = r.x.max(m.work_rect.x);
    let y1 = r.y.max(m.work_rect.y);
    let x2 = (r.x + r.w).min(m.work_rect.x + m.work_rect.w);
    let y2 = (r.y + r.h).min(m.work_rect.y + m.work_rect.h);
    (x2 - x1).max(0) * (y2 - y1).max(0)
}

// Re-export type-safe tag types
mod tag_types;
pub use tag_types::{MonitorDirection, TagMask, TagSelection};
