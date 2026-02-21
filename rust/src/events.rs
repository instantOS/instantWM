use crate::bar::{draw_bar, get_tag_width, reset_bar};
use crate::client::{
    configure, is_hidden, set_client_state, set_fullscreen, unmanage, update_title,
    update_wm_hints, win_to_client, WM_STATE_WITHDRAWN,
};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::keyboard::grab_keys;
use crate::monitor::{arrange, restack, update_geom, win_to_mon};
use crate::mouse::{reset_cursor, resize_mouse};
use crate::systray::{update_systray, win_to_systray_icon};
use crate::types::*;
use crate::util::clean_mask;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

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

pub fn button_press(_e: &ButtonPressEvent) {
    let globals = get_globals();
    let numlockmask = globals.numlockmask;
    let buttons = globals.buttons.clone();
    let altcursor = globals.altcursor;
    let selmon_id = globals.selmon;
    drop(globals);

    if let Some(sel_id) = selmon_id {
        if let Some(mon) = get_globals().monitors.get(sel_id) {
            if let Some(sel_win) = mon.sel {
                let is_floating = get_globals()
                    .clients
                    .get(&sel_win)
                    .map(|c| c.isfloating)
                    .unwrap_or(false);
                let has_tiling = has_tiling_layout(sel_id);
                if altcursor == AltCursor::Resize && (is_floating || !has_tiling) {
                    reset_cursor();
                    resize_mouse(&Arg::default());
                    return;
                }
            }
        }
    }

    for button in &buttons {
        if button.func.is_some() {
            let clean_button_mask = clean_mask(button.mask, numlockmask);
            if clean_button_mask == clean_mask(0, numlockmask) {
                if let Some(func) = button.func {
                    func(&button.arg);
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
    let net_active_window = globals.netatom[NetAtom::ActiveWindow as usize];
    drop(globals);

    if showsystray && e.window == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(e);
        }
        return;
    }

    let Some(win) = win_to_client(e.window) else {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(e, win);
    } else if e.type_ == net_active_window {
        handle_active_window(win);
    }
}

pub fn configure_notify(e: &ConfigureNotifyEvent) {
    let globals = get_globals();
    if e.window != globals.root {
        return;
    }
    drop(globals);

    {
        let mut globals = get_globals_mut();
        globals.sw = e.width as i32;
        globals.sh = e.height as i32;
    }

    update_geom();
    focus(None);
    arrange(None);
}

pub fn configure_request(e: &ConfigureRequestEvent) {
    if let Some(win) = win_to_client(e.window) {
        configure(win);
    } else {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.configure_window(
                e.window,
                &ConfigureWindowAux::new()
                    .x(e.x as i32)
                    .y(e.y as i32)
                    .width(e.width as u32)
                    .height(e.height as u32)
                    .border_width(e.border_width as u32),
            );
            let _ = conn.flush();
        }
    }
}

pub fn destroy_notify(e: &DestroyNotifyEvent) {
    if let Some(win) = win_to_client(e.window) {
        unmanage(win, true);
    } else if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
    }
}

pub fn enter_notify(e: &EnterNotifyEvent) {
    handle_bar_leave_reset(e);

    let globals = get_globals();
    if !globals.focusfollowsmouse {
        return;
    }
    drop(globals);

    let c = win_to_client(e.event);
    if let Some(win) = c {
        let globals = get_globals();
        if let Some(sel_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_id) {
                if mon.sel != Some(win) {
                    drop(globals);
                    focus(Some(win));
                }
            }
        }
    }
}

pub fn expose(e: &ExposeEvent) {
    if e.count != 0 {
        return;
    }

    if let Some(mon_id) = win_to_mon(e.window) {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            draw_bar(mon);
        }
    }
}

