mod animation;
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
mod xresources;

use std::env;
use std::process::exit;
use std::sync::atomic::Ordering;

use libc::{setlocale, LC_CTYPE};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as X11rbConnectionExt;

use crate::bar::x11::{update_bars, update_status};
use crate::config::init_config;
use crate::drw::Drw;
use crate::events::{cleanup, run, scan};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11, get_x11_mut, XlibDisplay, RUNNING};
use crate::keyboard::grab_keys;
use crate::monitor::update_geom;
use crate::types::*;
use crate::util::die;
use crate::xresources::{list_xresources, load_xresources, verify_tags_xres};

const VERSION: &str = env!("CARGO_PKG_VERSION");

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

pub fn quit() {
    RUNNING.store(false, Ordering::SeqCst);
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() == 2 {
        if args[1] == "-V" || args[1] == "--version" {
            println!("instantwm-{}", VERSION);
            exit(0);
        }
        if args[1] == "-X" || args[1] == "--xresources" {
            list_xresources();
            exit(0);
        }
        die("usage: instantwm [-VX]");
    } else if args.len() > 2 {
        die("usage: instantwm [-VX]");
    }

    if set_locale().is_err() {
        eprintln!("warning: no locale support");
    }

    let (conn, screen_num) = match RustConnection::connect(None) {
        Ok((c, s)) => (c, s),
        Err(_) => {
            eprintln!(
                "instantwm: Failed to open the display from the DISPLAY environment variable."
            );
            std::process::exit(1);
        }
    };

    {
        let x11 = get_x11_mut();
        x11.conn = Some(conn);
        x11.screen_num = screen_num;
    }

    let screen = {
        let x11 = get_x11();
        let conn_ref = x11.conn.as_ref().unwrap();
        conn_ref.setup().roots[screen_num].clone()
    };
    let root = screen.root;

    check_other_wm_init(root);

    init_globals(screen_num, root, &screen);
    {
        let mut globals = get_globals_mut();
        let x11 = get_x11().as_conn();
        let mut ctx = crate::contexts::WmCtx::new(&mut globals, x11);
        load_xresources(&mut ctx);
    }
    setup(screen_num, root, &screen);
    scan();
    run_autostart();
    run();
    cleanup();

    {
        let x11 = get_x11_mut();
        x11.conn = None;
    }
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

fn check_other_wm_init(root: Window) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return;
    };
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;

    let result =
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));

    if let Ok(cookie) = result {
        if cookie.check().is_err() {
            eprintln!("instantwm: another window manager is already running");
            std::process::exit(1);
        }
    } else {
        eprintln!("instantwm: another window manager is already running");
        std::process::exit(1);
    }

    let _ = conn.flush();
}

fn init_globals(screen_num: usize, root: Window, screen: &x11rb::protocol::xproto::Screen) {
    let cfg = init_config();

    // Initialize screen info
    {
        let globals = get_globals_mut();
        globals.cfg.screen = screen_num as i32;
        globals.cfg.root = root;
        globals.cfg.sw = screen.width_in_pixels as i32;
        globals.cfg.sh = screen.height_in_pixels as i32;
    }

    // Update runtime config from config
    crate::globals::update_config_from_config(&cfg);

    // Initialize tags from config
    crate::globals::init_tags_from_config(&cfg);
}

fn setup(screen_num: usize, root: Window, _screen: &x11rb::protocol::xproto::Screen) {
    setup_signal_handlers();

    while unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } > 0 {}

    let mut drw = match Drw::new(None) {
        Ok(d) => d,
        Err(_) => die("instantwm: cannot create drawing context"),
    };

    let fonts: Vec<&str> = {
        let g = get_globals();
        g.cfg.fonts.clone()
    };

    if drw.fontset_create(&fonts).is_err() {
        die("no fonts could be loaded.");
    }

    let font_height = drw.fonts.as_ref().map(|f| f.h).unwrap_or(12);

    let barheight = {
        let g = get_globals();
        g.cfg.barheight
    };

    let bh = if barheight > 0 {
        font_height + barheight as u32
    } else {
        font_height + 12
    };

    {
        let x11 = get_x11();
        let Some(ref conn) = x11.conn else {
            return;
        };
        init_atoms(conn);
    }

    init_cursors(&mut drw);
    init_schemes(&mut drw);

    {
        let globals = get_globals_mut();
        globals.cfg.xlibdisplay = XlibDisplay(drw.display());
        globals.cfg.drw = Some(drw);
        globals.cfg.bh = bh as i32;
        globals.cfg.lrpad = font_height as i32;
    }

    update_geom();
    {
        let mut globals = get_globals_mut();
        let x11 = get_x11().as_conn();
        let mut ctx = crate::contexts::WmCtx::new(&mut globals, x11);
        verify_tags_xres(&mut ctx);
        update_bars(&mut ctx);
        update_status(&mut ctx);
    }

    {
        let x11 = get_x11();
        let Some(ref conn) = x11.conn else {
            return;
        };
        init_wm_check_window(conn, screen_num, root);
    }

    let mask = EventMask::SUBSTRUCTURE_REDIRECT
        | EventMask::SUBSTRUCTURE_NOTIFY
        | EventMask::BUTTON_PRESS
        | EventMask::POINTER_MOTION
        | EventMask::ENTER_WINDOW
        | EventMask::LEAVE_WINDOW
        | EventMask::STRUCTURE_NOTIFY
        | EventMask::PROPERTY_CHANGE
        | EventMask::KEY_PRESS
        | EventMask::KEY_RELEASE;

    let cursor = {
        let g = get_globals();
        g.cfg.cursors[0].as_ref().map(|c| c.cursor).unwrap_or(0)
    };

    {
        let x11 = get_x11();
        let Some(ref conn) = x11.conn else {
            return;
        };
        let _ = conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .event_mask(mask)
                .cursor(cursor),
        );
        let _ = conn.flush();
    }

    {
        let mut globals = get_globals_mut();
        let x11 = get_x11().as_conn();
        let mut ctx = crate::contexts::WmCtx::new(&mut globals, x11);
        grab_keys(&ctx);
        focus(&mut ctx, None);
    }
}

