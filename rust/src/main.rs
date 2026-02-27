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
use std::sync::Arc;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::backend::wayland::compositor::{
    KeyboardFocusTarget, PointerFocusTarget, WaylandClientState, WaylandState,
};
use crate::backend::wayland::WaylandBackend;
use crate::backend::x11::X11Backend;
use crate::backend::Backend as WmBackend;
use crate::config::init_config;
use crate::drw::Drw;
use crate::globals::XlibDisplay;
use crate::types::*;
use crate::util::die;
use crate::wm::Wm;
use crate::xresources::list_xresources;
use smithay::backend::input::{
    AbsolutePositionEvent, Event, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
    PointerButtonEvent,
};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::desktop::space::render_output;
use smithay::desktop::utils::{send_frames_surface_tree, surface_primary_scanout_output};
use smithay::input::keyboard::FilterResult;
use smithay::output::Mode as OutputMode;
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use smithay::reexports::wayland_server::Display;
use smithay::utils::{Point, Transform, SERIAL_COUNTER};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::socket::ListeningSocketSource;

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
enum CliBackend {
    X11,
    Wayland,
}

#[derive(Debug, Parser)]
#[command(name = "instantwm", version, disable_help_subcommand = true)]
struct Cli {
    #[arg(short = 'X', long = "xresources")]
    xresources: bool,

    #[arg(long, value_enum, default_value_t = CliBackend::X11)]
    backend: CliBackend,
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
        CliBackend::X11 => run_x11(),
        CliBackend::Wayland => run_wayland(),
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

    let mut wm = Wm::new(WmBackend::X11(X11Backend::new(conn, screen_num)));
    wm_init(&mut wm);
    crate::events::setup(&mut wm);
    crate::events::scan(&mut wm);
    run_autostart();
    crate::events::run(&mut wm);
    crate::events::cleanup(&mut wm);
}

