use crate::bar::{draw_bar, get_tag_width, reset_bar};
use crate::client::{
    client_height, client_width, configure, is_hidden, is_visible, resize_client, set_client_state,
    set_fullscreen, unmanage, update_motif_hints, update_size_hints, update_title,
    update_window_type, update_wm_hints, win_to_client, WM_STATE_NORMAL, WM_STATE_WITHDRAWN,
};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::keyboard::grab_keys;
use crate::monitor::{arrange, rect_to_mon, restack, update_geom, win_to_mon};
use crate::mouse::{get_cursor_client, reset_cursor, resize_mouse};
use crate::overlay::show_overlay;
use crate::scratchpad::scratchpad_show;
use crate::systray::{
    get_systray_width, remove_systray_icon, update_systray, update_systray_icon_geom,
    update_systray_icon_state, win_to_systray_icon,
};
use crate::tags::view;
use crate::types::{self as types, *};
use crate::util::clean_mask;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;

static mut BAR_LEAVE_STATUS: i32 = 0;

fn has_tiling_layout(mon_id: MonitorId) -> bool {
    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(mon_id) {
        mon.sellt == 0
    } else {
        false
    }
}

pub fn button_press(e: &ButtonPressEvent) {
    let mut click = Click::RootWin;
    let mut arg = Arg::default();

    let globals = get_globals();
    let selmon_id = globals.selmon;
    let numlockmask = globals.numlockmask;
    let buttons = globals.buttons.clone();
    drop(globals);

    handle_focus_monitor(e.event, e.root_x as i32, e.root_y as i32);

    let globals = get_globals();
    if let Some(sel_id) = selmon_id {
        if let Some(mon) = globals.monitors.get(sel_id) {
            if e.event == mon.barwin {
                drop(globals);
                handle_bar_click(e, &mut click, &mut arg, sel_id);
            } else if let Some(win) = win_to_client(e.event) {
                drop(globals);
                handle_client_click(e, win, &mut click);
            } else if e.root_x as i32
                > globals
                    .monitors
                    .get(sel_id)
                    .map(|m| m.mx + m.mw - SIDEBAR_WIDTH)
                    .unwrap_or(0)
            {
                click = Click::SideBar;
            } else {
                drop(globals);
                handle_resize_click(e);
                return;
            }
        } else {
            drop(globals);
        }
    } else {
        drop(globals);
    }

    let globals = get_globals();
    let altcursor = globals.altcursor;
    if click == Click::RootWin && altcursor == AltCursor::Resize && e.detail == 1 {
        if let Some(sel_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_id) {
                if let Some(sel_win) = mon.sel {
                    let is_floating = globals
                        .clients
                        .get(&sel_win)
                        .map(|c| c.isfloating)
                        .unwrap_or(false);
                    let has_tiling = has_tiling_layout(sel_id);
                    if is_floating || !has_tiling {
                        drop(globals);
                        reset_cursor();
                        resize_mouse(&Arg::default());
                        return;
                    }
                }
            }
        }
    }
    drop(globals);

    for button in &buttons {
        if click == button.click && button.func.is_some() && button.button == e.detail {
            let clean_button_mask = clean_mask(button.mask, numlockmask);
            let clean_state = clean_mask(e.state.into(), numlockmask);
            if clean_button_mask == clean_state {
                let use_arg = matches!(
                    click,
                    Click::TagBar
                        | Click::WinTitle
                        | Click::CloseButton
                        | Click::ShutDown
                        | Click::SideBar
                        | Click::ResizeWidget
                ) && button.arg.i == 0;

                if let Some(func) = button.func {
                    func(if use_arg { &arg } else { &button.arg });
                }
            }
        }
    }
}

pub fn client_message(e: &ClientMessageEvent) {
    let globals = get_globals();
    let showsystray = globals.showsystray;
    let systray_win = globals.systray.as_ref().map(|s| s.win).unwrap_or(0);
    let net_system_tray_op = globals.netatom[NetAtom::SystemTrayOP as usize];
    let net_wm_state = globals.netatom[NetAtom::WMState as usize];
    let net_wm_fullscreen = globals.netatom[NetAtom::WMFullscreen as usize];
    let net_active_window = globals.netatom[NetAtom::ActiveWindow as usize];
    drop(globals);

    if showsystray && e.window == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(e);
        }
        return;
    }

    let c = win_to_client(e.window);
    if c.is_none() {
        return;
    }
    let win = c.unwrap();

    if e.type_ == net_wm_state {
        handle_net_wm_state(e, win);
    } else if e.type_ == net_active_window {
        handle_active_window(e, win);
    }
}

pub fn configure_notify(e: &ConfigureNotifyEvent) {
    let globals = get_globals();
    if e.window != globals.root {
        return;
    }

    let dirty = globals.sw != e.width as i32 || globals.sh != e.height as i32;
    drop(globals);

    {
        let mut globals = get_globals_mut();
        globals.sw = e.width as i32;
        globals.sh = e.height as i32;
    }

    let geom_changed = update_geom();

    if geom_changed || dirty {
        let globals = get_globals();
        let bh = globals.bh;
        let monitors = globals.monitors.clone();
        drop(globals);

        for (i, m) in monitors.iter().enumerate() {
            for (client_win, c) in get_globals().clients.iter() {
                if c.mon_id == Some(i) {
                    if c.isfakefullscreen {
                        let x11 = get_x11();
                        if let Some(ref conn) = x11.conn {
                            let _ = conn.configure_window(
                                m.barwin,
                                &ConfigureWindowAux::new()
                                    .x(m.wx)
                                    .y(m.by)
                                    .width(m.ww as u32)
                                    .height(bh as u32),
                            );
                        }
                    } else if c.is_fullscreen {
                        resize_client(*client_win, m.mx, m.my, m.mw, m.mh);
                    }
                }
            }

            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let _ = conn.configure_window(
                    m.barwin,
                    &ConfigureWindowAux::new()
                        .x(m.wx)
                        .y(m.by)
                        .width(m.ww as u32)
                        .height(bh as u32),
                );
            }
        }

        focus(None);
        arrange(None);
    }
}

