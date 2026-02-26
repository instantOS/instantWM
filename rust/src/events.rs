use crate::bar::{bar_position_at_x, bar_position_to_gesture};
use crate::bar::{draw_bar, draw_bars, reset_bar};
use crate::client::{
    configure, get_transient_for_hint, is_hidden, set_client_state, set_fullscreen, unmanage,
    update_title, update_wm_hints, win_to_client, WM_STATE_ICONIC, WM_STATE_WITHDRAWN,
};
use crate::commands::x_command;
use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module
use crate::keyboard::{
    grab_keys, key_press as keyboard_key_press, key_release as keyboard_key_release,
};
use crate::layouts::{arrange, restack};
use crate::monitor::{update_geom_ctx, win_to_mon_with_ctx};
use crate::mouse::{
    get_cursor_client_win, handle_floating_resize_hover, handle_sidebar_hover, hover_resize_mouse,
    reset_cursor, resize_mouse_directional,
};
use crate::systray;
use crate::tags::get_tag_width;
use crate::types::*;
use crate::util::clean_mask;
use crate::wm::Wm;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;

pub fn button_press(ctx: &mut WmCtx, e: &ButtonPressEvent) {
    // Client button grabs use GrabMode::SYNC; replay pointer events like dwm.
    let conn = ctx.x11.conn;
    let _ = conn.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
    let _ = conn.flush();

    let numlockmask = ctx.g.cfg.numlockmask;
    let buttons_clone = ctx.g.cfg.buttons.clone();
    let altcursor = ctx.g.altcursor;
    let mut selmon_id = ctx.g.selmon_id();
    let focusfollowsmouse = ctx.g.focusfollowsmouse;

    if let Some(clicked_mon) = win_to_mon_with_ctx(ctx, e.event) {
        if selmon_id != clicked_mon && (focusfollowsmouse || e.detail <= 3) {
            ctx.g.set_selmon(clicked_mon);
            selmon_id = clicked_mon;
            crate::focus::focus_soft(ctx, None);
        }
    };

    // Determine the full bar position — this carries the exact target
    // (tag index, window handle, etc.) through to the button action.
    let bar_pos: BarPosition;

    if let Some(win) = win_to_client(e.event) {
        bar_pos = BarPosition::ClientWin;
        // Only focus on button press if it's NOT a simple left/middle/right click
        // (e.g., for scroll wheel or other buttons). Simple clicks should not
        // change focus or raise windows - the user explicitly wants to interact
        // with the window without changing stacking order.
        // For focus-follows-mouse mode, we still focus since that's the expected behavior.
        if focusfollowsmouse && e.detail > 3 {
            crate::focus::focus_soft(ctx, Some(win));
            if let Some(mon_id) = ctx.g.clients.get(&win).and_then(|c| c.mon_id) {
                restack(ctx, mon_id);
            }
        }
    } else if let Some(mon) = ctx.g.monitor(selmon_id) {
        if e.event == mon.barwin {
            let position = bar_position_at_x(mon, ctx, e.event_x as i32);
            if position == BarPosition::StartMenu {
                reset_bar(ctx);
            }
            bar_pos = position;
        } else if (e.root_x as i32) > mon.monitor_rect.x + mon.monitor_rect.w - 50 {
            bar_pos = BarPosition::SideBar;
        } else {
            bar_pos = BarPosition::Root;
        }
    } else {
        bar_pos = BarPosition::Root;
    };

    if bar_pos == BarPosition::Root {
        if let Some(mon) = ctx.g.monitor(selmon_id) {
            if let Some(sel_win) = mon.sel {
                let is_floating = ctx
                    .g
                    .clients
                    .get(&sel_win)
                    .map(|c| c.isfloating)
                    .unwrap_or(false);
                let has_tiling = mon.is_tiling_layout();
                if altcursor == AltCursor::Resize && (is_floating || !has_tiling) {
                    let dir = ctx.g.resize_direction;
                    reset_cursor(ctx);
                    let btn = MouseButton::from_u8(e.detail).unwrap_or(MouseButton::Left);
                    if btn == MouseButton::Right {
                        crate::mouse::move_mouse(ctx, btn);
                    } else if btn == MouseButton::Left {
                        if dir == Some(crate::types::ResizeDirection::Top) {
                            crate::mouse::move_mouse(ctx, btn);
                        } else {
                            resize_mouse_directional(ctx, dir, btn);
                        }
                    }
                    return;
                }
            }
        }
    };

    let clean_state = clean_mask(e.state.into(), numlockmask);

    for button in &buttons_clone {
        if !button.matches(bar_pos) || button.button.as_u8() != e.detail {
            continue;
        }
        if clean_mask(button.mask, numlockmask) != clean_state {
            continue;
        }
        let arg = ButtonArg {
            pos: bar_pos,
            btn: button.button,
            rx: e.root_x as i32,
            ry: e.root_y as i32,
        };
        (button.action)(ctx, arg);
    }
}

