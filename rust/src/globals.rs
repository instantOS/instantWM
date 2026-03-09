use crate::client::manager::ClientManager;
use crate::config::commands::ExternalCommands;
use crate::drw::{Cursor, Drw};
use crate::monitor::MonitorManager;
use crate::types::color::{BorderScheme, StatusScheme};
use crate::types::*;
use std::sync::atomic::AtomicBool;
use x11rb::protocol::xproto::Window;

#[derive(Clone, Copy)]
pub struct XlibDisplay(pub *mut libc::c_void);
unsafe impl Send for XlibDisplay {}
unsafe impl Sync for XlibDisplay {}

/// X11-specific runtime configuration.
/// These fields are only meaningful on X11 and are left as defaults/zero on Wayland/DRM.
#[derive(Clone)]
pub struct X11RuntimeConfig {
    pub wmatom: WmAtoms,
    pub netatom: NetAtoms,
    pub xatom: XAtoms,
    pub motifatom: Atom,
    pub numlockmask: u32,
    pub screen: i32,
    pub root: Window,
    /// The small 1×1 window for _NET_SUPPORTING_WM_CHECK (EWMH).
    pub wmcheckwin: Window,
    pub xlibdisplay: XlibDisplay,
    pub drw: Option<Drw>,
    /// X11 color schemes for borders (different states: normal, tile focus, float focus, snap).
    pub borderscheme: crate::types::color::BorderScheme,
    /// X11 color scheme for status bar.
    pub statusscheme: crate::types::color::StatusScheme,
}

impl Default for X11RuntimeConfig {
    fn default() -> Self {
        Self {
            wmatom: WmAtoms::default(),
            netatom: NetAtoms::default(),
            xatom: XAtoms::default(),
            motifatom: 0,
            numlockmask: 0,
            screen: 0,
            root: 0,
            wmcheckwin: 0,
            xlibdisplay: XlibDisplay(std::ptr::null_mut()),
            drw: None,
            borderscheme: BorderScheme::default(),
            statusscheme: StatusScheme::default(),
        }
    }
}

/// Runtime configuration - values loaded from config
/// These are set during initialization and updated on reload
#[derive(Clone)]
pub struct RuntimeConfig {
    // Screen/Display info
    pub screen_width: i32,
    pub screen_height: i32,

    // Window manager configuration
    pub border_width_px: i32,
    pub snap: i32,
    pub startmenusize: i32,
    pub resizehints: i32,
    pub decorhints: i32,
    pub mfact: f32,
    pub nmaster: i32,
    pub show_bar: bool,
    pub topbar: bool,
    pub bar_height: i32,
    pub show_systray: bool,
    pub systraypinning: usize,
    pub systrayspacing: i32,

    // Raw color values for config (parsed at load time)
    pub windowcolors: WindowColorConfigs,
    pub closebuttoncolors: CloseButtonColorConfigs,
    pub bordercolors: BorderColorConfig,
    pub statusbarcolors: StatusColorConfig,

    // Bindings
    pub keys: Vec<Key>,
    pub desktop_keybinds: Vec<Key>,
    pub buttons: Vec<Button>,
    pub rules: Vec<Rule>,

    pub fonts: Vec<String>,
    pub config_font: String,
    pub instantmenumon: String,

    // External commands
    pub external_commands: ExternalCommands,

    // Cursors
    pub cursors: [Option<Cursor>; 10],
    pub horizontal_padding: i32,
    /// Template tag list cloned into every new monitor.
    pub tag_template: Vec<crate::types::Tag>,

    // Input configuration
    pub input: std::collections::HashMap<String, crate::config::config_toml::InputConfig>,

    // Status command
    pub status_command: Option<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            screen_width: 0,
            screen_height: 0,
            border_width_px: 1,
            snap: 32,
            startmenusize: 0,
            resizehints: 1,
            decorhints: 0,
            mfact: 0.55,
            nmaster: 1,
            show_bar: true,
            topbar: true,
            bar_height: 0,
            show_systray: true,
            systraypinning: 0,
            systrayspacing: 2,
            windowcolors: WindowColorConfigs::default(),
            closebuttoncolors: CloseButtonColorConfigs::default(),
            bordercolors: BorderColorConfig::default(),
            statusbarcolors: StatusColorConfig::default(),
            keys: Vec::new(),
            desktop_keybinds: Vec::new(),
            buttons: Vec::new(),
            rules: Vec::new(),
            fonts: Vec::new(),
            config_font: String::new(),
            instantmenumon: String::new(),
            external_commands: crate::config::commands::default_commands(),
            cursors: [const { None }; 10],
            horizontal_padding: 0,
            tag_template: Vec::new(),
            input: std::collections::HashMap::new(),
            status_command: None,
        }
    }
}