pub fn configure_request(e: &ConfigureRequestEvent) {
    let c = win_to_client(e.window);

    if let Some(win) = c {
        let globals = get_globals();
        let client = match globals.clients.get(&win) {
            Some(c) => c.clone(),
            None => return,
        };
        drop(globals);

        if e.value_mask.contains(ConfigWindow::BORDER_WIDTH) {
            let mut globals = get_globals_mut();
            if let Some(c) = globals.clients.get_mut(&win) {
                c.border_width = e.border_width as i32;
            }
        } else if client.isfloating || !has_tiling_layout(client.mon_id.unwrap_or(0)) {
            let m_id = client.mon_id.unwrap_or(0);
            let globals = get_globals();
            let m = globals.monitors.get(m_id).cloned();
            drop(globals);

            if let Some(m) = m {
                let mut new_x = client.x;
                let mut new_y = client.y;
                let mut new_w = client.w;
                let mut new_h = client.h;

                if e.value_mask.contains(ConfigWindow::X) {
                    new_x = m.mx + e.x as i32;
                }
                if e.value_mask.contains(ConfigWindow::Y) {
                    new_y = m.my + e.y as i32;
                }
                if e.value_mask.contains(ConfigWindow::WIDTH) {
                    new_w = e.width as i32;
                }
                if e.value_mask.contains(ConfigWindow::HEIGHT) {
                    new_h = e.height as i32;
                }

                if (new_x + new_w) > m.mx + m.mw && client.isfloating {
                    new_x = m.mx + (m.mw / 2 - client_width(&client) / 2);
                }
                if (new_y + new_h) > m.my + m.mh && client.isfloating {
                    new_y = m.my + (m.mh / 2 - client_height(&client) / 2);
                }

                let has_pos = e.value_mask.contains(ConfigWindow::X)
                    || e.value_mask.contains(ConfigWindow::Y);
                let has_size = e.value_mask.contains(ConfigWindow::WIDTH)
                    || e.value_mask.contains(ConfigWindow::HEIGHT);

                if has_pos && !has_size {
                    configure(win);
                }

                if is_visible(&client) {
                    let x11 = get_x11();
                    if let Some(ref conn) = x11.conn {
                        let _ = conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .x(new_x)
                                .y(new_y)
                                .width(new_w as u32)
                                .height(new_h as u32),
                        );
                    }
                }

                let mut globals = get_globals_mut();
                if let Some(c) = globals.clients.get_mut(&win) {
                    if e.value_mask.contains(ConfigWindow::X) {
                        c.oldx = c.x;
                        c.x = new_x;
                    }
                    if e.value_mask.contains(ConfigWindow::Y) {
                        c.oldy = c.y;
                        c.y = new_y;
                    }
                    if e.value_mask.contains(ConfigWindow::WIDTH) {
                        c.oldw = c.w;
                        c.w = new_w;
                    }
                    if e.value_mask.contains(ConfigWindow::HEIGHT) {
                        c.oldh = c.h;
                        c.h = new_h;
                    }
                }
            }
        } else {
            configure(win);
        }
    } else {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.configure_window(
                e.window,
                &ConfigureWindowAux::new()
                    .x(e.x)
                    .y(e.y)
                    .width(e.width as u32)
                    .height(e.height as u32)
                    .border_width(e.border_width as u32)
                    .sibling(e.sibling)
                    .stack_mode(e.stack_mode),
            );
        }
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.flush();
    }
}

pub fn destroy_notify(e: &DestroyNotifyEvent) {
    if let Some(win) = win_to_client(e.window) {
        unmanage(win, true);
    } else if let Some(icon) = win_to_systray_icon(e.window) {
        remove_systray_icon(&icon);
        let globals = get_globals();
        if let Some(sel_id) = globals.selmon {
            drop(globals);
            resize_bar_win(sel_id);
        }
        update_systray();
    }
}

pub fn enter_notify(e: &EnterNotifyEvent) {
    handle_bar_leave_reset(e);

    if (e.mode != NotifyMode::Normal || e.detail == NotifyDetail::Inferior)
        && e.event != get_globals().root
    {
        return;
    }

    let mut c = win_to_client(e.event);
    c = handle_floating_focus(e, c);
    if c.is_none() {
        return;
    }

    let globals = get_globals();
    if !globals.focusfollowsmouse {
        return;
    }
    drop(globals);

    if enternotify_monitor_switch(e.event, c) {
        focus(None);
        return;
    }

    if should_skip_floating_focus(e.event, c) {
        return;
    }

    if let Some(win) = c {
        let globals = get_globals();
        if let Some(sel_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_id) {
                if mon.sel == Some(win) {
                    return;
                }
            }
        }
        drop(globals);
        focus(Some(win));
    }
}

pub fn expose(e: &ExposeEvent) {
    if e.count != 0 {
        return;
    }

    let m = win_to_mon(e.window);
    if let Some(mon_id) = m {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            draw_bar(mon);
        }

        let is_selmon = globals.selmon == Some(mon_id);
        drop(globals);

        if is_selmon {
            update_systray();
        }
    }
}

pub fn focus_in(e: &FocusInEvent) {
    let globals = get_globals();
    if let Some(sel_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_id) {
            if let Some(sel_win) = mon.sel {
                if e.event != sel_win {
                    drop(globals);
                    crate::client::set_focus(sel_win);
                }
            }
        }
    }
}

pub fn key_press(e: &KeyPressEvent) {
    let globals = get_globals();
    let numlockmask = globals.numlockmask;
    let keys = globals.keys.clone();
    drop(globals);

    for key in &keys {
        if key.keysym == e.detail as u32 {
            let clean_key_mask = clean_mask(key.mod_mask, numlockmask);
            let clean_state = clean_mask(e.state.into(), numlockmask);
            if clean_key_mask == clean_state {
                if let Some(func) = key.func {
                    func(&key.arg);
                }
            }
        }
    }
}

pub fn key_release(e: &KeyReleaseEvent) {
    let globals = get_globals();
    let numlockmask = globals.numlockmask;
    let keys = globals.keys.clone();
    drop(globals);

    for key in &keys {
        if key.keysym == e.detail as u32 {
            let clean_key_mask = clean_mask(key.mod_mask, numlockmask);
            let clean_state = clean_mask(e.state.into(), numlockmask);
            if clean_key_mask == clean_state {}
        }
    }
}

pub fn mapping_notify(e: &MappingNotifyEvent) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.refresh_keyboard_mapping(e);
    }

    if e.request == Mapping::Keyboard {
        grab_keys();
    }
}

pub fn map_request(e: &MapRequestEvent) {
    if let Some(icon) = win_to_systray_icon(e.window) {
        let globals = get_globals();
        let systray_win = globals.systray.as_ref().map(|s| s.win).unwrap_or(0);
        let xembed = globals.netatom[NetAtom::Xembed as usize];
        drop(globals);

        send_event_embed(icon.win, xembed, systray_win, XEMBED_WINDOW_ACTIVATE);

        if let Some(sel_id) = get_globals().selmon {
            resize_bar_win(sel_id);
        }
        update_systray();
        return;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) = conn.get_window_attributes(e.window) {
            if let Ok(wa) = cookie.reply() {
                if wa.override_redirect {
                    return;
                }

                if win_to_client(e.window).is_none() {
                    crate::client::manage(
                        e.window,
                        wa.x,
                        wa.y,
                        wa.width as u32,
                        wa.height as u32,
                        wa.border_width as u32,
                    );
                }
            }
        }
    }
}

