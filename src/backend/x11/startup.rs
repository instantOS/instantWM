use std::process::exit;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::backend::Backend as WmBackend;
use crate::backend::BackendKind;
use crate::backend::x11::X11RuntimeConfig;
use crate::backend::x11::XlibDisplay;
use crate::backend::x11::draw::{BorderScheme, ColorScheme, DrawContext};
use crate::config::load_startup_config;
use crate::wm::Wm;

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

    let mut wm = Wm::new(WmBackend::new_x11(conn, screen_num));
    wm_init(&mut wm);
    crate::backend::x11::events::setup(&mut wm);
    {
        let ctx = wm.ctx();
        if let crate::contexts::WmCtx::X11(mut x11_ctx) = ctx {
            crate::backend::x11::events::scan(&mut x11_ctx);
        }
    }
    let mut ipc_server = crate::runtime::late_init_x11(&mut wm);

    crate::backend::x11::events::run(&mut wm, &mut ipc_server);
    crate::backend::x11::lifecycle::cleanup(&mut wm);
}

fn wm_init(wm: &mut Wm) {
    setup_signal_handlers();

    let (screen, root) = {
        let Some((conn, screen_num)) = wm.backend.x11_conn() else {
            return;
        };
        let screen = conn.setup().roots[screen_num].clone();
        let root = screen.root;
        crate::backend::x11::events::check_other_wm(conn, root);
        (screen, root)
    };

    init_globals(wm, root, &screen);

    init_atoms(&mut wm.backend);
    init_drw_and_schemes(wm);

    // Select events and initialise EWMH bits that depend on atoms + config.
    crate::backend::x11::events::setup_root(wm);

    // After atoms + drw exist, we can verify tag naming and create bars.
    crate::runtime::init_keyboard_layout(wm);
    {
        let crate::contexts::WmCtx::X11(mut ctx) = wm.ctx() else {
            return;
        };
        crate::backend::x11::bar::update_bars(
            ctx.core.state_mut(),
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray.as_ref(),
        );
        crate::backend::x11::bar::update_status(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        crate::backend::x11::keyboard::refresh_keyboard_mapping(&ctx.x11, ctx.x11_runtime);
        crate::backend::x11::keyboard::grab_keys(ctx.core.state(), &ctx.x11, ctx.x11_runtime);
        crate::focus::focus(&mut crate::contexts::WmCtx::X11(ctx.reborrow()), None);
    }
}

fn init_globals(wm: &mut Wm, root: Window, screen: &x11rb::protocol::xproto::Screen) {
    let cfg = load_startup_config(BackendKind::X11);

    // X11-specific runtime initialization
    if let Some(data) = wm.backend.x11_data_mut() {
        data.x11_runtime.root = root;
    }
    wm.core.derived.display.width = screen.width_in_pixels as i32;
    wm.core.derived.display.height = screen.height_in_pixels as i32;

    crate::core_state::apply_config(&mut wm.core, cfg);

    if !wm.core.config.monitors.is_empty() {
        let mut ctx = wm.ctx();
        crate::monitor::apply_monitor_config(&mut ctx);
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

fn init_atoms(backend: &mut crate::backend::Backend) {
    let (conn, x11_runtime) = match backend {
        crate::backend::Backend::X11(data) => (&mut data.conn, &mut data.x11_runtime),
        crate::backend::Backend::Wayland(_) => return,
    };
    const ATOM_NAMES: &[&str] = &[
        "WM_PROTOCOLS",
        "WM_DELETE_WINDOW",
        "WM_STATE",
        "WM_TAKE_FOCUS",
        "_NET_ACTIVE_WINDOW",
        "_NET_SUPPORTED",
        "_NET_SYSTEM_TRAY_S0",
        "_NET_SYSTEM_TRAY_OPCODE",
        "_NET_SYSTEM_TRAY_ORIENTATION",
        "_NET_SYSTEM_TRAY_ORIENTATION_HORZ",
        "_NET_WM_NAME",
        "_NET_WM_STATE",
        "_NET_SUPPORTING_WM_CHECK",
        "_NET_WM_STATE_FULLSCREEN",
        "_NET_WM_STATE_MAXIMIZED_VERT",
        "_NET_WM_STATE_MAXIMIZED_HORZ",
        "_NET_WM_WINDOW_TYPE",
        "_NET_WM_WINDOW_TYPE_DIALOG",
        "_NET_CLIENT_LIST",
        "_NET_CLIENT_INFO",
        "_NET_NUMBER_OF_DESKTOPS",
        "_NET_CURRENT_DESKTOP",
        "_NET_DESKTOP_NAMES",
        "_NET_DESKTOP_VIEWPORT",
        "_NET_DESKTOP_GEOMETRY",
        "_NET_WORKAREA",
        "_NET_WM_DESKTOP",
        "_MOTIF_WM_HINTS",
        "MANAGER",
        "_XEMBED",
        "_XEMBED_INFO",
        "UTF8_STRING",
        "_NET_STARTUP_ID",
        "_NET_WM_PID",
    ];
    // Queue every request first: atom setup now takes one round trip instead
    // of one round trip per atom.
    let requests: Vec<_> = ATOM_NAMES
        .iter()
        .map(|name| conn.intern_atom(false, name.as_bytes()))
        .collect();
    let mut atoms = requests.into_iter().map(|request| {
        request
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.atom)
            .unwrap_or(0)
    });
    let mut next_atom = || atoms.next().unwrap_or(0);
    let wm_protocols = next_atom();
    let wm_delete = next_atom();
    let wm_state = next_atom();
    let wm_take_focus = next_atom();
    let net_active_window = next_atom();
    let net_supported = next_atom();
    let net_system_tray = next_atom();
    let net_system_tray_op = next_atom();
    let net_system_tray_orientation = next_atom();
    let net_system_tray_orientation_horz = next_atom();
    let net_wm_name = next_atom();
    let net_wm_state = next_atom();
    let net_wm_check = next_atom();
    let net_wm_fullscreen = next_atom();
    let net_wm_maximized_vert = next_atom();
    let net_wm_maximized_horz = next_atom();
    let net_wm_window_type = next_atom();
    let net_wm_window_type_dialog = next_atom();
    let net_client_list = next_atom();
    let net_client_info = next_atom();
    let net_number_of_desktops = next_atom();
    let net_current_desktop = next_atom();
    let net_desktop_names = next_atom();
    let net_desktop_viewport = next_atom();
    let net_desktop_geometry = next_atom();
    let net_workarea = next_atom();
    let net_wm_desktop = next_atom();
    let motifatom = next_atom();
    let xembed_manager = next_atom();
    let xembed = next_atom();
    let xembed_info = next_atom();
    let utf8_string = next_atom();
    let net_startup_id = next_atom();
    let net_wm_pid = next_atom();

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
        wm_maximized_vert: net_wm_maximized_vert,
        wm_maximized_horz: net_wm_maximized_horz,
        wm_window_type: net_wm_window_type,
        wm_window_type_dialog: net_wm_window_type_dialog,
        client_list: net_client_list,
        client_info: net_client_info,
        number_of_desktops: net_number_of_desktops,
        current_desktop: net_current_desktop,
        desktop_names: net_desktop_names,
        desktop_viewport: net_desktop_viewport,
        desktop_geometry: net_desktop_geometry,
        workarea: net_workarea,
        wm_desktop: net_wm_desktop,
    };
    x11_runtime.motifatom = motifatom;
    x11_runtime.xatom = crate::types::XAtoms {
        manager: xembed_manager,
        xembed,
        xembed_info,
        utf8_string,
        net_startup_id,
        net_wm_pid,
    };
}

pub fn init_drw_and_schemes(wm: &mut Wm) {
    let Some(data) = wm.backend.x11_data_mut() else {
        return;
    };
    let mut drw = match DrawContext::new(None) {
        Ok(d) => d,
        Err(_) => panic!("instantwm: cannot create drawing context"),
    };

    let font_patterns = wm.core.config.fonts.xft_pixel_patterns();
    let fonts: Vec<&str> = font_patterns.iter().map(String::as_str).collect();
    drw.fontset_create(&fonts)
        .unwrap_or_else(|error| panic!("instantwm: {error}"));

    let metrics = wm.core.config.fonts.bar_metrics(wm.core.config.bar.height);
    let bordercolors = wm.core.config.colors.border;
    let statusbarcolors = wm.core.config.colors.status_bar;
    let close_color = wm.core.config.colors.close_button.gesture_color();

    init_cursors(&mut data.x11_runtime, &mut drw);
    init_schemes(
        &mut data.x11_runtime,
        &mut drw,
        &bordercolors,
        &statusbarcolors,
        close_color,
    );

    data.x11_runtime.xlibdisplay = XlibDisplay(drw.display());
    data.x11_runtime.draw = Some(drw);
    wm.core.derived.bar_height = metrics.height;
    wm.core.derived.bar_horizontal_padding = metrics.horizontal_padding;
}

fn init_cursors(x11_runtime: &mut X11RuntimeConfig, drw: &mut DrawContext) {
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
        if i < x11_runtime.cursors.len() {
            x11_runtime.cursors[i] = Some(cursor);
        }
    }
}

