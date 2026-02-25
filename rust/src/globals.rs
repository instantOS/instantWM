use crate::config::commands::ExternalCommands;
use crate::drw::{Cur, Drw};
use crate::types::*;
use once_cell::sync::Lazy;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use x11rb::protocol::xproto::Window;

#[derive(Clone, Copy)]
pub struct XlibDisplay(pub *mut libc::c_void);
unsafe impl Send for XlibDisplay {}
unsafe impl Sync for XlibDisplay {}

/// Runtime configuration - values loaded from config and xresources
/// These are set during initialization and updated on reload
#[derive(Clone)]
pub struct RuntimeConfig {
    // Screen/Display info
    pub screen: i32,
    pub root: Window,
    pub sw: i32,
    pub sh: i32,

    // Window manager configuration
    pub borderpx: i32,
    pub snap: i32,
    pub startmenusize: i32,
    pub resizehints: i32,
    pub decorhints: i32,
    pub mfact: f32,
    pub nmaster: i32,
    pub showbar: bool,
    pub topbar: bool,
    pub barheight: i32,
    pub showsystray: bool,
    pub systraypinning: usize,
    pub systrayspacing: i32,

    // X11 atoms
    pub wmatom: WmAtoms,
    pub netatom: NetAtoms,
    pub xatom: XAtoms,
    pub motifatom: Atom,
    pub numlockmask: u32,

    // Color schemes
    pub borderscheme: Option<BorderScheme>,
    pub statusscheme: Option<StatusScheme>,
    pub windowschemes: WindowSchemes,
    pub closebuttonschemes: CloseButtonSchemes,

    // Raw color strings for xresources override
    pub windowcolors: Vec<Vec<Vec<&'static str>>>,
    pub closebuttoncolors: Vec<Vec<Vec<&'static str>>>,
    pub bordercolors: Vec<&'static str>,
    pub statusbarcolors: Vec<&'static str>,

    // Bindings
    pub keys: Vec<Key>,
    pub dkeys: Vec<Key>,
    pub buttons: Vec<Button>,
    pub rules: Vec<Rule>,
    pub commands: Vec<XCommand>,

    // Resources
    pub resources: Vec<ResourcePref>,
    pub fonts: Vec<&'static str>,
    pub xresourcesfont: String,
    pub instantmenumon: String,

    // External commands
    pub external_commands: ExternalCommands,

    // Drawing context
    pub drw: Option<Drw>,
    pub xlibdisplay: XlibDisplay,
    pub cursors: [Option<Cur>; 10],
    pub bh: i32,
    pub lrpad: i32,
    /// Template tag list cloned into every new monitor.
    pub tag_template: Vec<crate::types::Tag>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            screen: 0,
            root: 0,
            sw: 0,
            sh: 0,
            borderpx: 1,
            snap: 32,
            startmenusize: 0,
            resizehints: 1,
            decorhints: 0,
            mfact: 0.55,
            nmaster: 1,
            showbar: true,
            topbar: true,
            barheight: 0,
            showsystray: true,
            systraypinning: 0,
            systrayspacing: 2,
            wmatom: WmAtoms::default(),
            netatom: NetAtoms::default(),
            xatom: XAtoms::default(),
            motifatom: 0,
            numlockmask: 0,
            borderscheme: None,
            statusscheme: None,
            windowschemes: WindowSchemes::default(),
            closebuttonschemes: CloseButtonSchemes::default(),
            windowcolors: Vec::new(),
            closebuttoncolors: Vec::new(),
            bordercolors: Vec::new(),
            statusbarcolors: Vec::new(),
            keys: Vec::new(),
            dkeys: Vec::new(),
            buttons: Vec::new(),
            rules: Vec::new(),
            commands: Vec::new(),
            resources: Vec::new(),
            fonts: Vec::new(),
            xresourcesfont: String::new(),
            instantmenumon: String::new(),
            external_commands: crate::config::commands::default_commands(),
            drw: None,
            xlibdisplay: XlibDisplay(std::ptr::null_mut()),
            cursors: [const { None }; 10],
            bh: 0,
            lrpad: 0,
            tag_template: Vec::new(),
        }
    }
}