pub fn client_message(ctx: &mut WmCtx, e: &ClientMessageEvent) {
    let showsystray = ctx.g.cfg.showsystray;
    let systray_win = ctx.g.systray.as_ref().map(|s| s.win).unwrap_or(0);
    let net_system_tray_op = ctx.g.cfg.netatom.system_tray_op;
    let net_wm_state = ctx.g.cfg.netatom.wm_state;
    let net_active_window = ctx.g.cfg.netatom.active_window;

    if showsystray && e.window == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(ctx, e);
        }
        return;
    };

    let Some(win) = win_to_client(e.window) else {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(ctx, e, win);
    } else if e.type_ == net_active_window {
        handle_active_window(ctx, win);
    };
}

pub fn configure_notify(ctx: &mut WmCtx, e: &ConfigureNotifyEvent) {
    if e.window != ctx.g.cfg.root {
        return;
    };

    ctx.g.cfg.screen_width = e.width as i32;
    ctx.g.cfg.screen_height = e.height as i32;

    update_geom_ctx(ctx);
    crate::focus::focus_soft(ctx, None);
    arrange(ctx, None);
}

pub fn configure_request(ctx: &mut WmCtx, e: &ConfigureRequestEvent) {
    if let Some(win) = win_to_client(e.window) {
        configure(ctx, win);
    } else {
        let conn = ctx.x11.conn;
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
    };
}

pub fn create_notify(_e: &CreateNotifyEvent) {}

pub fn destroy_notify(ctx: &mut WmCtx, e: &DestroyNotifyEvent) {
    if let Some(win) = win_to_client(e.window) {
        unmanage(ctx, win, true);
    } else if let Some(icon) = systray::win_to_systray_icon(ctx, e.window) {
        // Remove the icon from the systray list and client map, then resize
        // the bar and redraw the systray — matching the C code's sequence of
        // removesystrayicon(c) → resizebarwin(selmon) → updatesystray().
        systray::remove_systray_icon(ctx, icon);
        // Get monitor reference for resize_bar_win
        if let Some(mon) = ctx.g.selmon() {
            crate::bar::resize_bar_win(mon);
        }
        systray::update_systray(ctx);
    };
}

