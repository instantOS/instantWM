use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::systray::get_systray_width;
use crate::types::{Arg, Monitor};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn update_status() {
    let (root, selmon_idx) = {
        let g = get_globals();
        (g.root, g.selmon)
    };

    let text = get_text_prop(root, x11rb::protocol::xproto::AtomEnum::WM_NAME.into());
    {
        let g = get_globals_mut();
        match text {
            Some(t) => {
                if t.starts_with("ipc:") {
                    return;
                }
                g.status_text = t;
            }
            None => {
                g.status_text = format!("instantwm-{}", VERSION);
            }
        }
    }

    if let Some(m) = get_globals_mut().monitors.get_mut(selmon_idx) {
        super::draw_bar(m);
    }

    crate::systray::update_systray();
}

pub(crate) fn update_bar_pos(m: &mut Monitor) {
    let bh = get_globals().bh;
    update_bar_pos_with_bh(m, bh);
}

pub(crate) fn update_bar_pos_with_bh(m: &mut Monitor, bh: i32) {
    m.work_rect.y = m.monitor_rect.y;
    m.work_rect.h = m.monitor_rect.h;

    if m.showbar {
        m.work_rect.h -= bh;
        if m.topbar {
            m.by = m.work_rect.y;
            m.work_rect.y += bh;
        } else {
            m.by = m.work_rect.y + m.work_rect.h;
        }
    } else {
        m.by = -bh;
    }
}

pub(crate) fn resize_bar_win(m: &Monitor) {
    let g = get_globals();
    let bh = g.bh;
    let showsystray = g.showsystray;
    let is_selmon = g
        .monitors
        .get(g.selmon)
        .map_or(false, |selmon| selmon.num == m.num);

    let mut w = m.work_rect.w as u32;
    if showsystray && is_selmon {
        w = w.saturating_sub(get_systray_width());
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(
            m.barwin,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(m.work_rect.x as i32)
                .y(m.by as i32)
                .width(w)
                .height(bh as u32),
        );
    }
}

pub(crate) fn update_bars() {
    let (bar_configs, xlibdisplay, root, status_bg) = {
        let g = get_globals();
        let bh = g.bh;
        let showsystray = g.showsystray;
        let status_bg = parse_color_to_u32(g.statusbarcolors.get(1).copied().unwrap_or("#121212"));
        let xlibdisplay = g.xlibdisplay.0;
        let root = g.root;

        let mut bar_configs = Vec::new();
        for (i, m) in g.monitors.iter().enumerate() {
            if m.barwin != 0 {
                continue;
            }

            let mut w = m.work_rect.w as u32;
            if showsystray {
                if g.selmon == i {
                    w = w.saturating_sub(crate::systray::get_systray_width() as u32);
                }
            }
            bar_configs.push((i, m.work_rect.x, m.by, w, bh));
        }
        (bar_configs, xlibdisplay, root, status_bg)
    };

    if xlibdisplay.is_null() {
        return;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        for (i, wx, by, w, bh) in bar_configs {
            let win_id = conn.generate_id().unwrap();

            let aux = x11rb::protocol::xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(status_bg)
                .event_mask(
                    x11rb::protocol::xproto::EventMask::BUTTON_PRESS
                        | x11rb::protocol::xproto::EventMask::EXPOSURE
                        | x11rb::protocol::xproto::EventMask::LEAVE_WINDOW,
                );

            let _ = conn.create_window(
                x11rb::COPY_FROM_PARENT as u8,
                win_id,
                root,
                wx as i16,
                by as i16,
                w as u16,
                bh as u16,
                0,
                x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT as u32,
                &aux,
            );

            let _ = conn.map_window(win_id);
            let _ = conn.flush();

            let globals_mut = get_globals_mut();
            globals_mut.monitors[i].barwin = win_id;
        }
    }
}

pub(crate) fn toggle_bar(_arg: &Arg) {
    let g = get_globals_mut();

    let animated = g.animated;
    let client_count = g.clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        g.animated = false;
        tmp_no_anim = true;
    }

    let selmon_idx = g.selmon;
    if let Some(selmon) = g.monitors.get_mut(selmon_idx) {
        selmon.showbar = !selmon.showbar;

        let current_tag = selmon.current_tag;
        if current_tag > 0 && current_tag <= g.tags.tags.len() {
            g.tags.tags[current_tag - 1].showbar = selmon.showbar;
        }

        update_bar_pos(selmon);
        let m = selmon.clone();
        resize_bar_win(&m);
    }

    if tmp_no_anim {
        g.animated = true;
    }
}

fn get_text_prop(win: Window, atom: u32) -> Option<String> {
    let x11 = get_x11();
    let conn = x11.conn.as_ref()?;
    let reply = conn
        .get_property(
            false,
            win,
            atom,
            x11rb::protocol::xproto::AtomEnum::ANY,
            0,
            4096,
        )
        .ok()?
        .reply()
        .ok()?;
    if reply.format != 8 || reply.value.is_empty() {
        return None;
    }
    let nul_pos = reply
        .value
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(reply.value.len());
    String::from_utf8(reply.value[..nul_pos].to_vec()).ok()
}

fn parse_color_to_u32(color: &str) -> u32 {
    let color = color.trim_start_matches('#');
    if color.len() == 6 {
        let r = u32::from_str_radix(&color[0..2], 16).unwrap_or(0);
        let g = u32::from_str_radix(&color[2..4], 16).unwrap_or(0);
        let b = u32::from_str_radix(&color[4..6], 16).unwrap_or(0);
        (r << 16) | (g << 8) | b
    } else {
        0x121212
    }
}
