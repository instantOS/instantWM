use std::process::exit;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::backend::x11::X11Backend;
use crate::backend::Backend as WmBackend;
use crate::config::init_config;
use crate::drw::Drw;
use crate::globals::XlibDisplay;
use crate::types::*;
use crate::util::die;
use crate::wm::Wm;

use super::autostart::run_autostart;

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

pub fn run() {
    let (conn, screen_num) = match RustConnection::connect(None) {
        Ok((c, s)) => (c, s),
        Err(_) => {
            eprintln!(
                "instantwm: Failed to open the display from the DISPLAY environment variable.",
            );
            exit(1);
        }
    };

    let mut wm = Wm::new(WmBackend::X11(X11Backend::new(conn, screen_num)));
    wm_init(&mut wm);
    crate::events::setup(&mut wm);
    {
        let mut ctx = wm.ctx();
        if let crate::contexts::WmCtx::X11(mut x11_ctx) = ctx {
            crate::backend::x11::events::scan(&mut x11_ctx);
        }
    }
    run_autostart();
    let mut ipc_server = crate::ipc::IpcServer::bind().ok();

    if let Some(ref cmd) = wm.g.cfg.status_command {
        crate::bar::status::spawn_status_command(cmd);
    }

    crate::events::run(&mut wm, &mut ipc_server);
    crate::events::cleanup(&mut wm);
}

fn wm_init(wm: &mut Wm) {
    setup_signal_handlers();

    let (screen_num, screen, root) = {
        let Some(x11) = wm.backend.x11() else {
            return;
        };
        let screen_num = x11.screen_num;
        let screen = x11.conn.setup().roots[screen_num].clone();
        let root = screen.root;
        let conn = &x11.conn;
        crate::events::check_other_wm(conn, root);
        (screen_num, screen, root)
    };

    init_globals(wm, screen_num, root, &screen);

    {
        let Some(x11) = wm.backend.x11() else {
            return;
        };
        let conn = &x11.conn;
        init_atoms(&mut wm.x11_runtime, conn);
    }
    init_drw_and_schemes(wm);

    // Select events and initialise EWMH bits that depend on atoms + config.
    crate::events::setup_root(wm);

    // After atoms + drw exist, we can verify tag naming and create bars.
    {
        let ctx = wm.ctx();
        let crate::contexts::WmCtx::X11(mut ctx) = ctx else {
            return;
        };
        crate::bar::x11::update_bars(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref(),
        );
        crate::bar::x11::update_status(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
        crate::keyboard::grab_keys_x11(&ctx.core, &ctx.x11, ctx.x11_runtime);
        crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, None);
    }

    // Apply the initial keyboard layout if configured.
    {
        let mut ctx = wm.ctx();
        crate::keyboard_layout::init_keyboard_layout(&mut ctx);
    }
}

fn init_globals(
    wm: &mut Wm,
    screen_num: usize,
    root: Window,
    screen: &x11rb::protocol::xproto::Screen,
) {
    let cfg = init_config();

    wm.x11_runtime.screen = screen_num as i32;
    wm.x11_runtime.root = root;
    wm.g.cfg.screen_width = screen.width_in_pixels as i32;
    wm.g.cfg.screen_height = screen.height_in_pixels as i32;

    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
}

fn setup_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        libc::sigemptyset(&mut sa.sa_mask);
        sa.sa_flags = libc::SA_NOCLDSTOP | libc::SA_NOCLDWAIT | libc::SA_RESTART;
        libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
    }
}

fn init_atoms(x11_runtime: &mut crate::globals::X11RuntimeConfig, conn: &RustConnection) {
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

    x11_runtime.wmatom = crate::types::WmAtoms {
        protocols: wm_protocols,
        delete: wm_delete,
        state: wm_state,
        take_focus: wm_take_focus,
    };
    x11_runtime.netatom = crate::types::NetAtoms {
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
    x11_runtime.motifatom = motifatom;
    x11_runtime.xatom = crate::types::XAtoms {
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
    if wm.backend.x11().is_none() {
        return;
    }
    let mut drw = match Drw::new(None) {
        Ok(d) => d,
        Err(_) => die("instantwm: cannot create drawing context"),
    };

    let fonts: Vec<&str> = wm.g.cfg.fonts.iter().map(|f| f.as_str()).collect();
    if drw.fontset_create(&fonts).is_err() {
        die("no fonts could be loaded.");
    }

    let font_height = drw.fonts.as_ref().map(|f| f.h).unwrap_or(12);
    let bar_height = wm.g.cfg.bar_height;
    let bar_height = if bar_height > 0 {
        font_height + bar_height as u32
    } else {
        font_height + 12
    };

    init_cursors(wm, &mut drw);
    init_schemes(wm, &mut drw);

    wm.x11_runtime.xlibdisplay = XlibDisplay(drw.display());
    wm.x11_runtime.drw = Some(drw);
    wm.g.cfg.bar_height = bar_height as i32;
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
    use crate::bar::color::rgba_to_hex;

    let bordercolors = wm.g.cfg.bordercolors;
    let statusbarcolors = wm.g.cfg.statusbarcolors;

    let normal = drw
        .scm_create(&[&rgba_to_hex(bordercolors.normal)])
        .expect("Failed to create normal border color");
    let tile = drw
        .scm_create(&[&rgba_to_hex(bordercolors.tile_focus)])
        .expect("Failed to create tile focus border color");
    let float = drw
        .scm_create(&[&rgba_to_hex(bordercolors.float_focus)])
        .expect("Failed to create float focus border color");
    let snap = drw
        .scm_create(&[&rgba_to_hex(bordercolors.snap)])
        .expect("Failed to create snap border color");

    wm.x11_runtime.borderscheme = BorderScheme {
        normal: ColorScheme::from_vec(normal).expect("Failed to build normal border scheme"),
        tile_focus: ColorScheme::from_vec(tile).expect("Failed to build tile focus border scheme"),
        float_focus: ColorScheme::from_vec(float)
            .expect("Failed to build float focus border scheme"),
        snap: ColorScheme::from_vec(snap).expect("Failed to build snap border scheme"),
    };

    let status_clr = drw
        .scm_create(&[
            &rgba_to_hex(statusbarcolors.fg),
            &rgba_to_hex(statusbarcolors.bg),
            &rgba_to_hex(statusbarcolors.detail),
        ])
        .expect("Failed to create status bar colors");
    let status_cs =
        ColorScheme::from_vec(status_clr).expect("Failed to build status bar color scheme");
    wm.x11_runtime.statusscheme = StatusScheme::new(status_cs.fg, status_cs.bg, status_cs.detail);
}