pub fn motion_notify(e: &MotionNotifyEvent) {
    if e.event != get_globals().root {
        return;
    }

    let mut tagwidth = get_globals().tagwidth;
    if tagwidth == 0 {
        tagwidth = get_tag_width();
        get_globals_mut().tagwidth = tagwidth;
    }

    let m = rect_to_mon(e.event_x as i32, e.event_y as i32, 1, 1);
    let globals = get_globals();
    if let Some(m_id) = m {
        if Some(m_id) != globals.selmon && globals.focusfollowsmouse {
            let old_sel = globals.selmon;
            drop(globals);

            if let Some(old_id) = old_sel {
                if let Some(old_mon) = get_globals().monitors.get(old_id) {
                    if let Some(sel_win) = old_mon.sel {
                        crate::client::unfocus_win(sel_win, true);
                    }
                }
            }

            let mut globals = get_globals_mut();
            globals.selmon = Some(m_id);
            drop(globals);
            focus(None);
            return;
        }
    }
    drop(globals);

    let globals = get_globals();
    if let Some(sel_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_id) {
            let bh = globals.bh;
            if e.event_y as i32 >= mon.my + bh - 3 {
                drop(globals);
                if handle_floating_resize_hover(sel_id) {
                    return;
                }
                if handle_sidebar_hover(e, sel_id) {
                    return;
                }
                reset_bar();
                if get_globals().altcursor == AltCursor::Sidebar {
                    reset_cursor();
                }
                return;
            }

            if handle_overlay_gesture(e, sel_id) {
                return;
            }

            let layout_width = get_layout_symbol_width(sel_id);
            if (e.event_x as i32) < mon.mx + tagwidth + layout_width {
                drop(globals);
                handle_tagbar_hover(e, sel_id);
            } else if let Some(sel_win) = mon.sel {
                if (e.event_x as i32) < mon.mx + layout_width + tagwidth + mon.bar_clients_width {
                    drop(globals);
                    handle_titlebar_hover(e, sel_id);
                } else {
                    drop(globals);
                    reset_bar();
                }
            } else {
                drop(globals);
                reset_bar();
            }
        }
    }
}

