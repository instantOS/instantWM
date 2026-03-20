#![allow(clippy::too_many_arguments)]
use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::client::set_client_state;
use crate::contexts::CoreCtx;
use crate::types::Systray;
use crate::types::*;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

const XEMBED_MAPPED: u32 = 1 << 0;
const XEMBED_WINDOW_ACTIVATE: u32 = 1;
const XEMBED_WINDOW_DEACTIVATE: u32 = 2;
const XEMBED_EMBEDDED_VERSION: u32 = 0;

pub fn get_systray_width(core: &CoreCtx, systray: Option<&Systray>) -> u32 {
    if !core.globals().cfg.show_systray {
        return 1;
    }

    let mut w: u32 = 0;
    if let Some(systray) = systray {
        for &icon_win in &systray.icons {
            if let Some(c) = core.globals().clients.get(&icon_win) {
                w += c.geo.w as u32 + core.globals().cfg.systray_spacing as u32;
            }
        }
    }

    if w > 0 {
        w + core.globals().cfg.systray_spacing as u32
    } else {
        1
    }
}

/// Remove systray icon using dependency injection.
pub fn remove_systray_icon(core: &mut CoreCtx, systray: Option<&mut Systray>, icon_win: WindowId) {
    if !core.globals().cfg.show_systray {
        return;
    }

    if let Some(systray) = systray {
        systray.icons.retain(|&w| w != icon_win);
    }

    core.globals_mut().clients.remove(&icon_win);
}

/// Update systray icon geometry using dependency injection.
pub fn update_systray_icon_geom(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    icon_win: WindowId,
    w: i32,
    h: i32,
) {
    let bar_height = core.globals().cfg.bar_height;

    let (geo_x, geo_y) = core
        .globals()
        .clients
        .get(&icon_win)
        .map(|client| (client.geo.x, client.geo.y))
        .unwrap_or((0, 0));

    let new_geo_h = bar_height;
    let new_geo_w = if w == h {
        bar_height
    } else if h == bar_height {
        w
    } else {
        (bar_height as f32 * (w as f32 / h as f32)) as i32
    };

    let mut rect = Rect::new(geo_x, geo_y, new_geo_w, new_geo_h);

    let _ = crate::client::geometry::apply_size_hints(core, Some(x11), icon_win, &mut rect, false);

    // Now update the client with the computed values
    if let Some(client) = core.globals_mut().clients.get_mut(&icon_win) {
        client.geo.x = rect.x;
        client.geo.y = rect.y;
        client.geo.w = rect.w;
        client.geo.h = rect.h;

        if client.geo.h > bar_height {
            if client.geo.w == client.geo.h {
                client.geo.w = bar_height;
            } else {
                client.geo.w =
                    (bar_height as f32 * (client.geo.w as f32 / client.geo.h as f32)) as i32;
            }
            client.geo.h = bar_height;
        }
    }
}