pub fn focus_in(_e: &FocusInEvent) {
    let globals = get_globals();
    if let Some(sel_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_id) {
            if let Some(sel_win) = mon.sel {
                drop(globals);
                crate::client::set_focus(sel_win);
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

pub fn key_release(_e: &KeyReleaseEvent) {}

pub fn mapping_notify(_e: &MappingNotifyEvent) {
    grab_keys();
}

pub fn map_request(e: &MapRequestEvent) {
    if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
        return;
    }

    if win_to_client(e.window).is_none() {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            if let Ok(cookie) = conn.get_window_attributes(e.window) {
                if let Ok(wa) = cookie.reply() {
                    if !wa.override_redirect {
                        crate::client::manage(e.window, 0, 0, 800, 600, 1);
                    }
                }
            }
        }
    }
}

pub fn motion_notify(_e: &MotionNotifyEvent) {
    let globals = get_globals();
    let mut tagwidth = globals.tagwidth;
    if tagwidth == 0 {
        tagwidth = get_tag_width();
        drop(globals);
        let mut globals = get_globals_mut();
        globals.tagwidth = tagwidth;
    }
}

pub fn property_notify(e: &PropertyNotifyEvent) {
    if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
        return;
    }

    let globals = get_globals();
    if e.window == globals.root {
        drop(globals);
        crate::bar::update_status();
        return;
    }
    drop(globals);

    if let Some(win) = win_to_client(e.window) {
        match e.atom {
            x if x == AtomEnum::WM_NORMAL_HINTS.into() => {
                let mut globals = get_globals_mut();
                if let Some(c) = globals.clients.get_mut(&win) {
                    c.hintsvalid = 0;
                }
            }
            x if x == AtomEnum::WM_HINTS.into() => {
                update_wm_hints(win);
            }
            _ => {}
        }

        let net_wm_name = get_globals().netatom[NetAtom::WMName as usize];
        if e.atom == AtomEnum::WM_NAME.into() || e.atom == net_wm_name {
            update_title(win);
        }
    }
}

pub fn resize_request(e: &ResizeRequestEvent) {
    if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
    }
}

pub fn unmap_notify(e: &UnmapNotifyEvent) {
    if let Some(win) = win_to_client(e.window) {
        set_client_state(win, WM_STATE_WITHDRAWN);
        unmanage(win, false);
    } else if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
    }
}

pub fn leave_notify(_e: &LeaveNotifyEvent) {
    reset_bar();
}

fn handle_systray_dock_request(_e: &ClientMessageEvent) {}

fn handle_net_wm_state(e: &ClientMessageEvent, win: Window) {
    let data = e.data.as_data32();
    let fullscreen_action = data[0];

    if fullscreen_action == 1 {
        set_fullscreen(win, true);
    } else if fullscreen_action == 0 {
        set_fullscreen(win, false);
    }
}

fn handle_active_window(win: Window) {
    let is_hidden = is_hidden(win);
    if is_hidden {
        crate::client::show(win);
    }

    let globals = get_globals();
    if let Some(c) = globals.clients.get(&win) {
        if let Some(mon_id) = c.mon_id {
            drop(globals);
            focus(Some(win));
            let mut globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(mon_id) {
                restack(mon);
            }
        }
    }
}

fn handle_bar_leave_reset(_e: &EnterNotifyEvent) {
    unsafe {
        if BAR_LEAVE_STATUS != 0 {
            reset_bar();
            BAR_LEAVE_STATUS = 0;
        }
    }
}

pub fn run() {
    eprintln!("TRACE: events::run - START");

    eprintln!("TRACE: events::run - entering event loop");
    loop {
        // Acquire the X11 borrow only to wait for the event, then release
        // it before dispatching. This prevents deadlocks since event handlers
        // also need to borrow X11.
        let event = {
            let x11 = get_x11();
            let Some(ref conn) = x11.conn else {
                eprintln!("TRACE: events::run - no connection, returning");
                return;
            };
            match conn.wait_for_event() {
                Ok(event) => event,
                Err(e) => {
                    eprintln!("TRACE: events::run - error waiting for event: {:?}", e);
                    return;
                }
            }
        }; // X11 borrow released here

        dispatch_event(event);
    }
}

