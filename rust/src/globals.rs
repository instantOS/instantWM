use crate::config::commands::ExternalCommands;
use crate::drw::{Cur, Drw};
use crate::types::*;
use once_cell::sync::Lazy;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use x11rb::protocol::xproto::Window;

// Wrapper for Xlib display pointer that implements Send/Sync
// Xlib displays are thread-safe for reading (but not for concurrent writes to the same display)
#[derive(Clone, Copy)]
pub struct XlibDisplay(pub *mut libc::c_void);
unsafe impl Send for XlibDisplay {}
unsafe impl Sync for XlibDisplay {}

pub struct Globals {
    pub screen: i32,
    pub root: Window,
    pub sw: i32,
    pub sh: i32,
    /// All monitors attached to this display, indexed by `MonitorId`.
    /// `MonitorId` is a `usize` index into this `Vec`. This index-based
    /// approach is used throughout (e.g. `selmon`, `Client::mon_id`) to
    /// avoid self-referential structs and borrow checker conflicts.
    pub monitors: Vec<Monitor>,
    pub selmon: MonitorId,
    pub clients: HashMap<Window, Client>,
    pub client_list: Vec<ClientId>,
    /// Bar height in pixels (calculated from font metrics).
    /// This is the actual rendered height of the bar window.
    pub bh: i32,
    pub lrpad: i32,
    pub animated: bool,
    pub focusfollowsmouse: bool,
    pub focusfollowsfloatmouse: bool,
    pub altcursor: AltCursor,
    pub doubledraw: bool,
    pub specialnext: SpecialNext,
    pub bar_dragging: bool,
    pub tags: TagSet,
    /// Width of the status text area in pixels (cached for layout calculations).
    pub status_text_width: i32,
    /// Status text displayed in the bar (right side, shows system info).
    pub status_text: String,
    pub wmatom: WmAtoms,
    pub netatom: NetAtoms,
    pub xatom: XAtoms,
    pub motifatom: Atom,
    /// X11 modifier mask for NumLock (used when matching/grabbing keys and buttons).
    pub numlockmask: u32,
    pub showsystray: bool,
    /// Number of systray icons to pin to the start (0 = no pinning).
    pub systraypinning: usize,
    /// Pixel gap between systray icons.
    pub systrayspacing: i32,
    pub systray: Option<Systray>,
    //TODO: why is this an option? Can the window manager ever function without
    //this?
    pub drw: Option<Drw>,
    pub xlibdisplay: XlibDisplay,
    pub cursors: [Option<Cur>; 10],
    pub borderscheme: Option<BorderScheme>,
    pub statusscheme: Option<StatusScheme>,
    pub windowschemes: WindowSchemes,
    pub closebuttonschemes: CloseButtonSchemes,
    /// Start menu / tag bar width in pixels.
    pub startmenusize: i32,
    /// Snap-to-edge distance in pixels.
    pub snap: i32,
    pub resizehints: i32,
    pub layouts: Vec<&'static dyn Layout>,
    pub commands: Vec<XCommand>,
    pub buttons: Vec<Button>,
    pub fonts: Vec<&'static str>,
    pub windowcolors: Vec<Vec<Vec<&'static str>>>,
    pub closebuttoncolors: Vec<Vec<Vec<&'static str>>>,
    pub bordercolors: Vec<&'static str>,
    pub statusbarcolors: Vec<&'static str>,
    pub keys: Vec<Key>,
    pub dkeys: Vec<Key>,
    pub rules: Vec<Rule>,
    pub resources: Vec<ResourcePref>,
    /// Border width in pixels.
    pub borderpx: i32,
    pub decorhints: i32,
    pub mfact: f32,
    pub nmaster: i32,
    pub showbar: bool,
    pub topbar: bool,
    pub barheight: i32,
    pub xresourcesfont: String,
    pub instantmenumon: String,
    /// All external commands resolved at startup from [`crate::config::commands`].
    pub external_commands: ExternalCommands,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            screen: 0,
            root: 0,
            sw: 0,
            sh: 0,
            monitors: Vec::new(),
            selmon: 0,
            clients: HashMap::new(),
            client_list: Vec::new(),
            bh: 0,
            lrpad: 0,
            animated: true,
            focusfollowsmouse: true,
            focusfollowsfloatmouse: true,
            altcursor: AltCursor::None,
            doubledraw: false,
            specialnext: SpecialNext::None,
            bar_dragging: false,
            tags: TagSet::default(),
            status_text_width: 0,
            status_text: String::new(),
            wmatom: WmAtoms::default(),
            netatom: NetAtoms::default(),
            xatom: XAtoms::default(),
            motifatom: 0,
            numlockmask: 0,
            showsystray: true,
            systraypinning: 0,
            systrayspacing: 2,
            systray: None,
            drw: None,
            xlibdisplay: XlibDisplay(std::ptr::null_mut()),
            cursors: Default::default(),
            borderscheme: None,
            statusscheme: None,
            windowschemes: WindowSchemes::default(),
            closebuttonschemes: CloseButtonSchemes::default(),
            startmenusize: 0,
            snap: 32,
            resizehints: 1,
            layouts: Vec::new(),
            commands: Vec::new(),
            buttons: Vec::new(),
            fonts: Vec::new(),
            windowcolors: Vec::new(),
            closebuttoncolors: Vec::new(),
            bordercolors: Vec::new(),
            statusbarcolors: Vec::new(),
            keys: Vec::new(),
            dkeys: Vec::new(),
            rules: Vec::new(),
            resources: Vec::new(),
            borderpx: 1,
            decorhints: 0,
            mfact: 0.55,
            nmaster: 1,
            showbar: true,
            topbar: true,
            barheight: 0,
            xresourcesfont: String::new(),
            instantmenumon: String::new(),
            external_commands: crate::config::commands::default_commands(),
        }
    }
}

// SAFETY: instantWM is a single-threaded window manager.
// Globals are accessed from the main thread event loop, like dwm's C globals.
struct MainThreadCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for MainThreadCell<T> {}
unsafe impl<T> Send for MainThreadCell<T> {}

pub static GLOBALS: Lazy<MainThreadCell<Globals>> =
    Lazy::new(|| MainThreadCell(UnsafeCell::new(Globals::default())));
pub static RUNNING: AtomicBool = AtomicBool::new(true);

pub fn get_globals() -> &'static Globals {
    unsafe { &*GLOBALS.0.get() }
}

pub fn get_globals_mut() -> &'static mut Globals {
    unsafe { &mut *GLOBALS.0.get() }
}

pub struct X11Connection {
    pub conn: Option<x11rb::rust_connection::RustConnection>,
    pub screen_num: usize,
}

impl Default for X11Connection {
    fn default() -> Self {
        Self {
            conn: None,
            screen_num: 0,
        }
    }
}

pub static X11: Lazy<MainThreadCell<X11Connection>> =
    Lazy::new(|| MainThreadCell(UnsafeCell::new(X11Connection::default())));

pub fn get_x11() -> &'static X11Connection {
    unsafe { &*X11.0.get() }
}

pub fn get_x11_mut() -> &'static mut X11Connection {
    unsafe { &mut *X11.0.get() }
}
