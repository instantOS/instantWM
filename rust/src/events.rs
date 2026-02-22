use crate::bar::{draw_bar, draw_bars, get_layout_symbol_width, reset_bar};
use crate::client::{
    configure, is_hidden, set_client_state, set_fullscreen, unmanage, update_title,
    update_wm_hints, win_to_client, WM_STATE_WITHDRAWN,
};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11, RUNNING};
use crate::keyboard::{
    grab_keys, key_press as keyboard_key_press, key_release as keyboard_key_release,
};
use crate::monitor::{arrange, rect_to_mon_rect, restack, update_geom, win_to_mon};
use crate::mouse::{reset_cursor, resize_mouse};
use crate::systray::{get_systray_width, update_systray, win_to_systray_icon};
use crate::tags::{get_tag_at_x, get_tag_width};
use crate::types::*;
use crate::util::clean_mask;
use std::sync::atomic::{AtomicI32, Ordering};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;

/// Non-zero when the cursor left the bar and a reset is pending.
static BAR_LEAVE_STATUS: AtomicI32 = AtomicI32::new(0);

fn has_tiling_layout(mon_id: MonitorId) -> bool {
    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(mon_id) {
        crate::monitor::is_current_layout_tiling(mon, &globals.tags)
    } else {
        false
    }
}

fn classify_bar_click(e: &ButtonPressEvent, mon_id: MonitorId) -> (Click, Arg) {
    let mut arg = Arg::default();
    let g = get_globals();
    let Some(mon) = g.monitors.get(mon_id).cloned() else {
        return (Click::RootWin, arg);
    };

    let ev_x = e.event_x as i32;
    let start_menu_size = g.startmenusize;
    let tag_end = get_tag_width();
    let blw = get_layout_symbol_width(&mon);

    let status_hit_x =
        mon.work_rect.w - get_systray_width() as i32 - g.status_text_width + g.lrpad - 2;
    let bh = g.bh;

    if ev_x < start_menu_size {
        reset_bar();
        return (Click::StartMenu, arg);
    }

    let tag_idx = get_tag_at_x(ev_x);
    if tag_idx >= 0 {
        arg.ui = 1u32 << (tag_idx as u32);
        return (Click::TagBar, arg);
    }

    if ev_x < tag_end + blw {
        return (Click::LtSymbol, arg);
    }

    if mon.sel.is_none() && ev_x > tag_end + blw && ev_x < tag_end + blw + bh {
        return (Click::ShutDown, arg);
    }

    if ev_x > status_hit_x {
        return (Click::StatusText, arg);
    }

    let g = get_globals();
    let mut visible_clients: Vec<Window> = Vec::new();
    let mut current = mon.clients;
    while let Some(c_win) = current {
        let Some(c) = g.clients.get(&c_win) else {
            break;
        };
        current = c.next;
        if c.is_visible() {
            visible_clients.push(c_win);
        }
    }

    if !visible_clients.is_empty() {
        let mut title_end = tag_end + blw;
        let total_width = if mon.bar_clients_width > 0 {
            mon.bar_clients_width + 1
        } else {
            (mon.work_rect.w - title_end).max(0)
        };
        let each_width = total_width / visible_clients.len() as i32;
        let mut remainder = total_width % visible_clients.len() as i32;

        for c_win in visible_clients {
            let mut this_width = each_width;
            if remainder > 0 {
                this_width += 1;
                remainder -= 1;
            }
            title_end += this_width;
            if ev_x > title_end {
                continue;
            }

            arg.v = Some(c_win as usize);
            let title_start = title_end - this_width;
            let resize_start = title_start + this_width - RESIZE_WIDGET_WIDTH;
            if mon.sel == Some(c_win) && ev_x < title_start + CLOSE_BUTTON_HIT_WIDTH {
                return (Click::CloseButton, arg);
            }
            if mon.sel == Some(c_win) && ev_x > resize_start {
                return (Click::ResizeWidget, arg);
            }
            return (Click::WinTitle, arg);
        }
    }

    (Click::RootWin, arg)
}