pub fn property_notify(e: &PropertyNotifyEvent) {
    if let Some(icon) = win_to_systray_icon(e.window) {
        let globals = get_globals();
        let xatom_wm_normal_hints = x11rb::protocol::xproto::AtomEnum::WM_NORMAL_HINTS.into();
        drop(globals);

        if e.atom == xatom_wm_normal_hints {
            update_size_hints(&icon);
            update_systray_icon_geom(&icon, icon.w, icon.h);
        } else {
            update_systray_icon_state(&icon, e);
        }

        if let Some(sel_id) = get_globals().selmon {
            resize_bar_win(sel_id);
        }
        update_systray();
        return;
    }

    let globals = get_globals();
    if e.window == globals.root && e.atom == x11rb::protocol::xproto::AtomEnum::WM_NAME.into() {
        drop(globals);
        if !xcommand() {
            crate::bar::update_status();
        }
        return;
    }
    drop(globals);

    if e.state == Property::Delete {
        return;
    }

    if let Some(win) = win_to_client(e.window) {
        match e.atom {
            x if x == x11rb::protocol::xproto::AtomEnum::WM_TRANSIENT_FOR.into() => {
                let x11 = get_x11();
                if let Some(ref conn) = x11.conn {
                    if let Ok(cookie) = conn.get_property(
                        false,
                        win,
                        AtomEnum::WM_TRANSIENT_FOR,
                        AtomEnum::WINDOW,
                        0,
                        1,
                    ) {
                        if let Ok(reply) = cookie.reply() {
                            if let Some(trans) = reply.value32().and_then(|mut v| v.next()) {
                                if let Some(trans_client) = win_to_client(trans) {
                                    let mut globals = get_globals_mut();
                                    if let Some(c) = globals.clients.get_mut(&win) {
                                        if !c.isfloating {
                                            c.isfloating = true;
                                        }
                                    }
                                    drop(globals);
                                    if let Some(mon_id) =
                                        get_globals().clients.get(&win).and_then(|c| c.mon_id)
                                    {
                                        arrange(Some(mon_id));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            x if x == x11rb::protocol::xproto::AtomEnum::WM_NORMAL_HINTS.into() => {
                let mut globals = get_globals_mut();
                if let Some(c) = globals.clients.get_mut(&win) {
                    c.hintsvalid = 0;
                }
            }
            x if x == x11rb::protocol::xproto::AtomEnum::WM_HINTS.into() => {
                drop(get_globals());
                update_wm_hints(win);
                crate::bar::draw_bars();
            }
            _ => {}
        }

        let globals = get_globals();
        let net_wm_name = globals.netatom[NetAtom::WMName as usize];
        let xatom_wm_name = x11rb::protocol::xproto::AtomEnum::WM_NAME.into();
        drop(globals);

        if e.atom == xatom_wm_name || e.atom == net_wm_name {
            update_title(win);
            let globals = get_globals();
            if let Some(c) = globals.clients.get(&win) {
                if let Some(mon_id) = c.mon_id {
                    if let Some(mon) = globals.monitors.get(mon_id) {
                        if mon.sel == Some(win) {
                            drop(globals);
                            let mut globals = get_globals_mut();
                            if let Some(mon) = globals.monitors.get_mut(mon_id) {
                                draw_bar(mon);
                            }
                        }
                    }
                }
            }
        }

        let globals = get_globals();
        let net_wm_window_type = globals.netatom[NetAtom::WMWindowType as usize];
        drop(globals);

        if e.atom == net_wm_window_type {
            update_window_type(win);
        }

        let globals = get_globals();
        let motifatom = globals.motifatom;
        drop(globals);

        if e.atom == motifatom {
            update_motif_hints(win);
        }
    }
}

pub fn resize_request(e: &ResizeRequestEvent) {
    if let Some(icon) = win_to_systray_icon(e.window) {
        update_systray_icon_geom(&icon, e.width as i32, e.height as i32);
        if let Some(sel_id) = get_globals().selmon {
            resize_bar_win(sel_id);
        }
        update_systray();
    }
}

pub fn unmap_notify(e: &UnmapNotifyEvent) {
    if let Some(win) = win_to_client(e.window) {
        if e.from_send_event {
            set_client_state(win, WM_STATE_WITHDRAWN);
        } else {
            unmanage(win, false);
        }
    } else if let Some(icon) = win_to_systray_icon(e.window) {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.map_window(icon.win);
            let _ = conn.configure_window(
                icon.win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
        }
        update_systray();
    }
}

pub fn leave_notify(e: &LeaveNotifyEvent) {
    let m = win_to_mon(e.window);
    if let Some(mon_id) = m {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(mon_id) {
            if e.window == mon.barwin {
                drop(globals);
                reset_bar();
            }
        }
    }
}

fn handle_systray_dock_request(e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = data[2];
    if icon_win == 0 {
        return;
    }

    let globals = get_globals();
    let selmon_id = globals.selmon;
    let statusscheme = globals.statusscheme.clone();
    drop(globals);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        if let Ok(cookie) = conn.get_window_attributes(icon_win) {
            if let Ok(wa) = cookie.reply() {
                let mut c = ClientInner::default();
                c.win = icon_win;
                c.x = 0;
                c.oldx = 0;
                c.y = 0;
                c.oldy = 0;
                c.w = wa.width as i32;
                c.oldw = wa.width as i32;
                c.h = wa.height as i32;
                c.oldh = wa.height as i32;
                c.old_border_width = wa.border_width as i32;
                c.border_width = 0;
                c.isfloating = true;
                c.tags = 1;
                c.mon_id = selmon_id;

                update_size_hints(&mut c);
                update_systray_icon_geom(&mut c, wa.width as i32, wa.height as i32);

                let _ = conn.change_save_set(ChangeSaveSetMode::INSERT, icon_win);
                let _ = conn.change_window_attributes(
                    icon_win,
                    &ChangeWindowAttributesAux::new().event_mask(
                        EventMask::STRUCTURE_NOTIFY
                            | EventMask::PROPERTY_CHANGE
                            | EventMask::RESIZE_REDIRECT,
                    ),
                );

                let globals = get_globals();
                let systray_win = globals.systray.as_ref().map(|s| s.win).unwrap_or(0);
                drop(globals);

                let _ = conn.reparent_window(icon_win, systray_win, 0, 0);

                if let Some(ref scheme) = statusscheme {
                    let _ = conn.change_window_attributes(
                        icon_win,
                        &ChangeWindowAttributesAux::new().background_pixel(scheme.pixel),
                    );
                }

                let xembed = get_globals().netatom[NetAtom::Xembed as usize];
                send_event_embed(icon_win, xembed, systray_win, XEMBED_EMBEDDED_NOTIFY);
                send_event_embed(icon_win, xembed, systray_win, XEMBED_FOCUS_IN);
                send_event_embed(icon_win, xembed, systray_win, XEMBED_WINDOW_ACTIVATE);
                send_event_embed(icon_win, xembed, systray_win, XEMBED_MODALITY_ON);

                let _ = conn.flush();

                if let Some(sel_id) = selmon_id {
                    resize_bar_win(sel_id);
                }
                update_systray();
                set_client_state(icon_win, WM_STATE_NORMAL);

                let mut globals = get_globals_mut();
                if let Some(ref mut systray) = globals.systray {
                    systray.icons.push(icon_win as usize);
                }
                globals.clients.insert(icon_win, c);
            }
        }
    }
}

fn send_event_embed(win: Window, xembed: u32, systray_win: Window, message: u32) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: win,
            type_: xembed,
            data: ClientMessageData::from([
                message,
                CURRENT_TIME,
                systray_win,
                XEMBED_EMBEDDED_VERSION,
                0,
            ]),
        };
        let _ = conn.send_event(false, win, EventMask::STRUCTURE_NOTIFY, &event);
        let _ = conn.flush();
    }
}

fn handle_net_wm_state(e: &ClientMessageEvent, win: Window) {
    let data = e.data.as_data32();
    let globals = get_globals();
    let net_wm_fullscreen = globals.netatom[NetAtom::WMFullscreen as usize];
    drop(globals);

    if data[1] == net_wm_fullscreen || data[2] == net_wm_fullscreen {
        let fullscreen = data[0] == 1
            || (data[0] == 2 && {
                let globals = get_globals();
                if let Some(c) = globals.clients.get(&win) {
                    !c.is_fullscreen || c.isfakefullscreen
                } else {
                    false
                }
            });
        set_fullscreen(win, fullscreen);
    }
}

fn handle_active_window(e: &ClientMessageEvent, win: Window) {
    let globals = get_globals();
    let client = globals.clients.get(&win).cloned();
    let selmon_id = globals.selmon;
    drop(globals);

    let Some(c) = client else { return };

    if let Some(mon_id) = c.mon_id {
        let globals = get_globals();
        let is_overlay = globals.monitors.get(mon_id).and_then(|m| m.overlay) == Some(win);
        let is_scratchpad = c.is_scratchpad();
        drop(globals);

        if is_overlay {
            if Some(mon_id) != selmon_id {
                if let Some(old_sel) = selmon_id {
                    if let Some(old_mon) = get_globals().monitors.get(old_sel) {
                        if let Some(sel_win) = old_mon.sel {
                            crate::client::unfocus_win(sel_win, false);
                        }
                    }
                }
                let mut globals = get_globals_mut();
                globals.selmon = Some(mon_id);
                drop(globals);
                focus(None);
            }
            show_overlay(&Arg::default());
        } else if is_scratchpad {
            let mut globals = get_globals_mut();
            globals.selmon = Some(mon_id);
            drop(globals);
            let arg = Arg {
                v: Some(unsafe {
                    std::mem::transmute::<*const u8, usize>(c.scratchpad_name.as_ptr())
                }),
                ..Default::default()
            };
            scratchpad_show(&arg);
        } else {
            handle_active_window_regular(win, mon_id);
        }
    }
}

fn handle_active_window_regular(win: Window, mon_id: MonitorId) {
    let is_hidden = is_hidden(win);
    if is_hidden {
        crate::client::show(win);
    }

    let globals = get_globals();
    let numtags = globals.numtags;
    let client = globals.clients.get(&win).cloned();
    drop(globals);

    let Some(c) = client else { return };

    let mut tag_idx = 0;
    for i in 0..numtags as usize {
        if (1 << i) & c.tags != 0 {
            tag_idx = i;
            break;
        }
    }

    if tag_idx < numtags as usize {
        let arg = Arg {
            ui: 1 << tag_idx,
            ..Default::default()
        };

        let globals = get_globals();
        if Some(mon_id) != globals.selmon {
            if let Some(old_sel) = globals.selmon {
                if let Some(old_mon) = globals.monitors.get(old_sel) {
                    if let Some(sel_win) = old_mon.sel {
                        crate::client::unfocus_win(sel_win, false);
                    }
                }
            }
            drop(globals);
            let mut globals = get_globals_mut();
            globals.selmon = Some(mon_id);
        } else {
            drop(globals);
        }

        view(&arg);
        focus(Some(win));
        restack(&mut get_globals_mut().monitors.get_mut(mon_id).unwrap());
    }
}

fn handle_focus_monitor(win: Window, root_x: i32, root_y: i32) {
    let m = win_to_mon(win);
    if m.is_none() {
        let m = rect_to_mon(root_x, root_y, 1, 1);
        if let Some(m_id) = m {
            let globals = get_globals();
            if Some(m_id) != globals.selmon {
                let old_sel = globals.selmon;
                drop(globals);

                if let Some(old_id) = old_sel {
                    if let Some(old_mon) = get_globals().monitors.get(old_id) {
                        if let Some(sel_win) = old_mon.sel {
                            crate::client::unfocus_win(sel_win, true);
                        }
                    }
                }

                let mut globals = get_globals_mut();
                globals.selmon = Some(m_id);
                drop(globals);
                focus(None);
            }
        }
    } else if let Some(m_id) = m {
        let globals = get_globals();
        if Some(m_id) != globals.selmon {
            let old_sel = globals.selmon;
            drop(globals);

            if let Some(old_id) = old_sel {
                if let Some(old_mon) = get_globals().monitors.get(old_id) {
                    if let Some(sel_win) = old_mon.sel {
                        crate::client::unfocus_win(sel_win, true);
                    }
                }
            }

            let mut globals = get_globals_mut();
            globals.selmon = Some(m_id);
            drop(globals);
            focus(None);
        }
    }
}

fn handle_bar_click(e: &ButtonPressEvent, click: &mut Click, arg: &mut Arg, sel_id: MonitorId) {
    let globals = get_globals();
    let mon = match globals.monitors.get(sel_id) {
        Some(m) => m.clone(),
        None => return,
    };
    let startmenusize = globals.startmenusize;
    let numtags = globals.numtags;
    let lrpad = globals.lrpad;
    let statuswidth = globals.statuswidth;
    drop(globals);

    let mut occupied_tags: u32 = 0;
    for (win, c) in get_globals().clients.iter() {
        if c.mon_id == Some(sel_id) {
            occupied_tags |= if c.tags == 255 { 0 } else { c.tags };
        }
    }

    let mut x = startmenusize as i32;
    let mut i = 0;

    while e.event_x >= x && i < numtags as usize {
        if i < 9 {
            let showtags = mon.showtags;
            let tagset = mon.tagset[mon.seltags as usize];
            if showtags != 0 {
                if (occupied_tags & (1 << i)) == 0 && (tagset & (1 << i)) == 0 {
                    i += 1;
                    continue;
                }
            }
            x += get_text_width(&format!(" {}", i + 1));
        }
        i += 1;
    }

    if e.event_x < startmenusize as i32 {
        *click = Click::StartMenu;
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(sel_id) {
            mon.gesture = Gesture::None;
        }
        drop(globals);
        draw_bar_for_mon(sel_id);
    } else if i < numtags as usize {
        *click = Click::TagBar;
        arg.ui = 1 << i;
    } else {
        let blw = get_layout_symbol_width(sel_id);
        if e.event_x < x + blw {
            *click = Click::LtSymbol;
        } else if mon.sel.is_none() && e.event_x > x + blw && e.event_x < x + blw + get_globals().bh
        {
            *click = Click::ShutDown;
        } else if e.event_x > mon.ww - get_systray_width() as i32 - statuswidth + lrpad as i32 - 2 {
            *click = Click::StatusText;
        } else if mon.stack.is_some() {
            x += blw;
            let mut c = mon.clients;

            while e.event_x > x && c.is_some() {
                let cur_win = c.unwrap();
                let globals = get_globals();
                if let Some(client) = globals.clients.get(&cur_win) {
                    if is_visible(client) {
                        x += (1.0 / mon.bt as f64 * mon.bar_clients_width as f64) as i32;
                    }
                    c = client.next;
                } else {
                    break;
                }
            }

            if let Some(cur_win) = c {
                arg.v = Some(cur_win as usize);
                let titlewidth = (1.0 / mon.bt as f64 * mon.bar_clients_width as f64) as i32;
                let title_start = x - titlewidth;
                let resize_start = title_start + titlewidth - 30;

                if Some(cur_win) == mon.sel && e.event_x < title_start + 32 {
                    *click = Click::CloseButton;
                } else if Some(cur_win) == mon.sel && e.event_x > resize_start {
                    *click = Click::ResizeWidget;
                } else {
                    *click = Click::WinTitle;
                }
            } else {
                *click = Click::RootWin;
            }
        } else {
            *click = Click::RootWin;
        }
    }
}

fn handle_client_click(e: &ButtonPressEvent, win: Window, click: &mut Click) {
    let globals = get_globals();
    let focusfollowsmouse = globals.focusfollowsmouse;
    let selmon_id = globals.selmon;
    drop(globals);

    if focusfollowsmouse || e.detail <= 3 {
        focus(Some(win));
        if let Some(sel_id) = selmon_id {
            let mut globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(sel_id) {
                restack(mon);
            }
        }
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
        let _ = conn.flush();
    }

    *click = Click::ClientWin;
}

fn handle_resize_click(e: &ButtonPressEvent) {
    let globals = get_globals();
    if globals.altcursor == AltCursor::Resize && e.detail == 1 {
        if let Some(sel_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_id) {
                if let Some(sel_win) = mon.sel {
                    let is_floating = globals
                        .clients
                        .get(&sel_win)
                        .map(|c| c.isfloating)
                        .unwrap_or(false);
                    let has_tiling = has_tiling_layout(sel_id);
                    if is_floating || !has_tiling {
                        drop(globals);
                        reset_cursor();
                        resize_mouse(&Arg::default());
                        return;
                    }
                }
            }
        }
    }
}

fn handle_bar_leave_reset(e: &EnterNotifyEvent) {
    unsafe {
        if BAR_LEAVE_STATUS != 0
            && e.root_y as i32
                >= get_globals()
                    .monitors
                    .iter()
                    .next()
                    .map(|m| m.my + 5)
                    .unwrap_or(0)
        {
            reset_bar();
            BAR_LEAVE_STATUS = 0;
        }
    }
}

fn handle_floating_focus(e: &EnterNotifyEvent, c: Option<Window>) -> Option<Window> {
    let globals = get_globals();
    let entering_root = e.event == globals.root;
    let selmon_id = globals.selmon;

    let have_floating_sel = if let Some(sel_id) = selmon_id {
        if let Some(mon) = globals.monitors.get(sel_id) {
            if let Some(sel_win) = mon.sel {
                globals
                    .clients
                    .get(&sel_win)
                    .map(|c| c.isfloating)
                    .unwrap_or(false)
                    || !has_tiling_layout(sel_id)
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    let focusfollowsfloatmouse = globals.focusfollowsfloatmouse;
    drop(globals);

    if entering_root && have_floating_sel {
        let resizeexit = hover_resize_mouse(&Arg::default());
        if resizeexit {
            return None;
        }
        return c;
    }

    let c_val = match c {
        Some(w) => w,
        None => return c,
    };

    let globals = get_globals();
    let selmon_sel = globals
        .selmon
        .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel));
    let client = globals.clients.get(&c_val).cloned();
    drop(globals);

    let Some(sel_win) = selmon_sel else { return c };
    let Some(client_ref) = client else { return c };

    let sel_floating = globals
        .clients
        .get(&sel_win)
        .map(|c| c.isfloating)
        .unwrap_or(false);
    let has_tiling =
        if let Some(mon_id) = globals.monitors.iter().position(|m| m.sel == Some(sel_win)) {
            has_tiling_layout(mon_id)
        } else {
            true
        };

    if !(client_ref.isfloating || !has_tiling) && sel_floating {
        return c;
    }

    if c_val == sel_win {
        return c;
    }

    let is_visible_c = is_visible(&client_ref) || client_ref.issticky;

    if !(entering_root || is_visible_c) {
        return c;
    }

    let resizeexit = hover_resize_mouse(&Arg::default());
    if focusfollowsfloatmouse {
        if resizeexit {
            return None;
        }
        if let Some(newc) = get_cursor_client() {
            if newc.win != sel_win {
                return Some(newc.win);
            }
        }
    } else {
        return None;
    }

    c
}

fn enternotify_monitor_switch(event_win: Window, c: Option<Window>) -> bool {
    let m = if let Some(win) = c {
        get_globals()
            .clients
            .get(&win)
            .and_then(|client| client.mon_id)
    } else {
        win_to_mon(event_win)
    };

    let globals = get_globals();
    if m != globals.selmon {
        if let Some(old_sel) = globals.selmon {
            if let Some(old_mon) = globals.monitors.get(old_sel) {
                if let Some(sel_win) = old_mon.sel {
                    drop(globals);
                    crate::client::unfocus_win(sel_win, true);
                }
            }
        }
        let mut globals = get_globals_mut();
        globals.selmon = m;
        return true;
    }
    false
}

fn should_skip_floating_focus(event_win: Window, c: Option<Window>) -> bool {
    let globals = get_globals();
    if !globals.focusfollowsfloatmouse {
        if event_win != globals.root {
            if let Some(sel_id) = globals.selmon {
                if let Some(mon) = globals.monitors.get(sel_id) {
                    if let Some(sel_win) = mon.sel {
                        let sel_floating = globals
                            .clients
                            .get(&sel_win)
                            .map(|c| c.isfloating)
                            .unwrap_or(false);
                        if let Some(c_win) = c {
                            if let Some(client) = globals.clients.get(&c_win) {
                                if client.isfloating && sel_floating {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

fn handle_floating_resize_hover(sel_id: MonitorId) -> bool {
    let globals = get_globals();
    let mon = match globals.monitors.get(sel_id) {
        Some(m) => m,
        None => return false,
    };

    let sel_win = match mon.sel {
        Some(w) => w,
        None => return false,
    };

    let client = match globals.clients.get(&sel_win) {
        Some(c) => c,
        None => return false,
    };

    if !(client.isfloating || !has_tiling_layout(sel_id)) {
        return false;
    }
        return false;
    }

    let mut tilefound = false;
    for (win, c) in globals.clients.iter() {
        if c.mon_id == Some(sel_id)
            && is_visible(c)
            && !c.isfloating
            && tiling_layout_func(sel_id).is_some()
        {
            tilefound = true;
            break;
        }
    }

    if tilefound {
        return false;
    }

    drop(globals);

    if is_in_resize_border(sel_id) {
        if get_globals().altcursor != AltCursor::Resize {
            define_cursor_resize();
            let mut globals = get_globals_mut();
            globals.altcursor = AltCursor::Resize;
        }

        if let Some(newc) = get_cursor_client() {
            let globals = get_globals();
            if let Some(sel_id) = globals.selmon {
                if let Some(mon) = globals.monitors.get(sel_id) {
                    if mon.sel != Some(newc.win) {
                        drop(globals);
                        focus(Some(newc.win));
                    }
                }
            }
        }
        return true;
    }

    if get_globals().altcursor == AltCursor::Resize {
        reset_cursor();
    }

    false
}

fn handle_sidebar_hover(e: &MotionNotifyEvent, sel_id: MonitorId) -> bool {
    let globals = get_globals();
    let mon = match globals.monitors.get(sel_id) {
        Some(m) => m,
        None => return false,
    };
    let bh = globals.bh;
    drop(globals);

    if e.event_x as i32 > mon.mx + mon.mw - SIDEBAR_WIDTH {
        if globals.altcursor == AltCursor::None && e.event_y as i32 > bh + 60 {
            let mut globals = get_globals_mut();
            globals.altcursor = AltCursor::Sidebar;
            drop(globals);
            define_cursor_vert();
        }
        return true;
    }

    if globals.altcursor == AltCursor::Sidebar {
        let mut globals = get_globals_mut();
        globals.altcursor = AltCursor::None;
        drop(globals);
        undefine_cursor();
        define_cursor_normal();
        return true;
    }

    false
}

fn handle_overlay_gesture(e: &MotionNotifyEvent, sel_id: MonitorId) -> bool {
    let globals = get_globals();
    let mon = match globals.monitors.get(sel_id) {
        Some(m) => m,
        None => return false,
    };
    let systray_width = get_systray_width() as i32;
    drop(globals);

    if e.event_y as i32 == mon.my
        && e.event_x as i32 >= mon.mx + mon.ww - OVERLAY_ACTIVATION_ZONE - systray_width
    {
        if mon.gesture != Gesture::Overlay {
            let mut globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(sel_id) {
                mon.gesture = Gesture::Overlay;
            }
            drop(globals);
            crate::overlay::set_overlay(&Arg::default());
        }
        return true;
    }

    if mon.gesture == Gesture::Overlay {
        if e.event_y as i32 <= mon.my + OVERLAY_KEEP_ZONE_Y
            && e.event_x as i32 >= mon.mx + mon.ww - OVERLAY_KEEP_ZONE_X - systray_width
        {
            return true;
        }
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(sel_id) {
            mon.gesture = Gesture::None;
        }
        return false;
    }

    false
}

fn handle_tagbar_hover(e: &MotionNotifyEvent, sel_id: MonitorId) {
    let globals = get_globals();
    let mon = match globals.monitors.get(sel_id) {
        Some(m) => m.clone(),
        None => return,
    };
    let tagwidth = globals.tagwidth;
    let startmenusize = globals.startmenusize;
    let tags = globals.tags.clone();
    let numtags = globals.numtags;
    drop(globals);

    {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(sel_id) {
            mon.hoverclient = None;
        }
    }

    if (e.event_x as i32) < mon.mx + tagwidth && mon.showtags == 0 {
        if (e.event_x as i32) < mon.mx + startmenusize as i32 {
            let mut globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(sel_id) {
                mon.gesture = Gesture::StartMenu;
            }
            drop(globals);
            draw_bar_for_mon(sel_id);
        } else {
            let mut i = 0;
            let mut x = mon.mx + startmenusize as i32;

            while e.event_x as i32 >= x && i < numtags as usize && i < 8 {
                let tag_str = get_tag_string(&tags, i);
                x += get_text_width(&tag_str);
                i += 1;
            }

            let gesture_val = if i < numtags as usize {
                Gesture::from(i + 1)
            } else {
                Gesture::None
            };
            let current_gesture = mon.gesture;

            if gesture_val != current_gesture {
                let mut globals = get_globals_mut();
                if let Some(mon) = globals.monitors.get_mut(sel_id) {
                    mon.gesture = gesture_val;
                }
                drop(globals);
                draw_bar_for_mon(sel_id);
            }
        }
    } else {
        reset_bar();
    }
}

fn handle_titlebar_hover(e: &MotionNotifyEvent, sel_id: MonitorId) {
    let globals = get_globals();
    let mon = match globals.monitors.get(sel_id) {
        Some(m) => m.clone(),
        None => return,
    };
    let altcursor = globals.altcursor;
    drop(globals);

    if (e.event_x as i32) > mon.activeoffset as i32
        && (e.event_x as i32) < (mon.activeoffset as i32 + CLOSE_BUTTON_HIT_WIDTH)
    {
        if mon.gesture != Gesture::CloseButton {
            let mut globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(sel_id) {
                mon.gesture = Gesture::CloseButton;
            }
            drop(globals);
            draw_bar_for_mon(sel_id);
        }
    } else if mon.gesture == Gesture::CloseButton {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(sel_id) {
            mon.gesture = Gesture::None;
        }
        drop(globals);
        draw_bar_for_mon(sel_id);
    } else {
        let titlewidth = (1.0 / mon.bt as f64 * mon.bar_clients_width as f64) as i32;
        let resize_start = mon.activeoffset as i32 + titlewidth - RESIZE_WIDGET_WIDTH;
        let resize_end = mon.activeoffset as i32 + titlewidth;

        if altcursor == AltCursor::None {
            if (e.event_x as i32) > resize_start && (e.event_x as i32) < resize_end {
                define_cursor_resize();
                let mut globals = get_globals_mut();
                globals.altcursor = AltCursor::Resize;
            }
        } else if (e.event_x as i32) < resize_start || (e.event_x as i32) > resize_end {
            define_cursor_normal();
            let mut globals = get_globals_mut();
            globals.altcursor = AltCursor::None;
        }
    }

    if mon.stack.is_some() {
        let layout_width = get_layout_symbol_width(sel_id);
        let mut x = mon.mx + get_globals().tagwidth + layout_width;
        let mut c = mon.clients;

        let globals = get_globals();
        while e.event_x as i32 > x && c.is_some() {
            let cur_win = c.unwrap();
            if let Some(client) = globals.clients.get(&cur_win) {
                if is_visible(client) {
                    x += (1.0 / mon.bt as f64 * mon.bar_clients_width as f64) as i32;
                }
                c = client.next;
            } else {
                break;
            }
        }
        drop(globals);

        if let Some(cur_win) = c {
            let hoverclient = mon.hoverclient;
            if cur_win != hoverclient {
                let mut globals = get_globals_mut();
                if let Some(mon) = globals.monitors.get_mut(sel_id) {
                    mon.hoverclient = Some(cur_win);
                    mon.gesture = Gesture::None;
                }
                drop(globals);
                draw_bar_for_mon(sel_id);
            }
        }
    }
}

fn is_in_resize_border(_sel_id: MonitorId) -> bool {
    false
}

fn define_cursor_resize() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        if let Some(ref cursor) = globals.cursors[Cursor::Resize as usize] {
            let _ = conn.change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
            );
        }
    }
}

fn define_cursor_vert() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        if let Some(ref cursor) = globals.cursors[Cursor::Vert as usize] {
            let _ = conn.change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
            );
        }
    }
}

fn define_cursor_normal() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        if let Some(ref cursor) = globals.cursors[Cursor::Normal as usize] {
            let _ = conn.change_window_attributes(
                root,
                &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
            );
        }
    }
}

fn undefine_cursor() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().cursor(0));
    }
}

