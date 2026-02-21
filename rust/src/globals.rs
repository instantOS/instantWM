use crate::drw::{Clr, Cur, Drw};
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
    pub monitors: Vec<MonitorInner>,
    pub selmon: Option<MonitorId>,
    pub clients: HashMap<Window, ClientInner>,
    pub client_list: Vec<ClientId>,
    pub bh: i32,
    pub lrpad: i32,
    pub animated: bool,
    pub focusfollowsmouse: bool,
    pub focusfollowsfloatmouse: bool,
    pub altcursor: AltCursor,
    pub doubledraw: bool,
    pub specialnext: SpecialNext,
    pub bar_dragging: bool,
    pub tagwidth: i32,
    pub statuswidth: i32,
    pub showalttag: bool,
    pub tagprefix: bool,
    pub stext: [u8; 1024],
    pub wmatom: [u32; 4],
    pub netatom: [u32; 14],
    pub xatom: [u32; 3],
    pub motifatom: u32,
    pub numlockmask: u32,
    pub showsystray: bool,
    pub systraypinning: u32,
    pub systrayspacing: u32,
    pub systray: Option<Systray>,
    pub drw: Option<Drw>,
    pub xlibdisplay: XlibDisplay,
    pub cursors: [Option<Cur>; 10],
    pub borderscheme: Option<Vec<Clr>>,
    pub statusscheme: Option<Vec<Clr>>,
    pub tagschemes: Vec<Vec<Vec<Clr>>>,
    pub windowschemes: Vec<Vec<Vec<Clr>>>,
    pub closebuttonschemes: Vec<Vec<Vec<Clr>>>,
    pub startmenusize: u32,
    pub snap: u32,
    pub resizehints: i32,
    pub tags: [[u8; 16]; MAX_TAGS],
    pub tagsalt: Vec<&'static str>,
    pub layouts: Vec<&'static dyn Layout>,
    pub numtags: i32,
    pub keys_len: usize,
    pub dkeys_len: usize,
    pub commands_len: usize,
    pub buttons_len: usize,
    pub layouts_len: usize,
    pub rules_len: usize,
    pub fonts_len: usize,
    pub commands: Vec<XCommand>,
    pub buttons: Vec<Button>,
    pub fonts: Vec<&'static str>,
    pub tagcolors: Vec<Vec<Vec<&'static str>>>,
    pub windowcolors: Vec<Vec<Vec<&'static str>>>,
    pub closebuttoncolors: Vec<Vec<Vec<&'static str>>>,
    pub bordercolors: Vec<&'static str>,
    pub statusbarcolors: Vec<&'static str>,
    pub keys: Vec<Key>,
    pub dkeys: Vec<Key>,
    pub rules: Vec<Rule>,
    pub resources: Vec<ResourcePref>,
    pub tagmask: u32,
    pub borderpx: u32,
    pub decorhints: i32,
    pub mfact: f32,
    pub nmaster: i32,
    pub showbar: bool,
    pub topbar: bool,
    pub barheight: i32,
    pub xresourcesfont: [u8; 30],
    pub instantmenumon: [u8; 2],
    pub instantmenucmd: Vec<&'static str>,
    pub instantshutdowncmd: Vec<&'static str>,
    pub startmenucmd: Vec<&'static str>,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            screen: 0,
            root: 0,
            sw: 0,
            sh: 0,
            monitors: Vec::new(),
            selmon: None,
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
            tagwidth: 0,
            statuswidth: 0,
            showalttag: false,
            tagprefix: false,
            stext: [0; 1024],
            wmatom: [0; 4],
            netatom: [0; 14],
            xatom: [0; 3],
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
            tagschemes: Vec::new(),
            windowschemes: Vec::new(),
            closebuttonschemes: Vec::new(),
            startmenusize: 0,
            snap: 32,
            resizehints: 1,
            tags: [[0; 16]; MAX_TAGS],
            tagsalt: Vec::new(),
            layouts: Vec::new(),
            numtags: 0,
            keys_len: 0,
            dkeys_len: 0,
            commands_len: 0,
            buttons_len: 0,
            layouts_len: 0,
            rules_len: 0,
            fonts_len: 0,
            commands: Vec::new(),
            buttons: Vec::new(),
            fonts: Vec::new(),
            tagcolors: Vec::new(),
            windowcolors: Vec::new(),
            closebuttoncolors: Vec::new(),
            bordercolors: Vec::new(),
            statusbarcolors: Vec::new(),
            keys: Vec::new(),
            dkeys: Vec::new(),
            rules: Vec::new(),
            resources: Vec::new(),
            tagmask: 0,
            borderpx: 1,
            decorhints: 0,
            mfact: 0.55,
            nmaster: 1,
            showbar: true,
            topbar: true,
            barheight: 0,
            xresourcesfont: [0; 30],
            instantmenumon: [0; 2],
            instantmenucmd: Vec::new(),
            instantshutdowncmd: Vec::new(),
            startmenucmd: Vec::new(),
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
