use x11rb::protocol::xproto::Window;

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

#[derive(Debug, Clone, PartialEq)]
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

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            fg: Clr::default(),
            bg: Clr::default(),
            detail: Clr::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BorderScheme {
    pub normal: ColorScheme,
    pub tile_focus: ColorScheme,
    pub float_focus: ColorScheme,
    pub snap: ColorScheme,
}

impl Default for BorderScheme {
    fn default() -> Self {
        Self {
            normal: ColorScheme::default(),
            tile_focus: ColorScheme::default(),
            float_focus: ColorScheme::default(),
            snap: ColorScheme::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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

impl Default for StatusScheme {
    fn default() -> Self {
        Self {
            fg: Clr::default(),
            bg: Clr::default(),
            detail: Clr::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagSchemes {
    pub no_hover: Vec<ColorScheme>,
    pub hover: Vec<ColorScheme>,
}

impl Default for TagSchemes {
    fn default() -> Self {
        Self {
            no_hover: Vec::new(),
            hover: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSchemes {
    pub no_hover: Vec<ColorScheme>,
    pub hover: Vec<ColorScheme>,
}

impl Default for WindowSchemes {
    fn default() -> Self {
        Self {
            no_hover: Vec::new(),
            hover: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloseButtonSchemes {
    pub no_hover: Vec<ColorScheme>,
    pub hover: Vec<ColorScheme>,
}

impl Default for CloseButtonSchemes {
    fn default() -> Self {
        Self {
            no_hover: Vec::new(),
            hover: Vec::new(),
        }
    }
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

#[derive(Debug, Clone, PartialEq)]
pub struct TagColorConfigs {
    pub no_hover: Vec<ColorSchemeStrings>,
    pub hover: Vec<ColorSchemeStrings>,
}

impl Default for TagColorConfigs {
    fn default() -> Self {
        Self {
            no_hover: Vec::new(),
            hover: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowColorConfigs {
    pub no_hover: Vec<ColorSchemeStrings>,
    pub hover: Vec<ColorSchemeStrings>,
}

impl Default for WindowColorConfigs {
    fn default() -> Self {
        Self {
            no_hover: Vec::new(),
            hover: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloseButtonColorConfigs {
    pub no_hover: Vec<ColorSchemeStrings>,
    pub hover: Vec<ColorSchemeStrings>,
}

impl Default for CloseButtonColorConfigs {
    fn default() -> Self {
        Self {
            no_hover: Vec::new(),
            hover: Vec::new(),
        }
    }
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
#[derive(Debug, Clone)]
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

impl Default for TagSet {
    fn default() -> Self {
        Self {
            tags: Vec::new(),
            colors: Vec::new(),
            schemes: TagSchemes::default(),
            show_alt: false,
            prefix: false,
            width: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub alt_name: &'static str,
    // Pertag / Layout fields
    pub nmaster: i32,
    pub mfact: f32,
    pub sellt: u32,
    pub showbar: bool,
    pub ltidxs: [Option<usize>; 2],
}

impl Default for Tag {
    fn default() -> Self {
        Self {
            name: String::new(),
            alt_name: "",
            nmaster: 1,
            mfact: 0.55,
            sellt: 0,
            showbar: true,
            ltidxs: [None; 2],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AltCursor {
    #[default]
    None,
    Resize,
    //TODO: Port over sidebar from C codebase
    Sidebar,
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
/// Direction for overlay window positioning.
/// Note: This enum is currently unused but kept for potential future use
/// when implementing directional overlay/sidebar functionality from the C codebase.
#[allow(dead_code)]
pub enum OverlayDirection {
    Top,
    Right,
    Bottom,
    Left,
}

//simplify
//probably could also be an enum with None, Tag(u32), Overlay, CloseButton, StartMenu variants
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gesture {
    #[default]
    None,
    Tag1,
    Tag2,
    Tag3,
    Tag4,
    Tag5,
    Tag6,
    Tag7,
    Tag8,
    Tag9,
    Tag10,
    Tag11,
    Tag12,
    Tag13,
    Tag14,
    Tag15,
    Tag16,
    Tag17,
    Tag18,
    Tag19,
    Tag20,
    Tag21,
    Overlay,
    CloseButton,
    StartMenu,
}

impl Gesture {
    pub fn from_tag_index(tag_index: usize) -> Option<Self> {
        match tag_index {
            0 => Some(Self::Tag1),
            1 => Some(Self::Tag2),
            2 => Some(Self::Tag3),
            3 => Some(Self::Tag4),
            4 => Some(Self::Tag5),
            5 => Some(Self::Tag6),
            6 => Some(Self::Tag7),
            7 => Some(Self::Tag8),
            8 => Some(Self::Tag9),
            9 => Some(Self::Tag10),
            10 => Some(Self::Tag11),
            11 => Some(Self::Tag12),
            12 => Some(Self::Tag13),
            13 => Some(Self::Tag14),
            14 => Some(Self::Tag15),
            15 => Some(Self::Tag16),
            16 => Some(Self::Tag17),
            17 => Some(Self::Tag18),
            18 => Some(Self::Tag19),
            19 => Some(Self::Tag20),
            20 => Some(Self::Tag21),
            _ => None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpecialNext {
    #[default]
    None,
    Float,
}

/// Direction for focus movement and similar operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// Convert from direction index used in Arg.ui (0=Up, 1=Right, 2=Down, 3=Left)
    pub fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(Self::Up),
            1 => Some(Self::Right),
            2 => Some(Self::Down),
            3 => Some(Self::Left),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Arg {
    pub i: i32,
    pub ui: u32,
    pub f: f32,
    pub v: Option<usize>,
}

pub trait Layout: std::fmt::Debug {
    fn symbol(&self) -> &'static str;
    fn arrange(&self, m: &mut Monitor);
    fn is_tiling(&self) -> bool;
    fn is_monocle(&self) -> bool {
        false
    }
    fn is_overview(&self) -> bool {
        false
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
    pub snapstatus: SnapPosition,
    pub scratchpad_name: String,
    pub scratchpad_restore_tags: u32,
    pub mon_id: Option<MonitorId>,
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

    pub fn is_visible(&self) -> bool {
        if self.issticky {
            return true;
        }
        if let Some(mon_id) = self.mon_id {
            let globals = crate::globals::get_globals();
            if let Some(mon) = globals.monitors.get(mon_id) {
                let tags = mon.tagset[mon.seltags as usize];
                return (self.tags & tags) != 0;
            }
        }
        false
    }
}

/// Internal state of a monitor (screen) in the window manager.
///
/// This struct holds all runtime state for a monitor, including
/// geometry, tag state, client lists, and UI configuration.
#[derive(Debug, Clone, PartialEq)]
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
    pub overlaymode: i32,
    pub gesture: Gesture,
    pub barwin: Window,
    pub showtags: u32,
    pub current_tag: usize,
    pub prev_tag: usize,
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
            overlaymode: 0,
            gesture: Gesture::default(),
            barwin: 0,
            showtags: 0,
            current_tag: 0,
            prev_tag: 0,
            clients: None,
            sel: None,
            overlay: None,
            stack: None,
            fullscreen: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub class: Option<&'static str>,
    pub instance: Option<&'static str>,
    pub title: Option<&'static str>,
    pub tags: u32,
    pub isfloating: RuleFloat,
    pub monitor: i32,
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

/// Action that can be bound to a key or button.
/// This enum allows different function signatures to be used in bindings.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    /// No action
    None,
    /// Function taking an Arg pointer (legacy)
    WithArg(fn(&Arg)),
    /// Focus stack with direction
    FocusStack(bool),
    /// Focus in a direction
    FocusDirection(Direction),
    /// Cycle layout forward/backward
    CycleLayout(bool),
    /// Increment nmaster by delta
    IncNmaster(i32),
    /// Shift view forward/backward
    ShiftView(bool),
    /// Tag to left/right by offset
    TagToLeft(i32),
    TagToRight(i32),
}

impl Default for Action {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone)]
pub struct Key {
    pub mod_mask: u32,
    pub keysym: u32,
    pub func: Option<fn(&Arg)>,
    pub arg: Arg,
}

#[derive(Debug, Clone)]
pub struct Button {
    pub click: Click,
    pub mask: u32,
    pub button: u8,
    pub func: Option<fn(&Arg)>,
    pub arg: Arg,
}

#[derive(Debug, Clone)]
pub struct XCommand {
    pub cmd: &'static str,
    pub func: Option<fn(&Arg)>,
    pub arg: Arg,
    pub cmd_type: u32,
}

pub fn intersect(r: &Rect, m: &Monitor) -> i32 {
    let x1 = r.x.max(m.work_rect.x);
    let y1 = r.y.max(m.work_rect.y);
    let x2 = (r.x + r.w).min(m.work_rect.x + m.work_rect.w);
    let y2 = (r.y + r.h).min(m.work_rect.y + m.work_rect.h);
    (x2 - x1).max(0) * (y2 - y1).max(0)
}
