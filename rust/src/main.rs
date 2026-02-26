mod animation;
mod backend;
mod bar;
mod client;
mod commands;
mod config;
mod constants;
mod contexts;
mod drw;
mod events;
mod floating;
mod focus;
mod globals;
mod keyboard;
mod layouts;
mod monitor;
mod mouse;
mod overlay;
mod push;
mod scratchpad;
mod systray;
mod tags;
mod toggles;
mod types;
mod util;
mod wm;
mod xresources;

use clap::{Parser, ValueEnum};
use libc::{setlocale, LC_CTYPE};
use std::process::exit;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::backend::x11::X11Backend;
use crate::config::init_config;
use crate::drw::Drw;
use crate::globals::XlibDisplay;
use crate::types::*;
use crate::wm::Wm;
use crate::xresources::list_xresources;

const XC_LEFT_PTR: u32 = 68;
const XC_CROSSHAIR: u32 = 34;
const XC_FLEUR: u32 = 52;
const XC_HAND1: u32 = 58;
const XC_SB_V_DOUBLE_ARROW: u32 = 116;
const XC_SB_H_DOUBLE_ARROW: u32 = 108;
const XC_BOTTOM_LEFT_CORNER: u32 = 12;
const XC_BOTTOM_RIGHT_CORNER: u32 = 14;
const XC_TOP_LEFT_CORNER: u32 = 134;
const XC_TOP_RIGHT_CORNER: u32 = 136;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    X11,
    Wayland,
}

#[derive(Debug, Parser)]
#[command(name = "instantwm", version, disable_help_subcommand = true)]
struct Cli {
    #[arg(short = 'X', long = "xresources")]
    xresources: bool,

    #[arg(long, value_enum, default_value_t = Backend::X11)]
    backend: Backend,
}

fn main() {
    let cli = Cli::parse();

    if cli.xresources {
        list_xresources();
        exit(0);
    }

    if set_locale().is_err() {
        eprintln!("warning: no locale support");
    }

    match cli.backend {
        Backend::X11 => run_x11(),
        Backend::Wayland => run_wayland(),
    }
}

fn run_x11() {
    let (conn, screen_num) = match RustConnection::connect(None) {
        Ok((c, s)) => (c, s),
        Err(_) => {
            eprintln!(
                "instantwm: Failed to open the display from the DISPLAY environment variable.",
            );
            std::process::exit(1);
        }
    };

    let mut wm = Wm::new(X11Backend::new(conn, screen_num));
    wm_init(&mut wm);
    crate::events::setup(&mut wm);
    crate::events::scan(&mut wm);
    run_autostart();
    crate::events::run(&mut wm);
    crate::events::cleanup(&mut wm);
}

#[cfg(feature = "wayland_backend")]
fn run_wayland() -> ! {
    eprintln!("instantwm: Wayland backend is not wired yet.");
    exit(1);
}

#[cfg(not(feature = "wayland_backend"))]
fn run_wayland() -> ! {
    eprintln!(
        "instantwm: Wayland backend requested but not enabled. Rebuild with --features wayland_backend.",
    );
    exit(1);
}

fn set_locale() -> Result<(), ()> {
    unsafe {
        let result = setlocale(LC_CTYPE, b"\0".as_ptr() as *const i8);
        if result.is_null() {
            eprintln!("warning: no locale support");
        }
    }
    Ok(())
}

fn wm_init(wm: &mut Wm) {
    setup_signal_handlers();

    let screen_num = wm.x11.screen_num;
    let screen = wm.x11.conn.setup().roots[screen_num].clone();
    let root = screen.root;

    crate::events::check_other_wm(&wm.x11.conn, root);

    init_globals(wm, screen_num, root, &screen);

    {
        let mut ctx = wm.ctx();
        crate::xresources::load_xresources(&mut ctx);
    }

    let conn = &wm.x11.conn;
    init_atoms(&mut wm.g, conn);
    init_drw_and_schemes(wm);

    // Select events and initialise EWMH bits that depend on atoms + config.
    crate::events::setup_root(wm);

    // After atoms + drw exist, we can verify xresources and create bars.
    {
        let mut ctx = wm.ctx();
        crate::xresources::verify_tags_xres(&mut ctx);
        crate::bar::x11::update_bars(&mut ctx);
        crate::bar::x11::update_status(&mut ctx);
        crate::keyboard::grab_keys(&ctx);
        crate::focus::focus_soft(&mut ctx, None);
    }
}

