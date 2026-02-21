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
///
/// This replaces nine scattered fields on `Globals`:
/// `tags`, `tagsalt`, `numtags`, `tagmask`, `tagcolors`, `tagschemes`,
/// `tagwidth`, `showalttag`, and `tagprefix`.
#[derive(Debug, Clone)]
pub struct TagSet {
    /// Primary tag labels, one per active tag.
    pub names: Vec<String>,
    /// Alternate labels shown when `show_alt` is true.
    pub alt_names: Vec<&'static str>,
    /// Number of active tags.
    pub count: usize,
    /// Raw colour strings from config/xresources, indexed [tag][hover_state][colour_index].
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
        (1u32 << self.count).wrapping_sub(1)
    }
}

impl Default for TagSet {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            alt_names: Vec::new(),
            count: 0,
            colors: Vec::new(),
            schemes: TagSchemes::default(),
            show_alt: false,
            prefix: false,
            width: 0,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
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
    fn arrange(&self, m: &mut MonitorInner);
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

//TODO: why do both client and clientInner exist?
//TODO: dimensions should probaly be their own structs instead of 4 fields each time
#[derive(Debug, Clone, Default)]
pub struct ClientInner {
    pub name: String,
    pub mina: f32,
    pub maxa: f32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub saved_float_x: i32,
    pub saved_float_y: i32,
    pub saved_float_width: i32,
    pub saved_float_height: i32,
    pub oldx: i32,
    pub oldy: i32,
    pub oldw: i32,
    pub oldh: i32,
    pub basew: i32,
    pub baseh: i32,
    pub incw: i32,
    pub inch: i32,
    pub maxw: i32,
    pub maxh: i32,
    pub minw: i32,
    pub minh: i32,
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

impl ClientInner {
    pub fn total_width(&self) -> i32 {
        self.w + 2 * self.border_width
    }

    pub fn total_height(&self) -> i32 {
        self.h + 2 * self.border_width
    }

    pub fn is_scratchpad(&self) -> bool {
        !self.scratchpad_name.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pertag {
    pub current_tag: u32,
    pub prevtag: u32,
    pub nmasters: [i32; MAX_TAGS],
    pub mfacts: [f32; MAX_TAGS],
    pub sellts: [u32; MAX_TAGS],
    pub showbars: [bool; MAX_TAGS],
    pub ltidxs: [[Option<usize>; 2]; MAX_TAGS],
}

impl Default for Pertag {
    fn default() -> Self {
        Self {
            current_tag: 0,
            prevtag: 0,
            nmasters: [0; MAX_TAGS],
            mfacts: [0.0; MAX_TAGS],
            sellts: [0; MAX_TAGS],
            showbars: [false; MAX_TAGS],
            ltidxs: [[None; 2]; MAX_TAGS],
        }
    }
}

//TODO: dimensions should probably be their own struct, with utils for commonly used operations
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInner {
    pub ltsymbol: String,
    pub mfact: f32,
    pub nmaster: i32,
    pub num: i32,
    pub by: i32,
    pub bar_clients_width: i32,
    pub bt: i32,
    pub mx: i32,
    pub my: i32,
    pub mw: i32,
    pub mh: i32,
    pub wx: i32,
    pub wy: i32,
    pub ww: i32,
    pub wh: i32,
    pub seltags: u32,
    pub sellt: u32,
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
    pub pertag: Option<Box<Pertag>>,
    pub clients: Option<Window>,
    pub sel: Option<Window>,
    pub overlay: Option<Window>,
    pub stack: Option<Window>,
    pub fullscreen: Option<Window>,
}

impl Default for MonitorInner {
    fn default() -> Self {
        Self {
            ltsymbol: String::from("[]="),
            mfact: 0.55,
            nmaster: 1,
            num: 0,
            by: 0,
            bar_clients_width: 0,
            bt: 0,
            mx: 0,
            my: 0,
            mw: 0,
            mh: 0,
            wx: 0,
            wy: 0,
            ww: 0,
            wh: 0,
            seltags: 0,
            sellt: 0,
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
            pertag: None,
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

pub fn intersect(x: i32, y: i32, w: i32, h: i32, m: &MonitorInner) -> i32 {
    let x1 = x.max(m.wx);
    let y1 = y.max(m.wy);
    let x2 = (x + w).min(m.wx + m.ww);
    let y2 = (y + h).min(m.wy + m.wh);
    (x2 - x1).max(0) * (y2 - y1).max(0)
}

pub fn is_visible(tags: u32, mon_tags: u32, _seltags: u32, issticky: bool) -> bool {
    (tags & mon_tags) != 0 || issticky
}