/// Handle EnterNotify events for focus-follows-mouse behavior.
///
/// This is the Rust equivalent of the C code's `enternotify` and `handle_floating_focus`.
/// The key insight is that when floating windows overlap, we must use `get_cursor_client_win`
/// (which calls XQueryPointer) to get the actual topmost window under the cursor,
/// rather than just using the event window which could be a hidden window below.
pub fn enter_notify(ctx: &mut WmCtx, e: &EnterNotifyEvent) {
    let focusfollowsmouse = ctx.g.focusfollowsmouse;
    let focusfollowsfloatmouse = ctx.g.focusfollowsfloatmouse;
    let entering_root = e.event == ctx.g.cfg.root;

    // 1. Filter out invalid crossing events (grab/ungrab, inferior notify)
    if (e.mode != NotifyMode::NORMAL || e.detail == NotifyDetail::INFERIOR) && !entering_root {
        return;
    }

    // 2. Snapshot selection state before any changes
    let selmon_id = ctx.g.selmon_id();
    let sel_win = ctx.g.selmon().and_then(|m| m.sel);
    let is_floating_sel = {
        let is_floating = sel_win
            .and_then(|w| ctx.g.clients.get(&w))
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = ctx.g.selmon().map(|m| m.is_tiling_layout()).unwrap_or(true);
        is_floating || !has_tiling
    };
    let entering_client = win_to_client(e.event);

    // 3. Handle floating focus (matches C handle_floating_focus)
    //    When the selected window is floating and we enter a different window
    //    (root or client), offer the resize cursor via hover_resize_mouse.
    if is_floating_sel {
        // Special case: transitioning from a floating selection to a tiled
        // client under the cursor should activate the resize offer on the
        // floating window until the user commits (clicks) or moves away.
        // This avoids the "nothing happens" feel when hovering onto a tiled
        // window while a floating window is selected.
        if crate::mouse::floating_to_tiled_hover(ctx) {
            return;
        }

        // Case 1: Entering root with floating sel
        if entering_root {
            if hover_resize_mouse(ctx) {
                return;
            }
            // Fall through to normal focus handling
        }
        // Case 2: Entering a different client while sel is floating
        else if let Some(ew) = entering_client {
            if Some(ew) != sel_win {
                let resized = hover_resize_mouse(ctx);
                if focusfollowsfloatmouse {
                    if resized {
                        return;
                    }
                    // Use the actual topmost window under cursor for focus
                    if let Some(newc) = get_cursor_client_win(ctx) {
                        if Some(newc) != sel_win {
                            crate::focus::focus_soft(ctx, Some(newc));
                        }
                    }
                }
                return;
            }
        }
    }

    // 4. Handle Monitor Switch
    if focusfollowsmouse {
        if let Some(new_mon_id) = win_to_mon_with_ctx(ctx, e.event) {
            if new_mon_id != selmon_id {
                ctx.g.set_selmon(new_mon_id);
                crate::focus::focus_soft(ctx, None);
                return;
            }
        }
    }

    // 5. Determine what's actually under the cursor
    let topmost_win_under_cursor = get_cursor_client_win(ctx);

    // 6. Handle focus switching based on configuration
    if let Some(hovered_win) = topmost_win_under_cursor {
        let hovered_is_floating = ctx
            .g
            .clients
            .get(&hovered_win)
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = ctx.g.selmon().map(|m| m.is_tiling_layout()).unwrap_or(true);

        // Skip floating focus if focusfollowsfloatmouse is disabled
        if !focusfollowsfloatmouse && hovered_is_floating && has_tiling && !entering_root {
            return;
        }

        if !focusfollowsmouse {
            return;
        }

        // Apply the focus change if different
        if ctx.g.selmon().map(|m| m.sel) != Some(Some(hovered_win)) {
            crate::focus::focus_soft(ctx, Some(hovered_win));
        }
    }
}

pub fn expose(ctx: &mut WmCtx, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    if let Some(mon_id) = win_to_mon_with_ctx(ctx, e.window) {
        let is_barwin = ctx
            .g
            .monitors
            .get(mon_id)
            .is_some_and(|m| e.window == m.barwin);
        if is_barwin {
            draw_bar(ctx, mon_id);
        }
    };
}

pub fn focus_in(ctx: &mut WmCtx, _e: &FocusInEvent) {
    if let Some(sel_win) = ctx.g.selected_win() {
        crate::client::set_focus(ctx, sel_win);
    };
}

pub fn key_press(ctx: &mut WmCtx, e: &KeyPressEvent) {
    keyboard_key_press(ctx, e);
}

pub fn key_release(ctx: &mut WmCtx, e: &KeyReleaseEvent) {
    keyboard_key_release(ctx, e);
}

pub fn mapping_notify(ctx: &mut WmCtx, _e: &MappingNotifyEvent) {
    grab_keys(ctx);
}