pub struct Globals {
    // Runtime configuration (loaded from config + xresources)
    pub cfg: RuntimeConfig,

    // Runtime state (changes during WM operation)
    pub monitors: Vec<Monitor>,
    /// Index of the currently selected monitor in `monitors`.
    ///
    /// Private – use `selmon()`, `selmon_mut()`, `selmon_id()`, and
    /// `set_selmon()` to read/write this value so that `Monitor::monitor_id`
    /// is kept in sync automatically.
    selmon_idx: MonitorId,
    pub clients: HashMap<Window, Client>,
    pub client_list: Vec<ClientId>,
    pub tags: TagSet,
    pub systray: Option<Systray>,

    // Runtime flags
    pub animated: bool,
    pub focusfollowsmouse: bool,
    pub focusfollowsfloatmouse: bool,
    pub altcursor: AltCursor,
    pub resize_direction: Option<ResizeDirection>,
    pub doubledraw: bool,
    pub specialnext: SpecialNext,
    pub bar_dragging: bool,
    pub status_text_width: i32,
    pub status_text: String,
}

impl Globals {
    // -------------------------------------------------------------------------
    // Selected-monitor accessors
    // -------------------------------------------------------------------------

    /// Return a reference to the currently selected monitor, if one exists.
    #[inline]
    pub fn selmon(&self) -> Option<&Monitor> {
        self.monitors.get(self.selmon_idx)
    }

    /// Return a mutable reference to the currently selected monitor, if one exists.
    #[inline]
    pub fn selmon_mut(&mut self) -> Option<&mut Monitor> {
        self.monitors.get_mut(self.selmon_idx)
    }

    /// Return the `MonitorId` of the currently selected monitor.
    ///
    /// Equivalent to `selmon().map(|m| m.id())` but without a borrow.
    /// Prefer this over reaching into `g.selmon_idx` directly.
    #[inline]
    pub fn selmon_id(&self) -> MonitorId {
        self.selmon_idx
    }

    /// Change the currently selected monitor.
    ///
    /// `id` must be a valid index into `monitors`; passing an out-of-bounds
    /// value is not unsafe but `selmon()` will return `None` until corrected.
    #[inline]
    pub fn set_selmon(&mut self, id: MonitorId) {
        self.selmon_idx = id;
    }

    // -------------------------------------------------------------------------
    // Monitor vec management
    // -------------------------------------------------------------------------

    /// Append a monitor to the vec, stamp its `monitor_id`, and return the
    /// new id.  Always use this instead of `monitors.push()` directly so
    /// that `Monitor::monitor_id` is always correct.
    pub fn push_monitor(&mut self, mut m: Monitor) -> MonitorId {
        let id = self.monitors.len();
        m.monitor_id = id;
        self.monitors.push(m);
        id
    }

    /// Remove the monitor at `mon_id` and fix up all stored indices.
    ///
    /// After removal every monitor whose index shifted down by one has its
    /// `monitor_id` decremented, and `selmon_idx` is clamped / adjusted to
    /// remain valid.
    pub fn remove_monitor(&mut self, mon_id: MonitorId) {
        if mon_id >= self.monitors.len() {
            return;
        }
        self.monitors.remove(mon_id);
        // Re-stamp ids for monitors that shifted.
        for (i, m) in self.monitors.iter_mut().enumerate() {
            m.monitor_id = i;
        }
        // Adjust selected-monitor index.
        if self.selmon_idx == mon_id {
            self.selmon_idx = 0;
        } else if self.selmon_idx > mon_id {
            self.selmon_idx -= 1;
        }
    }

    // -------------------------------------------------------------------------
    // Arbitrary-monitor accessors
    // -------------------------------------------------------------------------

    /// Return a reference to the monitor with the given id, if it exists.
    #[inline]
    pub fn monitor(&self, id: MonitorId) -> Option<&Monitor> {
        self.monitors.get(id)
    }