fn dispatch_event(event: x11rb::protocol::Event) {
    eprintln!(
        "TRACE: dispatch_event - event type: {}",
        event.response_type()
    );
    match event {
        x11rb::protocol::Event::ButtonPress(e) => {
            eprintln!("TRACE: dispatch_event - ButtonPress");
            button_press(&e)
        }
        x11rb::protocol::Event::ClientMessage(e) => {
            eprintln!("TRACE: dispatch_event - ClientMessage");
            client_message(&e)
        }
        x11rb::protocol::Event::ConfigureNotify(e) => {
            eprintln!("TRACE: dispatch_event - ConfigureNotify");
            configure_notify(&e)
        }
        x11rb::protocol::Event::ConfigureRequest(e) => {
            eprintln!("TRACE: dispatch_event - ConfigureRequest");
            configure_request(&e)
        }
        x11rb::protocol::Event::DestroyNotify(e) => {
            eprintln!("TRACE: dispatch_event - DestroyNotify");
            destroy_notify(&e)
        }
        x11rb::protocol::Event::EnterNotify(e) => {
            eprintln!("TRACE: dispatch_event - EnterNotify");
            enter_notify(&e)
        }
        x11rb::protocol::Event::Expose(e) => {
            eprintln!("TRACE: dispatch_event - Expose");
            expose(&e)
        }
        x11rb::protocol::Event::FocusIn(e) => {
            eprintln!("TRACE: dispatch_event - FocusIn");
            focus_in(&e)
        }
        x11rb::protocol::Event::KeyPress(e) => {
            eprintln!("TRACE: dispatch_event - KeyPress");
            key_press(&e)
        }
        x11rb::protocol::Event::KeyRelease(e) => {
            eprintln!("TRACE: dispatch_event - KeyRelease");
            key_release(&e)
        }
        x11rb::protocol::Event::MappingNotify(e) => {
            eprintln!("TRACE: dispatch_event - MappingNotify");
            mapping_notify(&e)
        }
        x11rb::protocol::Event::MapRequest(e) => {
            eprintln!("TRACE: dispatch_event - MapRequest");
            map_request(&e)
        }
        x11rb::protocol::Event::MotionNotify(e) => {
            eprintln!("TRACE: dispatch_event - MotionNotify");
            motion_notify(&e)
        }
        x11rb::protocol::Event::PropertyNotify(e) => {
            eprintln!("TRACE: dispatch_event - PropertyNotify");
            property_notify(&e)
        }
        x11rb::protocol::Event::ResizeRequest(e) => {
            eprintln!("TRACE: dispatch_event - ResizeRequest");
            resize_request(&e)
        }
        x11rb::protocol::Event::UnmapNotify(e) => {
            eprintln!("TRACE: dispatch_event - UnmapNotify");
            unmap_notify(&e)
        }
        x11rb::protocol::Event::LeaveNotify(e) => {
            eprintln!("TRACE: dispatch_event - LeaveNotify");
            leave_notify(&e)
        }
        _ => {
            eprintln!("TRACE: dispatch_event - other event type");
        }
    }
}

pub fn scan() {
    let root = get_globals().root;

    let children = {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            if let Ok(cookie) = conn.query_tree(root) {
                if let Ok(reply) = cookie.reply() {
                    Some(reply.children)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(children) = children {
        for win in children {
            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                if let Ok(wa_cookie) = conn.get_window_attributes(win) {
                    if let Ok(wa) = wa_cookie.reply() {
                        if !wa.override_redirect {
                            if win_to_client(win).is_none() {
                                crate::client::manage(win, 0, 0, 800, 600, 1);
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

    let root = get_globals().root;
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
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
    let _ = conn.flush();

    update_geom();
    grab_keys();
}

pub fn cleanup() {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let _ = conn.grab_server();

    let globals = get_globals();
    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(win) = current {
            if let Some(c) = globals.clients.get(&win) {
                let old_bw = c.old_border_width;
                current = c.next;
                let _ = conn
                    .configure_window(win, &ConfigureWindowAux::new().border_width(old_bw as u32));
            } else {
                break;
            }
        }
    }

    let _ = conn.ungrab_server();
    let _ = conn.flush();
}