fn init_globals(
    wm: &mut Wm,
    screen_num: usize,
    root: Window,
    screen: &x11rb::protocol::xproto::Screen,
) {
    let cfg = init_config();

    wm.g.cfg.screen = screen_num as i32;
    wm.g.cfg.root = root;
    wm.g.cfg.screen_width = screen.width_in_pixels as i32;
    wm.g.cfg.screen_height = screen.height_in_pixels as i32;

    crate::globals::update_config_from_config(&cfg);
    crate::globals::init_tags_from_config(&cfg);
}

fn setup_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = libc::SA_NOCLDSTOP | libc::SA_NOCLDWAIT | libc::SA_RESTART;
        libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
    }
}

fn init_atoms(g: &mut crate::globals::Globals, conn: &RustConnection) {
    let _utf8string = intern_atom(conn, "UTF8_STRING", false);

    let wm_protocols = intern_atom(conn, "WM_PROTOCOLS", false);
    let wm_delete = intern_atom(conn, "WM_DELETE_WINDOW", false);
    let wm_state = intern_atom(conn, "WM_STATE", false);
    let wm_take_focus = intern_atom(conn, "WM_TAKE_FOCUS", false);

    let net_active_window = intern_atom(conn, "_NET_ACTIVE_WINDOW", false);
    let net_supported = intern_atom(conn, "_NET_SUPPORTED", false);
    let net_system_tray = intern_atom(conn, "_NET_SYSTEM_TRAY_S0", false);
    let net_system_tray_op = intern_atom(conn, "_NET_SYSTEM_TRAY_OPCODE", false);
    let net_system_tray_orientation = intern_atom(conn, "_NET_SYSTEM_TRAY_ORIENTATION", false);
    let net_system_tray_orientation_horz =
        intern_atom(conn, "_NET_SYSTEM_TRAY_ORIENTATION_HORZ", false);
    let net_wm_name = intern_atom(conn, "_NET_WM_NAME", false);
    let net_wm_state = intern_atom(conn, "_NET_WM_STATE", false);
    let net_wm_check = intern_atom(conn, "_NET_SUPPORTING_WM_CHECK", false);
    let net_wm_fullscreen = intern_atom(conn, "_NET_WM_STATE_FULLSCREEN", false);
    let net_wm_window_type = intern_atom(conn, "_NET_WM_WINDOW_TYPE", false);
    let net_wm_window_type_dialog = intern_atom(conn, "_NET_WM_WINDOW_TYPE_DIALOG", false);
    let net_client_list = intern_atom(conn, "_NET_CLIENT_LIST", false);
    let net_client_info = intern_atom(conn, "_NET_CLIENT_INFO", false);

    let motifatom = intern_atom(conn, "_MOTIF_WM_HINTS", false);

    let xembed_manager = intern_atom(conn, "MANAGER", false);
    let xembed = intern_atom(conn, "_XEMBED", false);
    let xembed_info = intern_atom(conn, "_XEMBED_INFO", false);

    g.cfg.wmatom = crate::types::WmAtoms {
        protocols: wm_protocols,
        delete: wm_delete,
        state: wm_state,
        take_focus: wm_take_focus,
    };
    g.cfg.netatom = crate::types::NetAtoms {
        active_window: net_active_window,
        supported: net_supported,
        system_tray: net_system_tray,
        system_tray_op: net_system_tray_op,
        system_tray_orientation: net_system_tray_orientation,
        system_tray_orientation_horz: net_system_tray_orientation_horz,
        wm_name: net_wm_name,
        wm_state: net_wm_state,
        wm_check: net_wm_check,
        wm_fullscreen: net_wm_fullscreen,
        wm_window_type: net_wm_window_type,
        wm_window_type_dialog: net_wm_window_type_dialog,
        client_list: net_client_list,
        client_info: net_client_info,
    };
    g.cfg.motifatom = motifatom;
    g.cfg.xatom = crate::types::XAtoms {
        manager: xembed_manager,
        xembed,
        xembed_info,
    };
}