    /// Return a mutable reference to the monitor with the given id, if it exists.
    #[inline]
    pub fn monitor_mut(&mut self, id: MonitorId) -> Option<&mut Monitor> {
        self.monitors.get_mut(id)
    }

    // -------------------------------------------------------------------------
    // Monitor iteration
    // -------------------------------------------------------------------------

    /// Iterate over all monitors, yielding `(MonitorId, &Monitor)` pairs.
    ///
    /// Prefer this over `monitors.iter().enumerate()` at call-sites that need
    /// the index alongside the monitor reference.
    #[inline]
    pub fn monitors_iter(&self) -> impl Iterator<Item = (MonitorId, &Monitor)> {
        self.monitors.iter().enumerate()
    }

    /// Iterate mutably over all monitors, yielding `(MonitorId, &mut Monitor)` pairs.
    #[inline]
    pub fn monitors_iter_mut(&mut self) -> impl Iterator<Item = (MonitorId, &mut Monitor)> {
        self.monitors.iter_mut().enumerate()
    }

    // -------------------------------------------------------------------------
    // Selected-monitor convenience helpers
    // -------------------------------------------------------------------------

    /// Return the window currently selected on the selected monitor, if any.
    #[inline]
    pub fn selected_win(&self) -> Option<x11rb::protocol::xproto::Window> {
        self.selmon().and_then(|m| m.sel)
    }
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            cfg: RuntimeConfig::default(),
            monitors: Vec::new(),
            selmon_idx: 0,
            clients: HashMap::new(),
            client_list: Vec::new(),
            tags: TagSet::default(),
            systray: None,
            animated: true,
            focusfollowsmouse: true,
            focusfollowsfloatmouse: true,
            altcursor: AltCursor::None,
            resize_direction: None,
            doubledraw: false,
            specialnext: SpecialNext::None,
            bar_dragging: false,
            status_text_width: 0,
            status_text: String::new(),
        }
    }
}

struct MainThreadCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for MainThreadCell<T> {}
unsafe impl<T> Send for MainThreadCell<T> {}

pub static GLOBALS: Lazy<MainThreadCell<Globals>> =
    Lazy::new(|| MainThreadCell(UnsafeCell::new(Globals::default())));

pub static RUNNING: AtomicBool = AtomicBool::new(true);

#[inline]
pub fn get_drw() -> &'static Drw {
    get_globals()
        .cfg
        .drw
        .as_ref()
        .expect("get_drw() called before setup() initialised the drawing context")
}

#[inline]
pub fn get_drw_mut() -> &'static mut Drw {
    get_globals_mut()
        .cfg
        .drw
        .as_mut()
        .expect("get_drw_mut() called before setup() initialised the drawing context")
}

pub fn get_globals() -> &'static Globals {
    unsafe { &*GLOBALS.0.get() }
}

pub fn get_globals_mut() -> &'static mut Globals {
    unsafe { &mut *GLOBALS.0.get() }
}

/// Storage for the X11 connection during initialization and shutdown.
/// After initialization, use [`X11Conn`] which guarantees the connection exists.
#[derive(Default)]
pub struct X11Connection {
    pub conn: Option<x11rb::rust_connection::RustConnection>,
    pub screen_num: usize,
}

/// A guaranteed X11 connection reference for use after initialization.
///
/// This type ensures at compile time that the X11 connection is available.
/// If X11 is not reachable, the window manager cannot function and should crash.
pub struct X11Conn<'a> {
    pub conn: &'a x11rb::rust_connection::RustConnection,
    pub screen_num: usize,
}

impl<'a> X11Conn<'a> {
    /// Create a new X11Conn from a reference to the connection and screen number.
    pub fn new(conn: &'a x11rb::rust_connection::RustConnection, screen_num: usize) -> Self {
        Self { conn, screen_num }
    }
}