/// State for an in-progress tag-bar drag (backend-agnostic).
///
/// State for an async window-title click/drag on the bar.
///
/// On X11, `window_title_mouse_handler` runs a synchronous grab loop.
/// On Wayland, this state machine is driven by the calloop's pointer
/// motion and button release events.
#[derive(Debug, Clone, Default)]
pub struct TitleDragState {
    /// Whether a title drag is currently active.
    pub active: bool,
    /// The window whose title was clicked.
    pub win: WindowId,
    /// The mouse button that started the interaction.
    pub button: MouseButton,
    /// Whether the window was focused when the click started.
    pub was_focused: bool,
    /// Whether the window was hidden when the click started.
    pub was_hidden: bool,
    /// Anchor X position (root coords) at press time.
    pub start_x: i32,
    /// Anchor Y position (root coords) at press time.
    pub start_y: i32,
    /// Window geometry at press time.
    pub win_start_geo: Rect,
    /// Geometry to persist when a drag is dropped on the bar and re-tiled.
    pub drop_restore_geo: Rect,
    /// Last pointer X seen for this interaction (root coords).
    pub last_root_x: i32,
    /// Last pointer Y seen for this interaction (root coords).
    pub last_root_y: i32,
    /// Whether the drag threshold has been exceeded.
    pub dragging: bool,
    /// Skip bar-title click semantics on release (used for CSD move requests).
    pub suppress_click_action: bool,
}

/// State for Wayland hover-border move/resize interactions.
#[derive(Debug, Clone)]
pub struct HoverResizeDragState {
    /// Whether a hover-border drag is currently active.
    pub active: bool,
    /// Target window being moved/resized.
    pub win: WindowId,
    /// Mouse button that started the interaction.
    pub button: MouseButton,
    /// Resize direction chosen at press time.
    pub direction: ResizeDirection,
    /// `true` for move mode, `false` for resize mode.
    pub move_mode: bool,
    /// Pointer anchor in root coords at press time.
    pub start_x: i32,
    /// Pointer anchor in root coords at press time.
    pub start_y: i32,
    /// Window geometry at press time.
    pub win_start_geo: Rect,
    /// Last pointer position seen for this interaction.
    pub last_root_x: i32,
    /// Last pointer position seen for this interaction.
    pub last_root_y: i32,
}

impl Default for HoverResizeDragState {
    fn default() -> Self {
        Self {
            active: false,
            win: WindowId::default(),
            button: MouseButton::Left,
            direction: ResizeDirection::BottomRight,
            move_mode: false,
            start_x: 0,
            start_y: 0,
            win_start_geo: Rect::default(),
            last_root_x: 0,
            last_root_y: 0,
        }
    }
}

/// On X11, the synchronous grab loop drives this. On Wayland, the calloop
/// press/motion/release events drive it asynchronously.
#[derive(Debug, Clone, Default)]
pub struct TagDragState {
    /// Whether a tag drag is currently active.
    pub active: bool,
    /// The initial tag bitmask that was clicked.
    pub initial_tag: u32,
    /// Monitor ID where the drag started.
    pub monitor_id: usize,
    /// Monitor X origin (for converting root coords to local).
    pub mon_mx: i32,
    /// Last seen tag gesture index (-1 = none).
    pub last_tag: i32,
    /// Whether cursor is still on the bar.
    pub cursor_on_bar: bool,
    /// Last motion coordinates + modifier state (for release handling).
    pub last_motion: Option<(i32, i32, u32)>,
    /// The mouse button that started the drag.
    pub button: MouseButton,
}

/// Consolidated state for mouse/touch interactions.
#[derive(Debug, Clone, Default)]
pub struct DragState {
    pub tag: TagDragState,
    pub title: TitleDragState,
    pub hover_resize: HoverResizeDragState,
    pub bar_active: bool,
    pub resize_direction: Option<ResizeDirection>,
    /// Last cursor index applied to the X11 root cursor.
    pub last_x11_cursor_index: Option<usize>,
}