pub fn button_press(e: &ButtonPressEvent) {
    // Client button grabs use GrabMode::SYNC; replay pointer events like dwm.
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
        let _ = conn.flush();
    }

    let globals = get_globals();
    let numlockmask = globals.numlockmask;
    let buttons = globals.buttons.clone();
    let altcursor = globals.altcursor;
    let mut selmon_id = globals.selmon;
    let focusfollowsmouse = globals.focusfollowsmouse;

    if let Some(clicked_mon) = win_to_mon(e.event) {
        if selmon_id != clicked_mon && (focusfollowsmouse || e.detail <= 3) {
            let globals = get_globals_mut();
            globals.selmon = clicked_mon;
            selmon_id = clicked_mon;
            focus(None);
        }
    }

    let mut click_target = Click::RootWin;
    let mut click_arg = Arg::default();

    if let Some(win) = win_to_client(e.event) {
        click_target = Click::ClientWin;
        click_arg.v = Some(win as usize);
        if focusfollowsmouse || e.detail <= 3 {
            focus(Some(win));
            let globals = get_globals_mut();
            if let Some(mon_id) = globals.clients.get(&win).and_then(|c| c.mon_id) {
                if let Some(mon) = globals.monitors.get_mut(mon_id) {
                    restack(mon);
                }
            }
        }
    } else {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            if e.event == mon.barwin {
                (click_target, click_arg) = classify_bar_click(e, selmon_id);
            } else if (e.root_x as i32) > mon.monitor_rect.x + mon.monitor_rect.w - 50 {
                click_target = Click::SideBar;
            }
        }
    }

    if click_target == Click::RootWin {
        if let Some(mon) = get_globals().monitors.get(selmon_id) {
            if let Some(sel_win) = mon.sel {
                let is_floating = get_globals()
                    .clients
                    .get(&sel_win)
                    .map(|c| c.isfloating)
                    .unwrap_or(false);
                let has_tiling = has_tiling_layout(selmon_id);
                if altcursor == AltCursor::Resize && (is_floating || !has_tiling) {
                    reset_cursor();
                    resize_mouse(&Arg::default());
                    return;
                }
            }
        }
    }

    let clean_state = clean_mask(e.state.into(), numlockmask);

    for button in &buttons {
        if button.click != click_target || button.button != e.detail {
            continue;
        }
        if clean_mask(button.mask, numlockmask) != clean_state {
            continue;
        }
        if let Some(func) = button.func {
            let dispatch_arg = if matches!(
                click_target,
                Click::TagBar
                    | Click::WinTitle
                    | Click::CloseButton
                    | Click::ShutDown
                    | Click::SideBar
                    | Click::ResizeWidget
            ) && button.arg.i == 0
            {
                click_arg
            } else {
                button.arg
            };
            func(&dispatch_arg);
        }
    }
}

pub fn client_message(e: &ClientMessageEvent) {
    let globals = get_globals();
    let showsystray = globals.showsystray;
    let systray_win = globals.systray.as_ref().map(|s| s.win).unwrap_or(0);
    let net_system_tray_op = globals.netatom.system_tray_op;
    let net_wm_state = globals.netatom.wm_state;
    let net_active_window = globals.netatom.active_window;

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

    {
        let globals = get_globals_mut();
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

pub fn create_notify(_e: &CreateNotifyEvent) {}

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

    let c = win_to_client(e.event);
    if let Some(win) = c {
        let globals = get_globals();
        let sel_id = globals.selmon;
        if let Some(mon) = globals.monitors.get(sel_id) {
            if mon.sel != Some(win) {
                focus(Some(win));
            }
        }
    }
}

pub fn expose(e: &ExposeEvent) {
    if e.count != 0 {
        return;
    }

    if let Some(mon_id) = win_to_mon(e.window) {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            if e.window != mon.barwin {
                return;
            }
            draw_bar(mon);
        }
    }
}

pub fn focus_in(_e: &FocusInEvent) {
    let globals = get_globals();
    let sel_id = globals.selmon;
    if let Some(mon) = globals.monitors.get(sel_id) {
        if let Some(sel_win) = mon.sel {
            crate::client::set_focus(sel_win);
        }
    }
}

pub fn key_press(e: &KeyPressEvent) {
    keyboard_key_press(e);
}

pub fn key_release(e: &KeyReleaseEvent) {
    keyboard_key_release(e);
}

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
            let override_redirect = conn
                .get_window_attributes(e.window)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|wa| wa.override_redirect)
                .unwrap_or(false);
            if !override_redirect {
                let (x, y, width, height, border_width) = conn
                    .get_geometry(e.window)
                    .ok()
                    .and_then(|geo| geo.reply().ok())
                    .map(|geo| {
                        (
                            //TODO: we should probably use the rectangle struct here
                            // and make manage take a rectangle as a parameter
                            geo.x as i32,
                            geo.y as i32,
                            geo.width as u32,
                            geo.height as u32,
                            geo.border_width as u32,
                        )
                    })
                    .unwrap_or((0, 0, 800, 600, 1));
                crate::client::manage(e.window, x, y, width, height, border_width);
            }
        }
    }
}