impl X11Connection {
    /// Get a guaranteed X11 connection reference.
    ///
    /// # Panics
    ///
    /// Panics if the connection is not available. This should only happen
    /// during initialization before the connection is established, or after
    /// cleanup when the connection has been closed.
    pub fn conn(&self) -> &x11rb::rust_connection::RustConnection {
        self.conn
            .as_ref()
            .expect("X11 connection not available - this is a fatal error for a window manager")
    }

    /// Create an X11Conn from this connection.
    ///
    /// # Panics
    ///
    /// Panics if the connection is not available.
    pub fn as_conn(&self) -> X11Conn<'_> {
        X11Conn {
            conn: self.conn(),
            screen_num: self.screen_num,
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

/// Update the runtime configuration from a config::Config struct
/// This is called during initialization and on reload
pub fn update_config_from_config(cfg: &crate::config::Config) {
    let g = get_globals_mut();
    g.cfg.borderpx = cfg.borderpx;
    g.cfg.snap = cfg.snap;
    g.cfg.startmenusize = cfg.startmenusize;
    g.cfg.systraypinning = cfg.systraypinning;
    g.cfg.systrayspacing = cfg.systrayspacing;
    g.cfg.showsystray = cfg.showsystray;
    g.cfg.showbar = cfg.showbar;
    g.cfg.topbar = cfg.topbar;
    g.cfg.barheight = cfg.barheight;
    g.cfg.resizehints = cfg.resizehints;
    g.cfg.decorhints = cfg.decorhints;
    g.cfg.mfact = cfg.mfact;
    g.cfg.nmaster = cfg.nmaster;

    g.cfg.windowcolors = cfg.windowcolors.clone();
    g.cfg.closebuttoncolors = cfg.closebuttoncolors.clone();
    g.cfg.bordercolors = cfg.bordercolors.clone();
    g.cfg.statusbarcolors = cfg.statusbarcolors.clone();

    g.cfg.keys = cfg.keys.clone();
    g.cfg.dkeys = cfg.dkeys.clone();
    g.cfg.buttons = cfg.buttons.clone();
    g.cfg.rules = cfg.rules.clone();
    g.cfg.commands = cfg.commands.clone();
    g.cfg.resources = cfg.resources.clone();
    g.cfg.fonts = cfg.fonts.clone();
    g.cfg.external_commands = cfg.external_commands.clone();
    // Rebuild tag template so monitor creation picks up any config changes.
    g.cfg.tag_template = build_tag_template(cfg);
}

/// Build the canonical tag template from config.
///
/// Returns a `Vec<Tag>` that every monitor should clone into its own
/// `tags` field via `Monitor::init_tags`.
pub fn build_tag_template(cfg: &crate::config::Config) -> Vec<crate::types::Tag> {
    let num_tags = cfg.num_tags;
    let mut template = Vec::with_capacity(num_tags);
    for i in 0..num_tags {
        let name = if i < cfg.tag_names.len() {
            cfg.tag_names[i].clone()
        } else {
            format!("{}", i + 1)
        };
        let alt_name = if i < cfg.tag_alt_names.len() {
            cfg.tag_alt_names[i]
        } else {
            ""
        };
        let mut tag = crate::types::Tag::default();
        tag.name = name;
        tag.alt_name = alt_name;
        tag.nmaster = cfg.nmaster;
        tag.mfact = cfg.mfact;
        tag.showbar = cfg.showbar;
        template.push(tag);
    }
    template
}

/// Initialize tags from config.
///
/// Stores `num_tags` in `TagSet` for mask/count helpers, then clones the
/// template into every monitor that has already been created (on first
/// startup there are none yet; `update_geom` will call `init_tags` on each
/// monitor as it creates them).
pub fn init_tags_from_config(cfg: &crate::config::Config) {
    let template = build_tag_template(cfg);
    let g = get_globals_mut();
    g.tags.colors = cfg.tag_colors.clone();
    g.tags.num_tags = cfg.num_tags;
    g.cfg.tag_template = template.clone();
    // Initialise any monitors that already exist (re-init on config reload).
    for mon in g.monitors.iter_mut() {
        mon.init_tags(&template);
    }
}