fn resize_bar_win(mon_id: MonitorId) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(mon_id) {
            let bh = globals.bh;
            let _ = conn.configure_window(
                mon.barwin,
                &ConfigureWindowAux::new()
                    .x(mon.wx)
                    .y(mon.by)
                    .width(mon.ww as u32)
                    .height(bh as u32),
            );
        }
    }
}

fn draw_bar_for_mon(mon_id: MonitorId) {
    let mut globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(mon_id) {
        draw_bar(mon);
    }
}

fn get_layout_symbol_width(_mon_id: MonitorId) -> i32 {
    40
}

fn get_text_width(_text: &str) -> i32 {
    30
}

fn get_tag_string(tags: &[[u8; 16]], idx: usize) -> String {
    if idx < tags.len() {
        let end = tags[idx].iter().position(|&b| b == 0).unwrap_or(16);
        String::from_utf8_lossy(&tags[idx][..end]).to_string()
    } else {
        String::new()
    }
}

fn xcommand() -> bool {
    let globals = get_globals();
    let stext = &globals.stext;
    let stext_str =
        String::from_utf8_lossy(&stext[..stext.iter().position(|&b| b == 0).unwrap_or(0)]);

    if stext_str.starts_with("cmd:") {
        let cmd = &stext_str[4..];
        for xcmd in &globals.commands {
            if cmd.starts_with(xcmd.cmd) {
                if let Some(func) = xcmd.func {
                    drop(globals);
                    func(&xcmd.arg);
                    return true;
                }
            }
        }
    }
    false
}