pub fn map_request(ctx: &mut WmCtx, e: &MapRequestEvent) {
    if let Some(_icon) = systray::win_to_systray_icon(ctx, e.window) {
        systray::update_systray(ctx);
        return;
    };

    if win_to_client(e.window).is_none() && !is_override_redirect(ctx, e.window) {
        let (geo, border_width) = get_win_geometry(ctx, e.window);
        crate::client::manage(ctx, e.window, geo, border_width);
    };
}

/// Handle mouse motion events for bar gesture detection and focus-follows-mouse.
pub fn motion_notify(ctx: &mut WmCtx, e: &MotionNotifyEvent) {
    if e.event != ctx.g.cfg.root {
        return;
    };

    let selmon_id = ctx.g.selmon_id();
    let tagwidth = get_tag_width(ctx);

    // Update cached tag width
    ctx.g.tags.width = tagwidth;

    let root_x = e.root_x as i32;
    let root_y = e.root_y as i32;

    // Handle focus-follows-mouse monitor switching
    if ctx.g.focusfollowsmouse {
        let rect = Rect {
            x: root_x,
            y: root_y,
            w: 1,
            h: 1,
        };
        if let Some(new_mon) =
            crate::types::find_monitor_by_rect(&ctx.g.monitors, &rect).or(Some(ctx.g.selmon_id()))
        {
            if new_mon != selmon_id {
                ctx.g.set_selmon(new_mon);
                crate::focus::focus_soft(ctx, None);
                return;
            }
        }
    };

    // Early-out: cursor is below the bar area.
    let (monitor_y, bar_height, current_gesture) = {
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        (mon.monitor_rect.y, ctx.g.cfg.bar_height, mon.gesture)
    };

    if root_y >= monitor_y + bar_height - 3 {
        if handle_floating_resize_hover(ctx, root_x, root_y, true) {
            return;
        }
        if handle_sidebar_hover(ctx, root_x, root_y) {
            return;
        }
        reset_bar(ctx);
        if ctx.g.altcursor == AltCursor::Sidebar {
            reset_cursor(ctx);
        }
        return;
    };

    // Compute the bar position from the cursor's monitor-local x coordinate,
    // then convert to a gesture for hover highlighting.
    let new_gesture = {
        let Some(mon) = ctx.g.selmon() else {
            return;
        };
        let local_x = root_x - mon.monitor_rect.x;
        let position = bar_position_at_x(mon, ctx, local_x);
        match position {
            // The status-text and root areas don't produce a hover gesture —
            // reset the bar and bail out so we don't light up anything.
            BarPosition::StatusText | BarPosition::Root => {
                reset_bar(ctx);
                return;
            }
            other => bar_position_to_gesture(other),
        }
    };

    if new_gesture != current_gesture {
        if let Some(mon) = ctx.g.selmon_mut() {
            mon.gesture = new_gesture;
        }
        draw_bar(ctx, selmon_id);
    };
}

pub fn property_notify(ctx: &mut WmCtx, e: &PropertyNotifyEvent) {
    if let Some(_icon) = systray::win_to_systray_icon(ctx, e.window) {
        systray::update_systray(ctx);
        return;
    };

    if e.window == ctx.g.cfg.root && e.atom == AtomEnum::WM_NAME.into() {
        if x_command(ctx) == 0 {
            crate::bar::x11::update_status(ctx);
        }
        return;
    };

    if let Some(win) = win_to_client(e.window) {
        match e.atom {
            x if x == AtomEnum::WM_NORMAL_HINTS.into() => {
                if let Some(c) = ctx.g.clients.get_mut(&win) {
                    c.hintsvalid = 0;
                }
            }
            x if x == AtomEnum::WM_HINTS.into() => {
                update_wm_hints(ctx, win);
                draw_bars(ctx);
            }
            _ => {}
        }

        let net_wm_name = ctx.g.cfg.netatom.wm_name;
        if e.atom == AtomEnum::WM_NAME.into() || e.atom == net_wm_name {
            update_title(ctx, win);
        }
    };
}

pub fn resize_request(ctx: &mut WmCtx, e: &ResizeRequestEvent) {
    if let Some(_icon) = systray::win_to_systray_icon(ctx, e.window) {
        systray::update_systray(ctx);
    };
}