/// Handle focus-follows-mouse by switching to the monitor under the cursor.
/// Returns true if a monitor switch occurred (caller should return early).
fn handle_focus_follows_mouse(selmon_id: MonitorId, root_x: i32, root_y: i32) -> bool {
    let globals = get_globals();
    if !globals.focusfollowsmouse {
        return false;
    }

    if let Some(new_mon) = rect_to_mon_rect(&Rect {
        x: root_x,
        y: root_y,
        w: 1,
        h: 1,
    }) {
        if new_mon != selmon_id {
            let globals = get_globals_mut();
            globals.selmon = new_mon;
            focus(None);
            return true;
        }
    }
    false
}

/// Get bar layout information for gesture detection.
fn get_bar_layout_info(mon_id: MonitorId, tagwidth: i32) -> Option<BarLayoutInfo> {
    let globals = get_globals();
    let mon = globals.monitors.get(mon_id)?;
    Some(BarLayoutInfo {
        monitor_x: mon.monitor_rect.x,
        monitor_y: mon.monitor_rect.y,
        bar_height: globals.bh,
        start_menu_size: globals.startmenusize,
        active_offset: mon.activeoffset as i32,
        bar_clients_width: mon.bar_clients_width,
        current_gesture: mon.gesture,
        has_selection: mon.sel.is_some(),
        tag_area_limit: mon.monitor_rect.x + tagwidth + get_layout_symbol_width(mon),
    })
}

/// Information needed for bar gesture detection.
struct BarLayoutInfo {
    monitor_x: i32,
    monitor_y: i32,
    bar_height: i32,
    start_menu_size: i32,
    active_offset: i32,
    bar_clients_width: i32,
    current_gesture: Gesture,
    has_selection: bool,
    tag_area_limit: i32,
}

/// Determine the gesture based on cursor position in the tag area.
fn detect_tag_area_gesture(root_x: i32, info: &BarLayoutInfo) -> Gesture {
    if root_x < info.monitor_x + info.start_menu_size {
        Gesture::StartMenu
    } else {
        let local_x = root_x - info.monitor_x;
        let tag = crate::tags::get_tag_at_x(local_x);
        if tag >= 0 {
            Gesture::from_tag_index(tag as usize).unwrap_or(Gesture::None)
        } else {
            Gesture::None
        }
    }
}

/// Determine the gesture based on cursor position in the title area.
fn detect_title_area_gesture(root_x: i32, info: &BarLayoutInfo) -> Gesture {
    if root_x > info.active_offset && root_x < info.active_offset + CLOSE_BUTTON_HIT_WIDTH {
        Gesture::CloseButton
    } else {
        Gesture::None
    }
}

/// Update the gesture state and redraw the bar if it changed.
fn update_gesture_state(mon_id: MonitorId, new_gesture: Gesture, current_gesture: Gesture) {
    if new_gesture != current_gesture {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(mon_id) {
            mon.gesture = new_gesture;
            draw_bar(mon);
        }
    }
}

/// Handle mouse motion events for bar gesture detection and focus-follows-mouse.
pub fn motion_notify(e: &MotionNotifyEvent) {
    let globals = get_globals();
    if e.event != globals.root {
        return;
    }

    let selmon_id = globals.selmon;
    let tagwidth = get_tag_width();

    // Update cached tag width
    {
        let globals = get_globals_mut();
        globals.tags.width = tagwidth;
    }

    let root_x = e.root_x as i32;
    let root_y = e.root_y as i32;

    // Handle focus-follows-mouse monitor switching
    if handle_focus_follows_mouse(selmon_id, root_x, root_y) {
        return;
    }

    // Get bar layout info for gesture detection
    let Some(layout_info) = get_bar_layout_info(selmon_id, tagwidth) else {
        return;
    };

    // Reset bar if cursor is below the bar area
    if root_y >= layout_info.monitor_y + layout_info.bar_height - 3 {
        reset_bar();
        return;
    }

    // Determine gesture based on cursor position
    let new_gesture = if root_x < layout_info.tag_area_limit {
        detect_tag_area_gesture(root_x, &layout_info)
    } else {
        let title_limit = layout_info.tag_area_limit + layout_info.bar_clients_width;
        if layout_info.has_selection && root_x < title_limit {
            detect_title_area_gesture(root_x, &layout_info)
        } else {
            reset_bar();
            return;
        }
    };

    update_gesture_state(selmon_id, new_gesture, layout_info.current_gesture);
}

pub fn property_notify(e: &PropertyNotifyEvent) {
    if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
        return;
    }

    let globals = get_globals();
    if e.window == globals.root {
        crate::bar::update_status();
        return;
    }

    if let Some(win) = win_to_client(e.window) {
        match e.atom {
            x if x == AtomEnum::WM_NORMAL_HINTS.into() => {
                let globals = get_globals_mut();
                if let Some(c) = globals.clients.get_mut(&win) {
                    c.hintsvalid = 0;
                }
            }
            x if x == AtomEnum::WM_HINTS.into() => {
                update_wm_hints(win);
                draw_bars();
            }
            _ => {}
        }

        let net_wm_name = get_globals().netatom.wm_name;
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
            focus(Some(win));
            let globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(mon_id) {
                restack(mon);
            }
        }
    }
}

