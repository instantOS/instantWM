use x11rb::protocol::xproto::Window;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetAtom {
    Supported,
    WMName,
    WMState,
    WMCheck,
    SystemTray,
    SystemTrayOP,
    SystemTrayOrientation,
    SystemTrayOrientationHorz,
    WMFullscreen,
    ActiveWindow,
    WMWindowType,
    WMWindowTypeDialog,
    ClientList,
    ClientInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WmAtom {
    Protocols,
    Delete,
    State,
    TakeFocus,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gesture {
    #[default]
    None = 0,
    Overlay = 30,
    CloseButton = 31,
    StartMenu = 32,
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

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub symbol: &'static str,
    pub arrange: fn(&mut MonitorInner),
}

pub type ClientId = usize;
pub type MonitorId = usize;

#[derive(Debug, Clone)]
pub struct ClientInner {
    pub name: [u8; 256],
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
    pub scratchpad_name: [u8; SCRATCHPAD_NAME_LEN],
    pub scratchpad_restore_tags: u32,
    pub mon_id: Option<MonitorId>,
    pub win: Window,
    pub next: Option<Window>,
    pub snext: Option<Window>,
}

impl Default for ClientInner {
    fn default() -> Self {
        Self {
            name: [0; 256],
            mina: 0.0,
            maxa: 0.0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            saved_float_x: 0,
            saved_float_y: 0,
            saved_float_width: 0,
            saved_float_height: 0,
            oldx: 0,
            oldy: 0,
            oldw: 0,
            oldh: 0,
            basew: 0,
            baseh: 0,
            incw: 0,
            inch: 0,
            maxw: 0,
            maxh: 0,
            minw: 0,
            minh: 0,
            hintsvalid: 0,
            border_width: 0,
            old_border_width: 0,
            tags: 0,
            isfixed: false,
            isfloating: false,
            isurgent: false,
            neverfocus: false,
            oldstate: 0,
            is_fullscreen: false,
            isfakefullscreen: false,
            islocked: false,
            issticky: false,
            snapstatus: SnapPosition::default(),
            scratchpad_name: [0; SCRATCHPAD_NAME_LEN],
            scratchpad_restore_tags: 0,
            mon_id: None,
            win: 0,
        }
    }
}

impl ClientInner {
    pub fn is_scratchpad(&self) -> bool {
        self.scratchpad_name[0] != 0
    }
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct MonitorInner {
    pub ltsymbol: [u8; 16],
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
}

impl Default for MonitorInner {
    fn default() -> Self {
        Self {
            ltsymbol: [0; 16],
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
    pub icons: Vec<ClientId>,
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

pub fn width(w: i32, border_width: i32) -> i32 {
    w + 2 * border_width
}

pub fn height(h: i32, border_width: i32) -> i32 {
    h + 2 * border_width
}