/// Keyboard (XKB) layout runtime state.
#[derive(Debug, Clone, Default)]
pub struct KeyboardLayoutState {
    /// Configured XKB layout names, e.g. `["us", "de", "fr"]`.
    pub layouts: Vec<String>,
    /// Per-layout XKB variants (parallel to `layouts`).
    pub variants: Vec<String>,
    /// XKB options string.
    pub options: Option<String>,
    /// XKB model string.
    pub model: Option<String>,
    /// Index of the currently active layout in `layouts`.
    pub current: usize,
}

impl KeyboardLayoutState {
    /// The currently active layout name, or `None` if no layouts are configured.
    pub fn current_layout(&self) -> Option<&str> {
        self.layouts.get(self.current).map(|s| s.as_str())
    }

    /// The variant for the currently active layout, or empty string.
    pub fn current_variant(&self) -> &str {
        self.variants
            .get(self.current)
            .map(|s| s.as_str())
            .unwrap_or("")
    }
}

pub struct Globals {
    // Runtime configuration (loaded from config files)
    pub cfg: RuntimeConfig,

    // Runtime state (changes during WM operation)
    pub monitors: MonitorManager,
    pub clients: ClientManager,
    pub tags: TagSet,

    // Runtime flags
    pub animated: bool,
    pub focus_follows_mouse: bool,
    pub focusfollowsfloatmouse: bool,
    pub altcursor: AltCursor,
    pub doubledraw: bool,
    pub specialnext: SpecialNext,
    pub drag: DragState,
    pub status_text_width: i32,
    pub status_text: String,

    /// XKB keyboard layout state.
    pub keyboard_layout: KeyboardLayoutState,
}

impl Globals {
    // -------------------------------------------------------------------------
    // Selected-monitor convenience helpers
    // -------------------------------------------------------------------------

    /// Return the window currently selected on the selected monitor, if any.
    #[inline]
    pub fn selected_win(&self) -> Option<WindowId> {
        self.monitors.sel().and_then(|m| m.sel)
    }

    /// Return the ID of the currently selected monitor.
    pub fn selected_monitor_id(&self) -> usize {
        self.monitors.sel_idx()
    }

    /// Change the currently selected monitor.
    pub fn set_selected_monitor(&mut self, id: usize) {
        self.monitors.set_sel_idx(id);
    }

    /// Shorthand to get the selected monitor.
    pub fn selected_monitor(&self) -> &Monitor {
        self.monitors.sel_unchecked()
    }

    /// Shorthand to get the selected monitor mutably.
    pub fn selected_monitor_mut(&mut self) -> &mut Monitor {
        self.monitors.sel_mut_unchecked()
    }

    /// Shorthand to get the selected monitor (Option version for cases that need it).
    pub fn selected_monitor_opt(&self) -> Option<&Monitor> {
        self.monitors.sel()
    }

    /// Shorthand to get the selected monitor mutably (Option version).
    pub fn selected_monitor_mut_opt(&mut self) -> Option<&mut Monitor> {
        self.monitors.sel_mut()
    }

    /// Delegation to get a monitor by index.
    pub fn monitor(&self, id: usize) -> Option<&Monitor> {
        self.monitors.get(id)
    }

    /// Delegation to get a mutable monitor by index.
    pub fn monitor_mut(&mut self, id: usize) -> Option<&mut Monitor> {
        self.monitors.get_mut(id)
    }

    /// Delegation to iterate over monitors.
    pub fn monitors_iter(&self) -> impl Iterator<Item = (usize, &Monitor)> {
        self.monitors.iter()
    }

    /// Iterate over all monitors (without index).
    pub fn monitors_iter_all(&self) -> impl Iterator<Item = &Monitor> {
        self.monitors.iter_all()
    }

    /// Delegation to iterate over monitors mutably.
    pub fn monitors_iter_mut(&mut self) -> impl Iterator<Item = (usize, &mut Monitor)> {
        self.monitors.iter_mut()
    }

    /// Iterate over all monitors mutably (without index).
    pub fn monitors_iter_all_mut(&mut self) -> impl Iterator<Item = &mut Monitor> {
        self.monitors.iter_all_mut()
    }

    // -------------------------------------------------------------------------
    // Client List Management (Attach/Detach)
    // -------------------------------------------------------------------------

    /// Attach `win` to its assigned monitor's focus list.
    pub fn attach(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.get(&win).and_then(|c| c.monitor_id) {
            if let Some(mon) = self.monitors.get_mut(mid) {
                mon.clients.insert(0, win);
            }
        }
    }