pub fn unmap_notify(ctx: &mut WmCtx, e: &UnmapNotifyEvent) {
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
            set_client_state(ctx, win, WM_STATE_WITHDRAWN);
        } else {
            // Real unmap — window is going away; remove from management.
            // unmanage(win, false) already calls set_client_state(WITHDRAWN)
            // internally, so we must not call it here first.
            unmanage(ctx, win, false);
        }
    } else if let Some(_icon) = systray::win_to_systray_icon(ctx, e.window) {
        // Systray icons sometimes unmap without destroying; re-map them.
        systray::update_systray(ctx);
    };
}

pub fn leave_notify(ctx: &mut WmCtx, _e: &LeaveNotifyEvent) {
    reset_bar(ctx);
}

fn handle_systray_dock_request(ctx: &mut WmCtx, e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = data[2];
    if icon_win == 0 {
        return;
    };

    let selmon_id = ctx.g.selmon_id();
    let systray_win_opt = ctx.g.systray.as_ref().map(|s| s.win);
    let statusescheme_bg_pixel = ctx
        .g
        .cfg
        .statusscheme
        .as_ref()
        .map(|s| s.bg.color.pixel as u32)
        .unwrap_or(0);

    let Some(systray_win) = systray_win_opt else {
        return;
    };

    let conn = ctx.x11.conn;
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
        ctx.g.clients.insert(icon_win, client);
        if let Some(ref mut systray) = ctx.g.systray {
            systray.icons.insert(0, icon_win);
        }
    };

    crate::client::update_size_hints_win(ctx, icon_win);
    systray::update_systray_icon_geom(ctx, icon_win, geo.w, geo.h);

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

    let xembed_atom = ctx.g.cfg.xatom.xembed;
    let structure_notify_mask = EventMask::STRUCTURE_NOTIFY.bits();

    crate::client::send_event(
        ctx,
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
        ctx,
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
        ctx,
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
        ctx,
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

    if let Some(mon) = ctx.g.monitor(selmon_id) {
        crate::bar::resize_bar_win_ctx(ctx, mon);
    };

    systray::update_systray(ctx);
    set_client_state(ctx, icon_win, 1);
}

fn handle_net_wm_state(ctx: &mut WmCtx, e: &ClientMessageEvent, win: Window) {
    let data = e.data.as_data32();
    let fullscreen_action = data[0];

    if fullscreen_action == 1 {
        set_fullscreen(ctx, win, true);
    } else if fullscreen_action == 0 {
        set_fullscreen(ctx, win, false);
    };
}

fn handle_active_window(ctx: &mut WmCtx, win: Window) {
    let is_hidden = is_hidden(win);
    if is_hidden {
        crate::client::show(ctx, win);
    };

    if let Some(c) = ctx.g.clients.get(&win) {
        if let Some(mon_id) = c.mon_id {
            crate::focus::focus_soft(ctx, Some(win));
            restack(ctx, mon_id);
        }
    };
}

pub fn run(wm: &mut Wm) {
    while wm.running {
        let event = match wm.x11.conn.wait_for_event() {
            Ok(event) => event,
            Err(_) => return,
        };
        dispatch_event(wm, event);
    }
}

