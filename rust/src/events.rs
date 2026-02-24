use crate::bar::{bar_position_at_x, BarPosition};
use crate::bar::{draw_bar, draw_bars, reset_bar};
use crate::client::visibility::get_state;
use crate::client::{
    configure, get_transient_for_hint, is_hidden, set_client_state, set_fullscreen, unmanage,
    update_title, update_wm_hints, win_to_client, WM_STATE_ICONIC, WM_STATE_WITHDRAWN,
};
use crate::commands::x_command;
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11, RUNNING};
use crate::keyboard::{
    grab_keys, key_press as keyboard_key_press, key_release as keyboard_key_release,
};
use crate::layouts::{arrange, restack};
use crate::monitor::{is_current_layout_tiling, rect_to_mon, update_geom, win_to_mon};
use crate::mouse::{
    find_floating_win_at_resize_border, get_cursor_client_win, handle_floating_resize_hover,
    handle_sidebar_hover, hover_resize_mouse, reset_cursor, resize_mouse_directional,
};
use crate::systray::{update_systray, win_to_systray_icon};
use crate::tags::get_tag_width;
use crate::types::*;
use crate::util::clean_mask;
use std::sync::atomic::Ordering;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;

pub fn button_press(e: &ButtonPressEvent) {
    // Client button grabs use GrabMode::SYNC; replay pointer events like dwm.
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
        let _ = conn.flush();
    }

    let globals = get_globals();
    let numlockmask = globals.numlockmask;
    let buttons = globals.buttons.as_slice();
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

    if let Some(win) = win_to_client(e.event) {
        click_target = Click::ClientWin;
        // Only focus on button press if it's NOT a simple left/middle/right click
        // (e.g., for scroll wheel or other buttons). Simple clicks should not
        // change focus or raise windows - the user explicitly wants to interact
        // with the window without changing stacking order.
        // For focus-follows-mouse mode, we still focus since that's the expected behavior.
        if focusfollowsmouse && e.detail > 3 {
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
                let position = bar_position_at_x(mon, globals, e.event_x as i32);
                if position == BarPosition::StartMenu {
                    reset_bar();
                }
                click_target = position.to_click();
            } else if (e.root_x as i32) > mon.monitor_rect.x + mon.monitor_rect.w - 50 {
                click_target = Click::SideBar;
            }
        }
    }

    if click_target == Click::RootWin {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            if let Some(sel_win) = mon.sel {
                let is_floating = globals
                    .clients
                    .get(&sel_win)
                    .map(|c| c.isfloating)
                    .unwrap_or(false);
                let has_tiling = crate::monitor::is_current_layout_tiling(mon, &globals.tags);
                if altcursor == AltCursor::Resize && (is_floating || !has_tiling) {
                    let dir = get_globals().resize_direction;
                    reset_cursor();
                    resize_mouse_directional(dir);
                    return;
                }
            }
        }
    }

    let clean_state = clean_mask(e.state.into(), numlockmask);

    for button in buttons {
        if button.click != click_target || button.button.as_u8() != e.detail {
            continue;
        }
        if clean_mask(button.mask, numlockmask) != clean_state {
            continue;
        }
        (button.action)();
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
    } else if let Some(icon) = win_to_systray_icon(e.window) {
        // Remove the icon from the systray list and client map, then resize
        // the bar and redraw the systray — matching the C code's sequence of
        // removesystrayicon(c) → resizebarwin(selmon) → updatesystray().
        crate::systray::remove_systray_icon(icon);
        {
            let globals = get_globals();
            let selmon_idx = globals.selmon;
            if let Some(mon) = globals.monitors.get(selmon_idx) {
                crate::bar::resize_bar_win(mon);
            }
        }
        update_systray();
    }
}