    /// Detach `win` from its assigned monitor's focus list.
    pub fn detach(&mut self, win: WindowId) {
        let monitor_id = self.clients.get(&win).and_then(|c| c.monitor_id);
        if let Some(mid) = monitor_id {
            if let Some(mon) = self.monitors.get_mut(mid) {
                if mon.clients.contains(&win) {
                    mon.clients.retain(|&w| w != win);
                    return;
                }
            }
        }

        // Fallback: search all monitors if not found on the assigned one.
        for mon in self.monitors.iter_all_mut() {
            if mon.clients.contains(&win) {
                mon.clients.retain(|&w| w != win);
            }
        }
    }

    /// Attach `win` to its assigned monitor's stacking list.
    pub fn attach_stack(&mut self, win: WindowId) {
        if let Some(mid) = self.clients.get(&win).and_then(|c| c.monitor_id) {
            if let Some(mon) = self.monitors.get_mut(mid) {
                mon.stack.insert(0, win);
                if mon.sel.is_none() {
                    mon.sel = Some(win);
                }
            }
        }
    }

    /// Detach `win` from its assigned monitor's stacking list.
    pub fn detach_stack(&mut self, win: WindowId) {
        let monitor_id = self.clients.get(&win).and_then(|c| c.monitor_id);
        if let Some(mid) = monitor_id {
            if let Some(mon) = self.monitors.get_mut(mid) {
                if mon.stack.contains(&win) {
                    mon.stack.retain(|&w| w != win);
                    if mon.sel == Some(win) {
                        mon.sel = mon.stack.first().copied();
                    }
                    return;
                }
            }
        }

        // Fallback: search all monitors if not found on the assigned one.
        for mon in self.monitors.iter_all_mut() {
            if mon.stack.contains(&win) {
                mon.stack.retain(|&w| w != win);
                if mon.sel == Some(win) {
                    mon.sel = mon.stack.first().copied();
                }
            }
        }
    }
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            cfg: RuntimeConfig::default(),
            monitors: MonitorManager::new(),
            clients: ClientManager::new(),
            tags: TagSet::default(),
            animated: true,
            focus_follows_mouse: true,
            focusfollowsfloatmouse: true,
            altcursor: AltCursor::None,
            doubledraw: false,
            specialnext: SpecialNext::None,
            drag: DragState::default(),
            status_text_width: 0,
            status_text: String::new(),
            keyboard_layout: KeyboardLayoutState::default(),
        }
    }
}

pub static RUNNING: AtomicBool = AtomicBool::new(true);

/// Storage for the X11 connection during initialization and shutdown.
/// After initialization, use [`crate::backend::x11::X11BackendRef`] which guarantees
/// the connection exists.
#[derive(Default)]
pub struct X11Connection {
    pub conn: Option<x11rb::rust_connection::RustConnection>,
    pub screen_num: usize,
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

    /// Create a borrowed X11 handle from this connection.
    ///
    /// # Panics
    ///
    /// Panics if the connection is not available.
    pub fn as_ref(&self) -> crate::backend::x11::X11BackendRef<'_> {
        crate::backend::x11::X11BackendRef::new(self.conn(), self.screen_num)
    }
}