fn dispatch_event(wm: &mut Wm, event: x11rb::protocol::Event) {
    let mut ctx = wm.ctx();

    match event {
        x11rb::protocol::Event::ButtonPress(e) => button_press(&mut ctx, &e),
        x11rb::protocol::Event::ClientMessage(e) => client_message(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureNotify(e) => configure_notify(&mut ctx, &e),
        x11rb::protocol::Event::ConfigureRequest(e) => configure_request(&mut ctx, &e),
        x11rb::protocol::Event::CreateNotify(e) => create_notify(&e),
        x11rb::protocol::Event::DestroyNotify(e) => destroy_notify(&mut ctx, &e),
        x11rb::protocol::Event::EnterNotify(e) => enter_notify(&mut ctx, &e),
        x11rb::protocol::Event::Expose(e) => expose(&mut ctx, &e),
        x11rb::protocol::Event::FocusIn(e) => focus_in(&mut ctx, &e),
        x11rb::protocol::Event::KeyPress(e) => key_press(&mut ctx, &e),
        x11rb::protocol::Event::KeyRelease(e) => key_release(&mut ctx, &e),
        x11rb::protocol::Event::MappingNotify(e) => mapping_notify(&mut ctx, &e),
        x11rb::protocol::Event::MapRequest(e) => map_request(&mut ctx, &e),
        x11rb::protocol::Event::MotionNotify(e) => motion_notify(&mut ctx, &e),
        x11rb::protocol::Event::PropertyNotify(e) => property_notify(&mut ctx, &e),
        x11rb::protocol::Event::ResizeRequest(e) => resize_request(&mut ctx, &e),
        x11rb::protocol::Event::UnmapNotify(e) => unmap_notify(&mut ctx, &e),
        x11rb::protocol::Event::LeaveNotify(e) => leave_notify(&mut ctx, &e),
        _ => {}
    };
}

// ---------------------------------------------------------------------------
// scan helpers
// ---------------------------------------------------------------------------

/// Fetch the geometry and border width for `win`.
///
/// Returns a sensible fallback (`800×600`, border `1`) when the request fails,
/// so callers never have to handle `None`.
fn get_win_geometry(ctx: &WmCtx, win: Window) -> (Rect, u32) {
    let conn = ctx.x11.conn;
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
fn is_override_redirect(ctx: &WmCtx, win: Window) -> bool {
    let conn = ctx.x11.conn;
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
fn classify_windows(ctx: &WmCtx, children: Vec<Window>) -> (Vec<Window>, Vec<Window>) {
    let mut managed = Vec::new();
    let mut transients = Vec::new();

    let conn = ctx.x11.conn;

    for win in children {
        if is_override_redirect(ctx, win) {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| wa.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        let is_iconic = is_window_iconic(ctx, win);

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

fn is_window_iconic(ctx: &WmCtx, win: Window) -> bool {
    let conn = ctx.x11.conn;

    let state_atom = ctx.g.cfg.wmatom.state;
    let Ok(cookie) = conn.get_property(false, win, state_atom, state_atom, 0, 2) else {
        return false;
    };
    let Ok(reply) = cookie.reply() else {
        return false;
    };

    reply
        .value32()
        .and_then(|mut it| it.next())
        .map(|v| v as i32 == WM_STATE_ICONIC)
        .unwrap_or(false)
}

/// Adopt all pre-existing X11 windows at WM startup.
///
/// Regular windows are managed first; transients second — matching the order
/// the original C dwm uses so that transient windows end up above their owners
/// in the client list.
pub fn scan(wm: &mut Wm) {
    let mut ctx = wm.ctx();
    let conn = ctx.x11.conn;
    let root = ctx.g.cfg.root;

    let children = {
        let Ok(tree_cookie) = conn.query_tree(root) else {
            return;
        };
        let Ok(tree_reply) = tree_cookie.reply() else {
            return;
        };
        tree_reply.children
    };

    let (managed, transients) = classify_windows(&ctx, children);

    for win in managed.into_iter().chain(transients) {
        let (geo, border_width) = get_win_geometry(&ctx, win);
        crate::client::manage(&mut ctx, win, geo, border_width);
    }
}

pub fn check_other_wm(conn: &x11rb::rust_connection::RustConnection, root: Window) {
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
}

pub fn setup(_wm: &mut Wm) {
    // setup is performed by main during wm_init.
}

pub fn setup_root(wm: &mut Wm) {
    let conn = &wm.x11.conn;
    let root = wm.g.cfg.root;
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

    let mut ctx = wm.ctx();
    update_geom_ctx(&mut ctx);
}

pub fn cleanup(wm: &mut Wm) {
    let conn = &wm.x11.conn;

    let _ = conn.grab_server();

    for (_id, mon) in wm.g.monitors_iter() {
        let mut current = mon.clients;
        while let Some(win) = current {
            if let Some(c) = wm.g.clients.get(&win) {
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
