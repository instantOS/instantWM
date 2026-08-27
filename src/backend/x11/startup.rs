use std::collections::HashMap;
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
        if !crate::backend::x11::keyboard::refresh_keyboard_mapping(&ctx.x11, ctx.x11_runtime) {
            log::warn!("initial X11 keyboard mapping read failed; retrying once");
            assert!(
                crate::backend::x11::keyboard::refresh_keyboard_mapping(&ctx.x11, ctx.x11_runtime,),
                "instantwm: failed to read the X11 keyboard mapping"
            );
        }
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
    let atoms = intern_atoms(conn, ATOM_NAMES);
    let atom = |name| atoms.get(name).copied().unwrap_or(0);

    x11_runtime.wmatom = crate::types::WmAtoms {
        protocols: atom("WM_PROTOCOLS"),
        delete: atom("WM_DELETE_WINDOW"),
        state: atom("WM_STATE"),
        take_focus: atom("WM_TAKE_FOCUS"),
    };
    x11_runtime.netatom = crate::types::NetAtoms {
        active_window: atom("_NET_ACTIVE_WINDOW"),
        supported: atom("_NET_SUPPORTED"),
        system_tray: atom("_NET_SYSTEM_TRAY_S0"),
        system_tray_op: atom("_NET_SYSTEM_TRAY_OPCODE"),
        system_tray_orientation: atom("_NET_SYSTEM_TRAY_ORIENTATION"),
        system_tray_orientation_horz: atom("_NET_SYSTEM_TRAY_ORIENTATION_HORZ"),
        wm_name: atom("_NET_WM_NAME"),
        wm_state: atom("_NET_WM_STATE"),
        wm_check: atom("_NET_SUPPORTING_WM_CHECK"),
        wm_fullscreen: atom("_NET_WM_STATE_FULLSCREEN"),
        wm_maximized_vert: atom("_NET_WM_STATE_MAXIMIZED_VERT"),
        wm_maximized_horz: atom("_NET_WM_STATE_MAXIMIZED_HORZ"),
        wm_window_type: atom("_NET_WM_WINDOW_TYPE"),
        wm_window_type_dialog: atom("_NET_WM_WINDOW_TYPE_DIALOG"),
        client_list: atom("_NET_CLIENT_LIST"),
        client_info: atom("_NET_CLIENT_INFO"),
        number_of_desktops: atom("_NET_NUMBER_OF_DESKTOPS"),
        current_desktop: atom("_NET_CURRENT_DESKTOP"),
        desktop_names: atom("_NET_DESKTOP_NAMES"),
        desktop_viewport: atom("_NET_DESKTOP_VIEWPORT"),
        desktop_geometry: atom("_NET_DESKTOP_GEOMETRY"),
        workarea: atom("_NET_WORKAREA"),
        wm_desktop: atom("_NET_WM_DESKTOP"),
    };
    x11_runtime.motifatom = atom("_MOTIF_WM_HINTS");
    x11_runtime.xatom = crate::types::XAtoms {
        manager: atom("MANAGER"),
        xembed: atom("_XEMBED"),
        xembed_info: atom("_XEMBED_INFO"),
        utf8_string: atom("UTF8_STRING"),
        net_startup_id: atom("_NET_STARTUP_ID"),
        net_wm_pid: atom("_NET_WM_PID"),
    };
}

fn intern_atoms(
    conn: &RustConnection,
    names: &'static [&'static str],
) -> HashMap<&'static str, u32> {
    // Queue every request first: atom setup takes one round trip, while the
    // name remains attached to its reply so field assignment is not positional.
    let requests: Vec<_> = names
        .iter()
        .map(|&name| (name, conn.intern_atom(false, name.as_bytes())))
        .collect();
    requests
        .into_iter()
        .map(|(name, request)| {
            let atom = request
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| reply.atom)
                .unwrap_or(0);
            (name, atom)
        })
        .collect()
}

pub fn init_drw_and_schemes(wm: &mut Wm) {
    let Some(data) = wm.backend.x11_data_mut() else {
        return;
    };
    let mut drw = match DrawContext::new(None) {
        Ok(d) => d,
        Err(_) => panic!("instantwm: cannot create drawing context"),
    };

    let font_patterns = xft_font_patterns(&wm.core.config.fonts);
    let fonts: Vec<_> = font_patterns
        .iter()
        .map(|(role, pattern)| (*role, pattern.as_str()))
        .collect();
    drw.fontset_create(&fonts)
        .unwrap_or_else(|error| panic!("instantwm: {error}"));
    drw.set_icon_gap_px(crate::bar::text::icon_boundary_pad_px(
        wm.core.config.fonts.text_size,
        wm.core.config.fonts.icon_size,
    ));

    let metrics = wm.core.config.fonts.bar_metrics(wm.core.config.bar.height);
    let bordercolors = wm.core.config.colors.border;
    let statusbarcolors = wm.core.config.colors.status;
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

fn xft_font_patterns(
    fonts: &crate::core_state::FontConfig,
) -> Vec<(crate::bar::text::FontRole, String)> {
    use crate::bar::text::FontRole;

    let pattern = |family: &str, size: f32| format!("{family}:pixelsize={size}");
    vec![
        (FontRole::Text, pattern(&fonts.text_family, fonts.text_size)),
        (FontRole::Icon, pattern(&fonts.icon_family, fonts.icon_size)),
    ]
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

#[cfg(test)]
mod tests {
    use super::xft_font_patterns;
    use crate::bar::text::FontRole;
    use crate::core_state::FontConfig;

    #[test]
    fn xft_patterns_keep_roles_and_use_logical_pixel_sizes() {
        let fonts = FontConfig {
            text_family: "Inter".into(),
            text_size: 12.0,
            icon_family: "Symbols Nerd Font".into(),
            icon_size: 16.0,
        };

        assert_eq!(
            xft_font_patterns(&fonts),
            [
                (FontRole::Text, "Inter:pixelsize=12".to_string()),
                (FontRole::Icon, "Symbols Nerd Font:pixelsize=16".to_string()),
            ]
        );
    }
}