pub type EventHandler = fn(&[u8]);

pub struct EventHandlerTable {
    handlers: [Option<EventHandler>; 36],
}

impl EventHandlerTable {
    pub fn new() -> Self {
        let mut table = Self {
            handlers: [None; 36],
        };

        table.handlers[ButtonPressEvent::NUMBER as usize] = Some(Self::button_press_handler);
        table.handlers[ClientMessageEvent::NUMBER as usize] = Some(Self::client_message_handler);
        table.handlers[ConfigureNotifyEvent::NUMBER as usize] =
            Some(Self::configure_notify_handler);
        table.handlers[ConfigureRequestEvent::NUMBER as usize] =
            Some(Self::configure_request_handler);
        table.handlers[DestroyNotifyEvent::NUMBER as usize] = Some(Self::destroy_notify_handler);
        table.handlers[EnterNotifyEvent::NUMBER as usize] = Some(Self::enter_notify_handler);
        table.handlers[ExposeEvent::NUMBER as usize] = Some(Self::expose_handler);
        table.handlers[FocusInEvent::NUMBER as usize] = Some(Self::focus_in_handler);
        table.handlers[KeyPressEvent::NUMBER as usize] = Some(Self::key_press_handler);
        table.handlers[KeyReleaseEvent::NUMBER as usize] = Some(Self::key_release_handler);
        table.handlers[MappingNotifyEvent::NUMBER as usize] = Some(Self::mapping_notify_handler);
        table.handlers[MapRequestEvent::NUMBER as usize] = Some(Self::map_request_handler);
        table.handlers[MotionNotifyEvent::NUMBER as usize] = Some(Self::motion_notify_handler);
        table.handlers[PropertyNotifyEvent::NUMBER as usize] = Some(Self::property_notify_handler);
        table.handlers[ResizeRequestEvent::NUMBER as usize] = Some(Self::resize_request_handler);
        table.handlers[UnmapNotifyEvent::NUMBER as usize] = Some(Self::unmap_notify_handler);
        table.handlers[LeaveNotifyEvent::NUMBER as usize] = Some(Self::leave_notify_handler);

        table
    }