fn run_wayland() -> ! {
    let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));
    init_wayland_globals(&mut wm);
    run_autostart();

    let mut event_loop: EventLoop<WaylandState> = EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();

    let display: Display<WaylandState> = Display::new().expect("wayland display");
    let mut display_handle = display.handle();
    let mut state = WaylandState::new(display, &loop_handle);
    if let WmBackend::Wayland(ref wayland) = wm.backend {
        wayland.attach_state(&mut state);
    }

    let (mut backend, mut winit_loop) =
        winit::init::<GlesRenderer>().expect("failed to init winit backend");
    let output_size = backend.window_size();
    let (initial_w, initial_h) = sanitize_wayland_size(output_size.w, output_size.h);
    wm.g.cfg.screen_width = initial_w;
    wm.g.cfg.screen_height = initial_h;
    monitor::update_geom_ctx(&mut wm.ctx());

    let output = state.create_output("winit", initial_w, initial_h);
    let mut damage_tracker = OutputDamageTracker::from_output(&output);

    let seat = state.seat.clone();
    let keyboard_handle = seat.get_keyboard().expect("keyboard missing from seat");
    let pointer_handle = seat.get_pointer().expect("pointer missing from seat");

    let listening_socket = ListeningSocketSource::new_auto().expect("wayland socket");
    let socket_name = listening_socket
        .socket_name()
        .to_string_lossy()
        .into_owned();
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    loop_handle
        .insert_source(listening_socket, |client, _, data| {
            let _ = data
                .display_handle
                .insert_client(client, Arc::new(WaylandClientState::default()));
        })
        .expect("listening socket source");

    let start_time = std::time::Instant::now();
    let mut pointer_location = Point::from((0.0, 0.0));

    let loop_signal: LoopSignal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(16), &mut state, move |state| {
            winit_loop.dispatch_new_events(|event| match event {
                WinitEvent::Resized { size, .. } => {
                    let (safe_w, safe_h) = sanitize_wayland_size(size.w, size.h);
                    let mode = OutputMode {
                        size: (safe_w, safe_h).into(),
                        refresh: 60_000,
                    };
                    wm.g.cfg.screen_width = safe_w;
                    wm.g.cfg.screen_height = safe_h;
                    monitor::update_geom_ctx(&mut wm.ctx());
                    output.change_current_state(
                        Some(mode),
                        Some(Transform::Normal),
                        None,
                        Some((0, 0).into()),
                    );
                    output.set_preferred(mode);
                }
                WinitEvent::Input(event) => match event {
                    InputEvent::Keyboard { event } => {
                        let serial = SERIAL_COUNTER.next_serial();
                        keyboard_handle.input(
                            state,
                            event.key_code(),
                            event.state(),
                            serial,
                            event.time() as u32,
                            |_data: &mut WaylandState,
                             modifiers: &smithay::input::keyboard::ModifiersState,
                             keysym: smithay::input::keyboard::KeysymHandle<'_>| {
                                if event.state() == smithay::backend::input::KeyState::Pressed {
                                    let mod_mask = modifiers_to_x11_mask(modifiers);
                                    let mut ctx = wm.ctx();
                                    if keyboard::handle_keysym(
                                        &mut ctx,
                                        u32::from(keysym.modified_sym()),
                                        mod_mask,
                                    ) {
                                        return FilterResult::Intercept(());
                                    }
                                }
                                FilterResult::Forward
                            },
                        );
                    }
                    InputEvent::PointerMotionAbsolute { event } => {
                        let size = backend.window_size();
                        let x = event.x_transformed(size.w);
                        let y = event.y_transformed(size.h);
                        pointer_location = Point::from((x, y));

                        let (focus, keyboard_focus) =
                            match state.space.element_under(pointer_location) {
                                Some((window, location)) => {
                                    let keyboard_focus =
                                        Some(KeyboardFocusTarget::Window(window.clone()));
                                    let focus = window.wl_surface().map(|surface| {
                                        (
                                            PointerFocusTarget::WlSurface(surface.into_owned()),
                                            location.to_f64(),
                                        )
                                    });
                                    (focus, keyboard_focus)
                                }
                                None => (None, None),
                            };

                        let serial = SERIAL_COUNTER.next_serial();
                        let motion = smithay::input::pointer::MotionEvent {
                            location: pointer_location,
                            serial,
                            time: event.time() as u32,
                        };
                        pointer_handle.motion(state, focus, &motion);
                        keyboard_handle.set_focus(state, keyboard_focus, serial);
                    }
                    InputEvent::PointerButton { event } => {
                        let serial = SERIAL_COUNTER.next_serial();
                        let button = smithay::input::pointer::ButtonEvent {
                            serial,
                            time: event.time() as u32,
                            button: event.button_code(),
                            state: event.state(),
                        };
                        pointer_handle.button(state, &button);
                    }
                    InputEvent::PointerAxis { event } => {
                        let mut frame =
                            smithay::input::pointer::AxisFrame::new(event.time() as u32);
                        frame = frame.source(event.source());
                        if let Some(amount) = event.amount(smithay::backend::input::Axis::Vertical)
                        {
                            frame = frame.value(smithay::backend::input::Axis::Vertical, amount);
                        }
                        if let Some(amount) =
                            event.amount(smithay::backend::input::Axis::Horizontal)
                        {
                            frame = frame.value(smithay::backend::input::Axis::Horizontal, amount);
                        }
                        if let Some(steps) =
                            event.amount_v120(smithay::backend::input::Axis::Vertical)
                        {
                            frame =
                                frame.v120(smithay::backend::input::Axis::Vertical, steps as i32);
                        }
                        if let Some(steps) =
                            event.amount_v120(smithay::backend::input::Axis::Horizontal)
                        {
                            frame =
                                frame.v120(smithay::backend::input::Axis::Horizontal, steps as i32);
                        }
                        pointer_handle.axis(state, frame);
                    }
                    _ => {}
                },
                WinitEvent::CloseRequested => {
                    loop_signal.stop();
                }
                _ => {}
            });

            state.sync_space_from_globals();
            let age = backend.buffer_age().unwrap_or(0);
            let damage = {
                let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");
                let custom_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
                let render_result = render_output(
                    &output,
                    renderer,
                    &mut framebuffer,
                    1.0,
                    age,
                    [&state.space],
                    &custom_elements,
                    &mut damage_tracker,
                    [0.05, 0.05, 0.07, 1.0],
                )
                .expect("render output");

                render_result.damage.cloned()
            };
            let _ = backend.submit(damage.as_deref());

            let time = start_time.elapsed();
            for window in state.space.elements() {
                if let Some(surface) = window.wl_surface() {
                    send_frames_surface_tree(
                        &surface,
                        &output,
                        time,
                        Some(Duration::from_millis(16)),
                        surface_primary_scanout_output,
                    );
                }
            }

            if display_handle.flush_clients().is_err() {
                loop_signal.stop();
            }
        })
        .expect("wayland event loop run");
    exit(0);
}

fn init_wayland_globals(wm: &mut Wm) {
    let cfg = init_config();
    wm.g.cfg.screen_width = 1280;
    wm.g.cfg.screen_height = 800;
    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.cfg.showbar = false;
    wm.g.cfg.numlockmask = 0;
    monitor::update_geom_ctx(&mut wm.ctx());
}

#[inline]
fn sanitize_wayland_size(w: i32, h: i32) -> (i32, i32) {
    const WAYLAND_MIN_DIM: i32 = 64;
    (w.max(WAYLAND_MIN_DIM), h.max(WAYLAND_MIN_DIM))
}

fn modifiers_to_x11_mask(mods: &smithay::input::keyboard::ModifiersState) -> u32 {
    let mut mask = 0u32;
    if mods.shift {
        mask |= crate::config::SHIFT;
    }
    if mods.ctrl {
        mask |= crate::config::CONTROL;
    }
    if mods.alt {
        mask |= crate::config::MOD1;
    }
    if mods.logo {
        mask |= crate::config::MODKEY;
    }
    mask
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
        let mut ctx = wm.ctx();
        crate::xresources::load_xresources(&mut ctx);
    }

    {
        let Some(x11) = wm.backend.x11() else {
            return;
        };
        let conn = &x11.conn;
        init_atoms(&mut wm.g, conn);
    }
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
    if wm.backend.x11().is_none() {
        return;
    }
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