/// Apply config values to the given `Globals` instance.
pub fn apply_config(g: &mut Globals, cfg: &crate::config::Config) {
    g.cfg.border_width_px = cfg.borderpx;
    g.cfg.input = cfg.input.clone();
    g.cfg.snap = cfg.snap;
    g.cfg.startmenusize = cfg.startmenusize;
    g.cfg.systraypinning = cfg.systraypinning;
    g.cfg.systrayspacing = cfg.systrayspacing;
    g.cfg.show_systray = cfg.showsystray;
    g.cfg.show_bar = cfg.showbar;
    g.cfg.topbar = cfg.topbar;
    g.cfg.bar_height = cfg.bar_height;
    g.cfg.resizehints = cfg.resizehints;
    g.cfg.decorhints = cfg.decorhints;
    g.cfg.mfact = cfg.mfact;
    g.cfg.nmaster = cfg.nmaster;

    g.cfg.windowcolors = cfg.windowcolors.clone();
    g.cfg.closebuttoncolors = cfg.closebuttoncolors.clone();
    g.cfg.bordercolors = cfg.bordercolors.clone();
    g.cfg.statusbarcolors = cfg.statusbarcolors.clone();

    g.cfg.keys = cfg.keys.clone();
    g.cfg.desktop_keybinds = cfg.desktop_keybinds.clone();
    g.cfg.buttons = cfg.buttons.clone();
    g.cfg.rules = cfg.rules.clone();
    g.cfg.fonts = cfg.fonts.clone();
    g.cfg.external_commands = cfg.external_commands.clone();
    g.cfg.status_command = cfg.status_command.clone();

    // Initialize keyboard layout state from config
    if !cfg.keyboard_layouts.is_empty() {
        g.keyboard_layout = KeyboardLayoutState {
            layouts: cfg.keyboard_layouts.clone(),
            variants: cfg.keyboard_variants.clone(),
            options: cfg.keyboard_options.clone(),
            model: cfg.keyboard_model.clone(),
            current: 0,
        };
    }

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
            cfg.tag_alt_names[i].clone()
        } else {
            String::new()
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

/// Apply tag configuration to the given `Globals` instance.
pub fn apply_tags_config(g: &mut Globals, cfg: &crate::config::Config) {
    let template = build_tag_template(cfg);
    g.tags.colors = cfg.tag_colors.clone();
    g.tags.num_tags = cfg.num_tags;
    g.cfg.tag_template = template.clone();
    // Initialise any monitors that already exist (re-init on config reload).
    for (_i, mon) in g.monitors.iter_mut() {
        mon.init_tags(&template);
    }
}

impl Globals {
    /// Get the status bar color scheme.
    pub fn status_scheme(&self) -> crate::bar::paint::BarScheme {
        let c = &self.cfg.statusbarcolors;
        crate::bar::paint::BarScheme {
            fg: c.fg,
            bg: c.bg,
            detail: c.detail,
        }
    }

    /// Get the tag hover fill scheme.
    pub fn tag_hover_fill_scheme(&self) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeHover, SchemeTag};

        let colors = self
            .tags
            .colors
            .scheme(SchemeHover::Hover, SchemeTag::Filled);
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Get the color scheme for a tag.
    pub fn tag_scheme(
        &self,
        m: &Monitor,
        tag_index: u32,
        occupied_tags: u32,
        is_hover: bool,
    ) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeHover, SchemeTag};

        let scheme_idx = if occupied_tags & (1 << tag_index) != 0 {
            let selmon = self.selected_monitor();
            let sel_has_tag = selmon
                .sel
                .and_then(|selected_window| {
                    self.clients
                        .get(&selected_window)
                        .map(|c| c.tags & (1 << tag_index) != 0)
                })
                .unwrap_or(false);

            let is_selected = selmon.num == m.num;

            if is_selected && sel_has_tag {
                SchemeTag::Focus
            } else if m.selected_tags() & (1 << tag_index) != 0 {
                SchemeTag::NoFocus
            } else if m.showtags == 0 {
                SchemeTag::Filled
            } else {
                SchemeTag::Inactive
            }
        } else if m.selected_tags() & (1 << tag_index) != 0 {
            SchemeTag::Empty
        } else {
            SchemeTag::Inactive
        };

        let colors = self.tags.colors.scheme(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            scheme_idx,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Get the color scheme for a client window.
    pub fn window_scheme(&self, c: &Client, is_hover: bool) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeHover, SchemeWin};

        let selmon = self.selected_monitor();
        let is_selected = selmon.sel == Some(c.win);
        let is_overlay = selmon.overlay == Some(c.win);

        let scheme_idx = if is_selected {
            if is_overlay {
                SchemeWin::OverlayFocus
            } else if c.issticky {
                SchemeWin::StickyFocus
            } else {
                SchemeWin::Focus
            }
        } else if is_overlay {
            SchemeWin::Overlay
        } else if c.issticky {
            SchemeWin::Sticky
        } else if c.is_hidden {
            SchemeWin::Minimized
        } else {
            SchemeWin::Normal
        };

        let colors = self.cfg.windowcolors.scheme(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            scheme_idx,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    /// Get the close button color scheme.
    pub fn close_button_scheme(
        &self,
        is_hover: bool,
        is_locked: bool,
        is_fullscreen: bool,
    ) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeClose, SchemeHover};

        let scheme_idx = if is_locked {
            SchemeClose::Locked
        } else if is_fullscreen {
            SchemeClose::Fullscreen
        } else {
            SchemeClose::Normal
        };

        let colors = self.cfg.closebuttoncolors.scheme(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            scheme_idx,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }
}