    pub fn get_handler(&self, event_type: u8) -> Option<EventHandler> {
        if (event_type as usize) < self.handlers.len() {
            self.handlers[event_type as usize]
        } else {
            None
        }
    }

    fn button_press_handler(data: &[u8]) {
        if let Ok(e) = ButtonPressEvent::try_parse(data) {
            button_press(&e);
        }
    }

    fn client_message_handler(data: &[u8]) {
        if let Ok(e) = ClientMessageEvent::try_parse(data) {
            client_message(&e);
        }
    }

    fn configure_notify_handler(data: &[u8]) {
        if let Ok(e) = ConfigureNotifyEvent::try_parse(data) {
            configure_notify(&e);
        }
    }

    fn configure_request_handler(data: &[u8]) {
        if let Ok(e) = ConfigureRequestEvent::try_parse(data) {
            configure_request(&e);
        }
    }

    fn destroy_notify_handler(data: &[u8]) {
        if let Ok(e) = DestroyNotifyEvent::try_parse(data) {
            destroy_notify(&e);
        }
    }

    fn enter_notify_handler(data: &[u8]) {
        if let Ok(e) = EnterNotifyEvent::try_parse(data) {
            enter_notify(&e);
        }
    }

    fn expose_handler(data: &[u8]) {
        if let Ok(e) = ExposeEvent::try_parse(data) {
            expose(&e);
        }
    }