/// Update systray icon state using dependency injection.
pub fn update_systray_icon_state(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    systray: Option<&Systray>,
    icon_win: WindowId,
    ev: &PropertyNotifyEvent,
) {
    if !core.globals().cfg.show_systray {
        return;
    }

    let xembed_info_atom = x11_runtime.xatom.xembed_info;
    if ev.atom != xembed_info_atom {
        return;
    }

    let x11_icon_win: Window = icon_win.into();

    let flags = get_atom_prop(x11, icon_win, xembed_info_atom);

    if flags == 0 {
        return;
    }

    let (current_tags, _has_systray) = {
        if let Some(client) = core.globals_mut().clients.get_mut(&icon_win) {
            (client.tags, systray.is_some())
        } else {
            return;
        }
    };

    if (flags & XEMBED_MAPPED) != 0 && current_tags == 0 {
        if let Some(client) = core.globals_mut().clients.get_mut(&icon_win) {
            client.tags = 1;
        }

        let systray_win = systray.as_ref().map(|s| s.win).unwrap_or_default();
        let conn = x11.conn;
        let _ = conn.map_window(x11_icon_win);
        let _ = conn.configure_window(
            x11_icon_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        send_event(
            conn,
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_ACTIVATE as i64,
            0,
            u32::from(systray_win) as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
        set_client_state(core, x11, x11_runtime, icon_win, 1);
    } else if (flags & XEMBED_MAPPED) == 0 && current_tags != 0 {
        if let Some(client) = core.globals_mut().clients.get_mut(&icon_win) {
            client.tags = 0;
        }

        let systray_win = systray.as_ref().map(|s| s.win).unwrap_or_default();
        let conn = x11.conn;
        let _ = conn.unmap_window(x11_icon_win);
        send_event(
            conn,
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_DEACTIVATE as i64,
            0,
            u32::from(systray_win) as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
        set_client_state(core, x11, x11_runtime, icon_win, 0);
    }
}

/// Update systray using dependency injection.
pub fn update_systray(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    mut systray: Option<&mut Systray>,
) {
    if !core.globals().cfg.show_systray {
        return;
    }

    if x11_runtime.xlibdisplay.0.is_null() {
        return;
    }

    // Flush Xlib display to ensure all Xlib requests are sent before using x11rb
    unsafe {
        crate::backend::x11::draw::XFlush(x11_runtime.xlibdisplay.0);
    }

    let (x, bar_y, _showbar, bar_win) = {
        let m = systray_to_mon(core, None);
        let mon = match core.globals().monitor(m) {
            Some(mon) => mon,
            None => return,
        };
        (
            mon.monitor_rect.x + mon.monitor_rect.w,
            mon.bar_y,
            mon.showbar,
            mon.bar_win,
        )
    };

    let mut w: u32 = 1;

    let systray_exists = systray.is_some();

    if !systray_exists {
        let root = x11_runtime.root;
        let bar_height = core.globals().cfg.bar_height;
        let net_system_tray = x11_runtime.netatom.system_tray;
        let net_system_tray_horz = x11_runtime.netatom.system_tray_orientation_horz;
        let manager_atom = x11_runtime.xatom.manager;
        let bg_pixel = x11_runtime.statusscheme.bg.color.pixel as u32;

        let systray_win = Some(x11.conn).and_then(|conn| {
            let systray_win = conn.generate_id().ok()?;

            let result = conn.create_window(
                x11rb::COPY_FROM_PARENT as u8,
                systray_win,
                root,
                x as i16,
                bar_y as i16,
                w as u16,
                bar_height as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE)
                    .override_redirect(1)
                    .background_pixel(bg_pixel),
            );

            if result.is_err() {
                return None;
            }

            let _ = result.and_then(|cookie| {
                cookie
                    .check()
                    .map_err(|_| x11rb::errors::ConnectionError::UnknownError)
            });

            let _ = conn.change_property32(
                PropMode::REPLACE,
                systray_win,
                net_system_tray,
                AtomEnum::CARDINAL,
                &[net_system_tray_horz],
            );

            let _ = conn.change_window_attributes(
                systray_win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
            );

            let _ = conn.map_window(systray_win);

            let _ = conn.change_window_attributes(
                systray_win,
                &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
            );

            let _ = conn.set_selection_owner(systray_win, net_system_tray, CURRENT_TIME);

            // Send MANAGER event to root window to announce systray
            // Use non-blocking approach
            let event = ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: root,
                type_: manager_atom,
                data: ClientMessageData::from([CURRENT_TIME, net_system_tray, systray_win, 0, 0]),
            };
            let _ = conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, event);

            Some(systray_win)
        });

        let Some(systray_win) = systray_win else {
            return;
        };

        let new_systray = Systray {
            win: WindowId::from(systray_win),
            icons: Vec::new(),
        };

        if let Some(ref mut s) = systray {
            **s = new_systray;
        }
    }

    let (systray_win, icons) = match systray {
        Some(ref s) => (s.win, s.icons.clone()),
        None => return,
    };

    let bar_height = core.globals().cfg.bar_height;
    let systrayspacing = core.globals().cfg.systray_spacing;
    let bg_pixel = x11_runtime.statusscheme.bg.color.pixel as u32;

    let icon_layout: Vec<(WindowId, i32, i32)> = icons
        .iter()
        .filter_map(|icon_win| {
            core.globals()
                .clients
                .get(icon_win)
                .map(|client| (*icon_win, client.geo.w, client.geo.h))
        })
        .collect();

    let mut systray_width = 0u32;
    for _ in 0..icon_layout.len() {
        systray_width += systrayspacing as u32;
    }
    for (_, icon_w, _) in &icon_layout {
        systray_width += *icon_w as u32;
    }

    {
        let conn = x11.conn;
        w = 0;
        for (icon_win, icon_w, icon_h) in icon_layout {
            let x11_icon_win: Window = icon_win.into();
            let _ = conn.change_window_attributes(
                x11_icon_win,
                &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
            );
            let _ = conn.map_window(x11_icon_win);

            w += systrayspacing as u32;

            let _ = conn.configure_window(
                x11_icon_win,
                &ConfigureWindowAux::new()
                    .x(w as i32)
                    .y(0)
                    .width(icon_w as u32)
                    .height(icon_h as u32),
            );

            w += icon_w as u32;
        }
    }

    let x11_systray_win: Window = systray_win.into();
    let x11_bar_win: Window = bar_win.into();

    w = if systray_width > 0 {
        systray_width + systrayspacing as u32
    } else {
        1
    };
    let x = x - w as i32;

    let conn = x11.conn;

    let _ = conn.configure_window(
        x11_systray_win,
        &ConfigureWindowAux::new()
            .x(x)
            .y(bar_y)
            .width(w)
            .height(bar_height as u32),
    );

    let _ = conn.configure_window(
        x11_systray_win,
        &ConfigureWindowAux::new()
            .stack_mode(StackMode::ABOVE)
            .sibling(x11_bar_win),
    );

    let _ = conn.map_window(x11_systray_win);

    let _ = conn.flush();
}