fn setup_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = libc::SA_NOCLDSTOP | libc::SA_NOCLDWAIT | libc::SA_RESTART;
        libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
    }
}

fn init_atoms<C: Connection>(conn: &C) {
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

    let globals = get_globals_mut();
    globals.cfg.wmatom = crate::types::WmAtoms {
        protocols: wm_protocols,
        delete: wm_delete,
        state: wm_state,
        take_focus: wm_take_focus,
    };
    globals.cfg.netatom = crate::types::NetAtoms {
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
    globals.cfg.motifatom = motifatom;
    globals.cfg.xatom = crate::types::XAtoms {
        manager: xembed_manager,
        xembed,
        xembed_info,
    };
}

fn intern_atom<C: Connection>(conn: &C, name: &str, only_if_exists: bool) -> u32 {
    match conn.intern_atom(only_if_exists, name.as_bytes()) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply.atom,
            Err(_) => 0,
        },
        Err(_) => 0,
    }
}

fn init_cursors(drw: &mut Drw) {
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

    let globals = get_globals_mut();
    for (i, cursor) in cursors.into_iter().enumerate() {
        if i < globals.cfg.cursors.len() {
            globals.cfg.cursors[i] = Some(cursor);
        }
    }
}

fn init_schemes(drw: &mut Drw) {
    let bordercolors: Vec<&str> = {
        let g = get_globals();
        g.cfg.bordercolors.clone()
    };
    let statusbarcolors: Vec<&str> = {
        let g = get_globals();
        g.cfg.statusbarcolors.clone()
    };
    let tagcolors: Vec<Vec<Vec<&str>>> = {
        let g = get_globals();
        g.tags.colors.clone()
    };
    let windowcolors: Vec<Vec<Vec<&str>>> = {
        let g = get_globals();
        g.cfg.windowcolors.clone()
    };
    let closebuttoncolors: Vec<Vec<Vec<&str>>> = {
        let g = get_globals();
        g.cfg.closebuttoncolors.clone()
    };

    let borderscheme = drw.scm_create(&bordercolors).ok().map(|clr| {
        if clr.len() >= 4 {
            BorderScheme {
                normal: ColorScheme::new(clr[0].clone(), clr[1].clone(), clr[1].clone()),
                tile_focus: ColorScheme::new(clr[0].clone(), clr[1].clone(), clr[1].clone()),
                float_focus: ColorScheme::new(clr[0].clone(), clr[2].clone(), clr[2].clone()),
                snap: ColorScheme::new(clr[0].clone(), clr[3].clone(), clr[3].clone()),
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

    let globals = get_globals_mut();
    globals.cfg.borderscheme = borderscheme;
    globals.cfg.statusscheme = statusscheme;
    globals.tags.schemes = tagschemes;
    globals.cfg.windowschemes = windowschemes;
    globals.cfg.closebuttonschemes = closebuttonschemes;
}

fn init_wm_check_window<C: Connection>(conn: &C, _screen_num: usize, root: Window) {
    let wmcheckwin = conn.generate_id().ok().unwrap_or(0);

    if wmcheckwin == 0 {
        return;
    }

    let _ = conn.create_window(
        0u8,
        wmcheckwin,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    );

    let (net_wm_check, net_wm_name, net_supported, net_atoms, net_client_list, net_client_info) = {
        let g = get_globals();
        let na = g.cfg.netatom;
        let net_atoms = vec![
            na.active_window,
            na.supported,
            na.system_tray,
            na.system_tray_op,
            na.system_tray_orientation,
            na.system_tray_orientation_horz,
            na.wm_name,
            na.wm_state,
            na.wm_check,
            na.wm_fullscreen,
            na.wm_window_type,
            na.wm_window_type_dialog,
            na.client_list,
            na.client_info,
        ];
        (
            na.wm_check,
            na.wm_name,
            na.supported,
            net_atoms,
            na.client_list,
            na.client_info,
        )
    };

    let _ = conn.change_property32(
        PropMode::REPLACE,
        wmcheckwin,
        net_wm_check,
        u32::from(AtomEnum::WINDOW),
        &[wmcheckwin],
    );

    let _ = conn.change_property(
        PropMode::REPLACE,
        wmcheckwin,
        net_wm_name,
        AtomEnum::STRING,
        8u8,
        3u32,
        b"dwm",
    );

    let _ = conn.change_property32(
        PropMode::REPLACE,
        root,
        net_wm_check,
        u32::from(AtomEnum::WINDOW),
        &[wmcheckwin],
    );

    let _ = conn.change_property32(
        PropMode::REPLACE,
        root,
        net_supported,
        u32::from(AtomEnum::ATOM),
        &net_atoms,
    );

    let _ = conn.delete_property(root, net_client_list);
    let _ = conn.delete_property(root, net_client_info);

    let _ = conn.flush();
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