fn intern_atom(conn: &RustConnection, name: &str, only_if_exists: bool) -> u32 {
    match conn.intern_atom(only_if_exists, name.as_bytes()) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply.atom,
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

fn init_drw_and_schemes(wm: &mut Wm) {
    let mut drw = match Drw::new(None) {
        Ok(d) => d,
        Err(_) => die("instantwm: cannot create drawing context"),
    };

    let fonts: Vec<&str> = wm.g.cfg.fonts.clone();
    if drw.fontset_create(&fonts).is_err() {
        die("no fonts could be loaded.");
    }

    let font_height = drw.fonts.as_ref().map(|f| f.h).unwrap_or(12);
    let barheight = wm.g.cfg.barheight;
    let bh = if barheight > 0 {
        font_height + barheight as u32
    } else {
        font_height + 12
    };

    init_cursors(wm, &mut drw);
    init_schemes(wm, &mut drw);

    wm.g.cfg.xlibdisplay = XlibDisplay(drw.display());
    wm.g.cfg.drw = Some(drw);
    wm.g.cfg.bar_height = bh as i32;
    wm.g.cfg.horizontal_padding = font_height as i32;
}

fn init_cursors(wm: &mut Wm, drw: &mut Drw) {
    let cursors = [
        drw.cur_create(XC_LEFT_PTR),
        drw.cur_create(XC_CROSSHAIR),
        drw.cur_create(XC_FLEUR),
        drw.cur_create(XC_HAND1),
        drw.cur_create(XC_SB_V_DOUBLE_ARROW),
        drw.cur_create(XC_SB_H_DOUBLE_ARROW),
        drw.cur_create(XC_BOTTOM_LEFT_CORNER),
        drw.cur_create(XC_BOTTOM_RIGHT_CORNER),
        drw.cur_create(XC_TOP_LEFT_CORNER),
        drw.cur_create(XC_TOP_RIGHT_CORNER),
    ];

    for (i, cursor) in cursors.into_iter().enumerate() {
        if i < wm.g.cfg.cursors.len() {
            wm.g.cfg.cursors[i] = Some(cursor);
        }
    }
}

fn init_schemes(wm: &mut Wm, drw: &mut Drw) {
    let bordercolors: Vec<&str> = wm.g.cfg.bordercolors.clone();
    let statusbarcolors: Vec<&str> = wm.g.cfg.statusbarcolors.clone();
    let tagcolors: Vec<Vec<Vec<&str>>> = wm.g.tags.colors.clone();
    let windowcolors: Vec<Vec<Vec<&str>>> = wm.g.cfg.windowcolors.clone();
    let closebuttoncolors: Vec<Vec<Vec<&str>>> = wm.g.cfg.closebuttoncolors.clone();

    const BORDER_NORMAL: usize = 0;
    const BORDER_TILE_FOCUS: usize = 1;
    const BORDER_FLOAT_FOCUS: usize = 2;
    const BORDER_SNAP: usize = 3;

    let borderscheme = drw.scm_create(&bordercolors).ok().map(|clr| {
        if clr.len() >= 4 {
            BorderScheme {
                normal: ColorScheme::new(
                    clr[BORDER_NORMAL].clone(),
                    clr[BORDER_NORMAL].clone(),
                    clr[BORDER_NORMAL].clone(),
                ),
                tile_focus: ColorScheme::new(
                    clr[BORDER_TILE_FOCUS].clone(),
                    clr[BORDER_TILE_FOCUS].clone(),
                    clr[BORDER_TILE_FOCUS].clone(),
                ),
                float_focus: ColorScheme::new(
                    clr[BORDER_FLOAT_FOCUS].clone(),
                    clr[BORDER_FLOAT_FOCUS].clone(),
                    clr[BORDER_FLOAT_FOCUS].clone(),
                ),
                snap: ColorScheme::new(
                    clr[BORDER_SNAP].clone(),
                    clr[BORDER_SNAP].clone(),
                    clr[BORDER_SNAP].clone(),
                ),
            }
        } else {
            BorderScheme::default()
        }
    });

    let statusscheme = drw
        .scm_create(&statusbarcolors)
        .ok()
        .map(|clr| StatusScheme::new(clr[0].clone(), clr[1].clone(), clr[2].clone()));

    let mut tagschemes_no_hover: Vec<ColorScheme> = Vec::new();
    let mut tagschemes_hover: Vec<ColorScheme> = Vec::new();

    if let Some(no_hover) = tagcolors.first() {
        for scheme_colors in no_hover {
            if let Ok(clr) = drw.scm_create(scheme_colors) {
                if let Some(cs) = ColorScheme::from_vec(clr) {
                    tagschemes_no_hover.push(cs);
                }
            }
        }
    }

    if let Some(hover) = tagcolors.get(1) {
        for scheme_colors in hover {
            if let Ok(clr) = drw.scm_create(scheme_colors) {
                if let Some(cs) = ColorScheme::from_vec(clr) {
                    tagschemes_hover.push(cs);
                }
            }
        }
    }

    let tagschemes = TagSchemes {
        no_hover: tagschemes_no_hover,
        hover: tagschemes_hover,
    };

    let mut windowschemes_no_hover: Vec<ColorScheme> = Vec::new();
    let mut windowschemes_hover: Vec<ColorScheme> = Vec::new();

    if let Some(no_hover) = windowcolors.first() {
        for scheme_colors in no_hover {
            if let Ok(clr) = drw.scm_create(scheme_colors) {
                if let Some(cs) = ColorScheme::from_vec(clr) {
                    windowschemes_no_hover.push(cs);
                }
            }
        }
    }

    if let Some(hover) = windowcolors.get(1) {
        for scheme_colors in hover {
            if let Ok(clr) = drw.scm_create(scheme_colors) {
                if let Some(cs) = ColorScheme::from_vec(clr) {
                    windowschemes_hover.push(cs);
                }
            }
        }
    }

    let windowschemes = WindowSchemes {
        no_hover: windowschemes_no_hover,
        hover: windowschemes_hover,
    };

    let mut closebuttonschemes_no_hover: Vec<ColorScheme> = Vec::new();
    let mut closebuttonschemes_hover: Vec<ColorScheme> = Vec::new();

    if let Some(no_hover) = closebuttoncolors.first() {
        for scheme_colors in no_hover {
            if let Ok(clr) = drw.scm_create(scheme_colors) {
                if let Some(cs) = ColorScheme::from_vec(clr) {
                    closebuttonschemes_no_hover.push(cs);
                }
            }
        }
    }

    if let Some(hover) = closebuttoncolors.get(1) {
        for scheme_colors in hover {
            if let Ok(clr) = drw.scm_create(scheme_colors) {
                if let Some(cs) = ColorScheme::from_vec(clr) {
                    closebuttonschemes_hover.push(cs);
                }
            }
        }
    }

    let closebuttonschemes = CloseButtonSchemes {
        no_hover: closebuttonschemes_no_hover,
        hover: closebuttonschemes_hover,
    };

    wm.g.cfg.borderscheme = borderscheme;
    wm.g.cfg.statusscheme = statusscheme;
    wm.g.tags.schemes = tagschemes;
    wm.g.cfg.windowschemes = windowschemes;
    wm.g.cfg.closebuttonschemes = closebuttonschemes;
}

fn run_autostart() {
    unsafe {
        match libc::fork() {
            -1 => {
                eprintln!("instantwm: fork failed for autostart");
            }
            0 => {
                libc::setsid();

                let _ = libc::system(
                    b"command -v instantautostart || { sleep 4 && notify-send 'instantutils missing, please install instantutils!!!'; } &\0"
                        .as_ptr() as *const i8,
                );
                let _ = libc::system(b"instantautostart &\0".as_ptr() as *const i8);

                libc::_exit(0);
            }
            _ => {}
        }
    }
}