/// Convert window to systray icon using dependency injection.
pub fn win_to_systray_icon(
    core: &CoreCtx,
    systray: Option<&Systray>,
    win: WindowId,
) -> Option<WindowId> {
    if !core.globals().cfg.show_systray {
        return None;
    }

    if let Some(systray) = systray {
        for &icon_win in &systray.icons {
            if icon_win == win {
                return Some(win);
            }
        }
    }
    None
}

/// Get monitor for systray using dependency injection.
pub fn systray_to_mon(core: &mut CoreCtx, m: Option<MonitorId>) -> MonitorId {
    if core.globals().cfg.systray_pinning == 0 {
        return match m {
            Some(id) => {
                if id == core.globals().selected_monitor_id() {
                    id
                } else {
                    core.globals().selected_monitor_id()
                }
            }
            None => core.globals().selected_monitor_id(),
        };
    }

    let n = core.globals().monitors.count();
    let target = core.globals().cfg.systray_pinning.min(n);

    if core.globals().cfg.systray_pinning > n {
        0
    } else {
        target.saturating_sub(1)
    }
}

/// Get atom property using dependency injection.
fn get_atom_prop(x11: &X11BackendRef, win: WindowId, atom: u32) -> u32 {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    if let Ok(cookie) = conn.get_property(false, x11_win, atom, AtomEnum::CARDINAL, 0, 2)
        && let Ok(reply) = cookie.reply()
        && let Some(val) = reply.value32().and_then(|mut v| v.next())
    {
        return val;
    }
    0
}

/// Send X event using dependency injection.
fn send_event(
    conn: &impl Connection,
    win: WindowId,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) {
    let x11_win: Window = win.into();
    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: x11_win,
        type_: proto,
        data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
    };
    let _ = conn.send_event(false, x11_win, EventMask::from(mask), event);
}