fn handle_bar_leave_reset(_e: &EnterNotifyEvent) {
    if BAR_LEAVE_STATUS.load(Ordering::Relaxed) != 0 {
        reset_bar();
        BAR_LEAVE_STATUS.store(0, Ordering::Relaxed);
    }
}

pub fn run() {
    while RUNNING.load(Ordering::SeqCst) {
        let event = {
            let x11 = get_x11();
            let Some(ref conn) = x11.conn else {
                return;
            };
            match conn.wait_for_event() {
                Ok(event) => event,
                Err(_) => return,
            }
        };
        dispatch_event(event);
    }
}

fn dispatch_event(event: x11rb::protocol::Event) {
    match event {
        x11rb::protocol::Event::ButtonPress(e) => button_press(&e),
        x11rb::protocol::Event::ClientMessage(e) => client_message(&e),
        x11rb::protocol::Event::ConfigureNotify(e) => configure_notify(&e),
        x11rb::protocol::Event::ConfigureRequest(e) => configure_request(&e),
        x11rb::protocol::Event::CreateNotify(e) => create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => destroy_notify(&e),
        x11rb::protocol::Event::EnterNotify(e) => enter_notify(&e),
        x11rb::protocol::Event::Expose(e) => expose(&e),
        x11rb::protocol::Event::FocusIn(e) => focus_in(&e),
        x11rb::protocol::Event::KeyPress(e) => key_press(&e),
        x11rb::protocol::Event::KeyRelease(e) => key_release(&e),
        x11rb::protocol::Event::MappingNotify(e) => mapping_notify(&e),
        x11rb::protocol::Event::MapRequest(e) => map_request(&e),
        x11rb::protocol::Event::MotionNotify(e) => motion_notify(&e),
        x11rb::protocol::Event::PropertyNotify(e) => property_notify(&e),
        x11rb::protocol::Event::ResizeRequest(e) => resize_request(&e),
        x11rb::protocol::Event::UnmapNotify(e) => unmap_notify(&e),
        x11rb::protocol::Event::LeaveNotify(e) => leave_notify(&e),
        _ => {}
    }
}

pub fn scan() {
    let (root, wm_state_atom) = {
        let globals = get_globals();
        (globals.root, globals.wmatom.state)
    };

    let (managed, transients) = {
        let x11 = get_x11();
        let Some(ref conn) = x11.conn else {
            return;
        };

        let Ok(tree_cookie) = conn.query_tree(root) else {
            return;
        };
        let Ok(tree_reply) = tree_cookie.reply() else {
            return;
        };

        let mut managed = Vec::new();
        let mut transients = Vec::new();
        for win in tree_reply.children {
            let Ok(wa_cookie) = conn.get_window_attributes(win) else {
                continue;
            };
            let Ok(wa) = wa_cookie.reply() else {
                continue;
            };
            if wa.override_redirect {
                continue;
            }

            let is_transient = conn
                .get_property(
                    false,
                    win,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    0,
                    1,
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
                .is_some();

            let is_viewable = wa.map_state == MapState::VIEWABLE;
            let is_iconic = conn
                .get_property(false, win, wm_state_atom, wm_state_atom, 0, 2)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| reply.value32().and_then(|mut values| values.next()))
                .map(|state| state == crate::client::WM_STATE_ICONIC as u32)
                .unwrap_or(false);

            if !is_viewable && !is_iconic {
                continue;
            }
            if win_to_client(win).is_some() {
                continue;
            }

            let (x, y, width, height, border_width) = conn
                .get_geometry(win)
                .ok()
                .and_then(|geo| geo.reply().ok())
                .map(|geo| {
                    (
                        geo.x as i32,
                        geo.y as i32,
                        geo.width as u32,
                        geo.height as u32,
                        geo.border_width as u32,
                    )
                })
                .unwrap_or((0, 0, 800, 600, 1));
            let attrs = (win, x, y, width, height, border_width);
            if is_transient {
                transients.push(attrs);
            } else {
                managed.push(attrs);
            }
        }
        (managed, transients)
    };

    for (win, x, y, width, height, border_width) in managed {
        crate::client::manage(win, x, y, width, height, border_width);
    }
    for (win, x, y, width, height, border_width) in transients {
        crate::client::manage(win, x, y, width, height, border_width);
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
        let globals = get_globals_mut();
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