    fn focus_in_handler(data: &[u8]) {
        if let Ok(e) = FocusInEvent::try_parse(data) {
            focus_in(&e);
        }
    }

    fn key_press_handler(data: &[u8]) {
        if let Ok(e) = KeyPressEvent::try_parse(data) {
            key_press(&e);
        }
    }

    fn key_release_handler(data: &[u8]) {
        if let Ok(e) = KeyReleaseEvent::try_parse(data) {
            key_release(&e);
        }
    }

    fn mapping_notify_handler(data: &[u8]) {
        if let Ok(e) = MappingNotifyEvent::try_parse(data) {
            mapping_notify(&e);
        }
    }

    fn map_request_handler(data: &[u8]) {
        if let Ok(e) = MapRequestEvent::try_parse(data) {
            map_request(&e);
        }
    }

    fn motion_notify_handler(data: &[u8]) {
        if let Ok(e) = MotionNotifyEvent::try_parse(data) {
            motion_notify(&e);
        }
    }

    fn property_notify_handler(data: &[u8]) {
        if let Ok(e) = PropertyNotifyEvent::try_parse(data) {
            property_notify(&e);
        }
    }

    fn resize_request_handler(data: &[u8]) {
        if let Ok(e) = ResizeRequestEvent::try_parse(data) {
            resize_request(&e);
        }
    }

    fn unmap_notify_handler(data: &[u8]) {
        if let Ok(e) = UnmapNotifyEvent::try_parse(data) {
            unmap_notify(&e);
        }
    }

    fn leave_notify_handler(data: &[u8]) {
        if let Ok(e) = LeaveNotifyEvent::try_parse(data) {
            leave_notify(&e);
        }
    }
}

impl Default for EventHandlerTable {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run() {
    let handler_table = EventHandlerTable::new();

    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    loop {
        match conn.wait_for_event() {
            Ok(event) => {
                let event_type = event.response_type() & 0x7f;
                if let Some(handler) = handler_table.get_handler(event_type) {
                    handler(&event);
                }
            }
            Err(_) => break,
        }
    }
}

pub fn scan() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let globals = get_globals();
    let root = globals.root;
    drop(globals);

    if let Ok(cookie) = conn.query_tree(root) {
        if let Ok(reply) = cookie.reply() {
            for win in reply.children {
                if let Ok(wa_cookie) = conn.get_window_attributes(win) {
                    if let Ok(wa) = wa_cookie.reply() {
                        if !wa.override_redirect && wa.map_state == MapState::Viewable {
                            if win_to_client(win).is_none() {
                                crate::client::manage(
                                    win,
                                    wa.x,
                                    wa.y,
                                    wa.width as u32,
                                    wa.height as u32,
                                    wa.border_width as u32,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn check_other_wm() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let globals = get_globals();
    let root = globals.root;
    drop(globals);

    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));

    if let Err(_) = conn.check_for_error() {
        crate::util::die("instantwm: another window manager is already running");
    }
}

pub fn setup() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let screen = conn.setup().roots.get(x11.screen_num).cloned();
    let Some(screen) = screen else { return };

    let root = screen.root;

    {
        let mut globals = get_globals_mut();
        globals.screen = x11.screen_num as i32;
        globals.root = root;
        globals.sw = screen.width_in_pixels as i32;
        globals.sh = screen.height_in_pixels as i32;
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

    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));

    let _ = conn.grab_server();

    for child in &conn.query_tree(root).unwrap().reply().unwrap().children {
        let _ = conn.change_window_attributes(
            *child,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        );
    }

    let _ = conn.ungrab_server();

    let _ = conn.flush();

    update_geom();
    grab_keys();
}

pub fn cleanup() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let globals = get_globals();
    let root = globals.root;
    drop(globals);

    let _ = conn.grab_server();

    for mon in &get_globals().monitors {
        let mut current = mon.clients;
        while let Some(win) = current {
            if let Some(c) = get_globals().clients.get(&win) {
                let old_bw = c.old_border_width;
                current = c.next;

                let _ = conn.change_window_attributes(
                    win,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
                );
                let _ = conn
                    .configure_window(win, &ConfigureWindowAux::new().border_width(old_bw as u32));
                let _ = conn.ungrab_button(0, 0, win);
            } else {
                break;
            }
        }
    }

    let _ = conn.ungrab_server();
    let _ = conn.flush();
}