/// Handle EnterNotify events for focus-follows-mouse behavior.
///
/// This is the Rust equivalent of the C code's `enternotify` and `handle_floating_focus`.
/// The key insight is that when floating windows overlap, we must use `get_cursor_client_win`
/// (which calls XQueryPointer) to get the actual topmost window under the cursor,
/// rather than just using the event window which could be a hidden window below.
pub fn enter_notify(e: &EnterNotifyEvent) {
    let globals = get_globals();
    let root = globals.root;
    let focusfollowsmouse = globals.focusfollowsmouse;
    let focusfollowsfloatmouse = globals.focusfollowsfloatmouse;
    let entering_root = e.event == root;

    // Filter out invalid crossing events (grab/ungrab, inferior notify)
    if (e.mode != NotifyMode::NORMAL || e.detail == NotifyDetail::INFERIOR) && e.event != root {
        return;
    }

    // Get current selection info
    let (selmon_id, sel_win, is_floating_sel) = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        let sel_win = globals.monitors.get(selmon_id).and_then(|m| m.sel);
        let is_floating = sel_win
            .and_then(|w| globals.clients.get(&w))
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = globals
            .monitors
            .get(selmon_id)
            .map(|m| is_current_layout_tiling(m, &globals.tags))
            .unwrap_or(true);
        (selmon_id, sel_win, is_floating || !has_tiling)
    };

    // Handle entering root window with a floating selection: try resize mode
    if entering_root && is_floating_sel && hover_resize_mouse() {
        return;
    }
    // If hover_resize_mouse didn't grab, fall through to normal focus handling

    // Check if we're entering a client window
    let entering_client = win_to_client(e.event);

    // Handle floating window focus logic
    if let Some(client_win) = entering_client {
        let is_visible = get_globals()
            .clients
            .get(&client_win)
            .map(|c| c.is_visible())
            .unwrap_or(false);

        // If we have a floating selection and are entering a different client,
        // try hover_resize_mouse first (same as C code's handle_floating_focus)
        if sel_win != Some(client_win) && is_floating_sel && (entering_root || is_visible) {
            if hover_resize_mouse() {
                return;
            }

            // If hover_resize_mouse exited without action, focus the window under cursor
            if focusfollowsfloatmouse {
                if let Some(newc) = get_cursor_client_win() {
                    if newc != sel_win.unwrap_or(0) {
                        focus(Some(newc));
                    }
                }
                return;
            }
        }
    }

    // Skip focus changes for floating windows if focusfollowsfloatmouse is disabled
    // (unless we're entering root, the window is visible, or selection is sticky)
    if !focusfollowsfloatmouse {
        if let Some(client_win) = entering_client {
            let is_floating = get_globals()
                .clients
                .get(&client_win)
                .map(|c| c.isfloating)
                .unwrap_or(false);
            let has_tiling = get_globals()
                .monitors
                .get(selmon_id)
                .map(|m| is_current_layout_tiling(m, &get_globals().tags))
                .unwrap_or(true);

            // Skip focus for floating windows when not in floating layout
            if e.event != root && sel_win.is_some() && is_floating && has_tiling {
                return;
            }
        }
    }

    // Standard focus-follows-mouse behavior
    if !focusfollowsmouse {
        return;
    }

    // Handle monitor switching
    if let Some(new_mon_id) = win_to_mon(e.event) {
        if new_mon_id != selmon_id {
            {
                let globals = get_globals_mut();
                globals.selmon = new_mon_id;
            }
            focus(None);
            return;
        }
    }

    // Focus the window under the cursor, not just the event window
    // This ensures we focus the topmost window when windows overlap
    if entering_client.is_some() || entering_root {
        if let Some(win) = get_cursor_client_win() {
            let globals = get_globals();
            let sel_id = globals.selmon;
            if let Some(mon) = globals.monitors.get(sel_id) {
                if mon.sel != Some(win) {
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

    if win_to_client(e.window).is_none() && !is_override_redirect(e.window) {
        let (geo, border_width) = get_win_geometry(e.window);
        crate::client::manage(e.window, geo, border_width);
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
    if get_globals().focusfollowsmouse {
        if let Some(new_mon) = rect_to_mon(&Rect {
            x: root_x,
            y: root_y,
            w: 1,
            h: 1,
        }) {
            if new_mon != selmon_id {
                let globals = get_globals_mut();
                globals.selmon = new_mon;
                focus(None);
                return;
            }
        }
    }

    // Early-out: cursor is below the bar area.
    let (monitor_y, bar_height, current_gesture) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        (mon.monitor_rect.y, globals.bh, mon.gesture)
    };

    if root_y >= monitor_y + bar_height - 3 {
        if handle_floating_resize_hover() {
            return;
        }
        if handle_sidebar_hover(root_x, root_y) {
            return;
        }
        reset_bar();
        if get_globals().altcursor == AltCursor::Sidebar {
            reset_cursor();
        }
        return;
    }

    // Compute the bar position from the cursor's monitor-local x coordinate,
    // then convert to a gesture for hover highlighting.
    let new_gesture = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(selmon_id) else {
            return;
        };
        let local_x = root_x - mon.monitor_rect.x;
        let position = bar_position_at_x(mon, globals, local_x);
        match position {
            // The status-text and root areas don't produce a hover gesture —
            // reset the bar and bail out so we don't light up anything.
            BarPosition::StatusText | BarPosition::Root => {
                reset_bar();
                return;
            }
            other => other.to_gesture(),
        }
    };

    if new_gesture != current_gesture {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(selmon_id) {
            mon.gesture = new_gesture;
            draw_bar(mon);
        }
    }
}

pub fn property_notify(e: &PropertyNotifyEvent) {
    if let Some(_icon) = win_to_systray_icon(e.window) {
        update_systray();
        return;
    }

    let globals = get_globals();
    if e.window == globals.root && e.atom == AtomEnum::WM_NAME.into() {
        if x_command() == 0 {
            crate::bar::x11::update_status();
        }
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
        // Bit 7 of response_type is the X11 "send_event" flag.  When set the
        // UnmapNotify was generated synthetically (e.g. a client withdrawing
        // itself) rather than by a real XUnmapWindow call.  In that case we
        // only update WM_STATE to WithdrawnState and leave the client managed,
        // exactly as the original C code does:
        //
        //   if (ev->send_event) setclientstate(c, WithdrawnState);
        //   else                unmanage(c, 0);
        if e.response_type & 0x80 != 0 {
            // Synthetic unmap — client is withdrawing; just record state.
            set_client_state(win, WM_STATE_WITHDRAWN);
        } else {
            // Real unmap — window is going away; remove from management.
            // unmanage(win, false) already calls set_client_state(WITHDRAWN)
            // internally, so we must not call it here first.
            unmanage(win, false);
        }
    } else if let Some(_icon) = win_to_systray_icon(e.window) {
        // Systray icons sometimes unmap without destroying; re-map them.
        update_systray();
    }
}

pub fn leave_notify(e: &LeaveNotifyEvent) {
    reset_bar();

    let leaving_client = win_to_client(e.event);

    if let Some(leaving_win) = leaving_client {
        if let Some(hover_win) = find_floating_win_at_resize_border() {
            if hover_win == leaving_win {
                if get_globals()
                    .monitors
                    .get(get_globals().selmon)
                    .and_then(|m| m.sel)
                    != Some(hover_win)
                {
                    focus(Some(hover_win));
                }
                if hover_resize_mouse() {}
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

    let (selmon_id, systray_win_opt, statusescheme_bg_pixel) = {
        let globals = get_globals();
        let bg_pixel = globals
            .statusscheme
            .as_ref()
            .map(|s| s.bg.color.pixel as u32)
            .unwrap_or(0);
        (
            globals.selmon,
            globals.systray.as_ref().map(|s| s.win),
            bg_pixel,
        )
    };

    let Some(systray_win) = systray_win_opt else {
        return;
    };

    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return;
    };

    let (geo, border_width) = conn
        .get_geometry(icon_win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|wa| {
            (
                Rect {
                    x: 0,
                    y: 0,
                    w: wa.width as i32,
                    h: wa.height as i32,
                },
                wa.border_width as i32,
            )
        })
        .unwrap_or((
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            0,
        ));

    let client = Client {
        win: icon_win,
        geo,
        old_geo: geo,
        old_border_width: border_width,
        border_width: 0,
        isfloating: true,
        tags: 1,
        mon_id: Some(selmon_id),
        ..Default::default()
    };

    {
        let globals = get_globals_mut();
        globals.clients.insert(icon_win, client);
        if let Some(ref mut systray) = globals.systray {
            systray.icons.insert(0, icon_win);
        }
    }

    crate::client::update_size_hints_win(icon_win);
    crate::systray::update_systray_icon_geom(icon_win, geo.w, geo.h);

    let _ = conn.change_save_set(SetMode::INSERT, icon_win);

    let mask =
        EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE | EventMask::RESIZE_REDIRECT;
    let _ =
        conn.change_window_attributes(icon_win, &ChangeWindowAttributesAux::new().event_mask(mask));

    let _ = conn.reparent_window(icon_win, systray_win, 0, 0);

    let _ = conn.change_window_attributes(
        icon_win,
        &ChangeWindowAttributesAux::new().background_pixel(statusescheme_bg_pixel),
    );

    let xembed_atom = get_globals().xatom.xembed;
    let structure_notify_mask = EventMask::STRUCTURE_NOTIFY.bits();

    crate::client::send_event(
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_EMBEDDED_NOTIFY as i64,
        0,
        systray_win as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event(
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_FOCUS_IN as i64,
        0,
        systray_win as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event(
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_WINDOW_ACTIVATE as i64,
        0,
        systray_win as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    crate::client::send_event(
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        XEMBED_MODALITY_ON as i64,
        0,
        systray_win as i64,
        XEMBED_EMBEDDED_VERSION as i64,
    );

    let _ = conn.flush();

    {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            crate::bar::resize_bar_win(mon);
        }
    }

    update_systray();
    set_client_state(icon_win, 1);
}

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

// ---------------------------------------------------------------------------
// scan helpers
// ---------------------------------------------------------------------------

/// Fetch the geometry and border width for `win`.
///
/// Returns a sensible fallback (`800×600`, border `1`) when the request fails,
/// so callers never have to handle `None`.
fn get_win_geometry(win: Window) -> (Rect, u32) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return (
            Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            1,
        );
    };
    conn.get_geometry(win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|geo| {
            (
                Rect {
                    x: geo.x as i32,
                    y: geo.y as i32,
                    w: geo.width as i32,
                    h: geo.height as i32,
                },
                geo.border_width as u32,
            )
        })
        .unwrap_or((
            Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            1,
        ))
}

/// Returns `true` when the `override_redirect` attribute is set on `win`.
///
/// Such windows manage themselves (e.g. tooltips, menus) and must be ignored
/// by the WM.
fn is_override_redirect(win: Window) -> bool {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return false;
    };
    conn.get_window_attributes(win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|wa| wa.override_redirect)
        .unwrap_or(false)
}

/// Partition `children` into `(managed, transients)`.
///
/// A window is eligible for management when **all** of the following hold:
///
/// 1. Its `override_redirect` flag is **not** set.
/// 2. It is either viewable (`MapState::VIEWABLE`) or iconic (`WM_STATE_ICONIC`).
/// 3. It is not already tracked as a client.
///
/// Eligible windows whose `WM_TRANSIENT_FOR` hint names an owner go into
/// `transients`; all others go into `managed`.  The caller should manage the
/// `managed` slice first so that owner windows exist before their transients.
fn classify_windows(children: Vec<Window>) -> (Vec<Window>, Vec<Window>) {
    let mut managed = Vec::new();
    let mut transients = Vec::new();

    for win in children {
        // Skip self-managing windows.
        if is_override_redirect(win) {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = {
            let x11 = get_x11();
            x11.conn
                .as_ref()
                .and_then(|conn| conn.get_window_attributes(win).ok())
                .and_then(|cookie| cookie.reply().ok())
                .map(|wa| wa.map_state == MapState::VIEWABLE)
                .unwrap_or(false)
        };
        let is_iconic = get_state(win) == WM_STATE_ICONIC;

        if !is_viewable && !is_iconic {
            continue;
        }

        // Skip already-managed windows.
        if win_to_client(win).is_some() {
            continue;
        }

        if get_transient_for_hint(win).is_some() {
            transients.push(win);
        } else {
            managed.push(win);
        }
    }

    (managed, transients)
}

/// Adopt all pre-existing X11 windows at WM startup.
///
/// Regular windows are managed first; transients second — matching the order
/// the original C dwm uses so that transient windows end up above their owners
/// in the client list.
pub fn scan() {
    let root = get_globals().root;

    let children = {
        let x11 = get_x11();
        let Some(ref conn) = x11.conn else { return };
        let Ok(tree_cookie) = conn.query_tree(root) else {
            return;
        };
        let Ok(tree_reply) = tree_cookie.reply() else {
            return;
        };
        tree_reply.children
    };

    let (managed, transients) = classify_windows(children);

    for win in managed.into_iter().chain(transients) {
        let (geo, border_width) = get_win_geometry(win);
        crate::client::manage(win, geo, border_width);
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