fn init_schemes(
    x11_runtime: &mut X11RuntimeConfig,
    drw: &mut DrawContext,
    bordercolors: &crate::types::BorderColorConfig,
    statusbarcolors: &crate::types::StatusColorConfig,
    close_color: crate::types::color::Rgba,
) {
    let normal = drw
        .clr_create(&bordercolors.normal.to_string())
        .expect("Failed to create normal border color");
    let tile = drw
        .clr_create(&bordercolors.tile_focus.to_string())
        .expect("Failed to create tile focus border color");
    let float = drw
        .clr_create(&bordercolors.float_focus.to_string())
        .expect("Failed to create float focus border color");
    let snap = drw
        .clr_create(&bordercolors.snap.to_string())
        .expect("Failed to create snap border color");
    let close = drw
        .clr_create(&close_color.to_string())
        .expect("Failed to create close gesture border color");

    let borderscheme = BorderScheme {
        normal: ColorScheme::from_single(normal),
        tile_focus: ColorScheme::from_single(tile),
        float_focus: ColorScheme::from_single(float),
        snap: ColorScheme::from_single(snap),
        close: ColorScheme::from_single(close),
    };

    let status = drw
        .scm_create(&[
            &statusbarcolors.fg.to_string(),
            &statusbarcolors.bg.to_string(),
            &statusbarcolors.detail.to_string(),
        ])
        .expect("Failed to create status bar colors");

    x11_runtime.border_scheme = borderscheme;
    x11_runtime.status_scheme = ColorScheme::new(status.fg, status.bg, status.detail);
}
