use crate::backend::x11::events::setup::XEMBED_EMBEDDED_NOTIFY;
use crate::backend::x11::events::setup::XEMBED_EMBEDDED_VERSION;
use crate::backend::x11::events::setup::XEMBED_FOCUS_IN;
use crate::backend::x11::events::setup::XEMBED_MODALITY_ON;
use crate::backend::x11::events::setup::XEMBED_WINDOW_ACTIVATE;
use crate::backend::x11::lifecycle::unmanage;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::{
    AltCursor, BarPosition, Client, Gesture, MouseButton, Rect, WindowId,
};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::get_win_geometry;
use super::is_override_redirect;
use super::setup::SYSTEM_TRAY_REQUEST_DOCK;

fn send_xembed_event(
    ctx: &mut WmCtxX11<'_>,
    icon_win: WindowId,
    systray_win: WindowId,
    msg: u32,
    a: i64,
    b: i64,
) {
    let xembed_atom = ctx.x11_runtime.xatom.xembed;
    let structure_notify_mask = EventMask::STRUCTURE_NOTIFY.bits();
    crate::client::send_event_x11(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        xembed_atom,
        structure_notify_mask,
        CURRENT_TIME as i64,
        msg as i64,
        a,
        u32::from(systray_win) as i64,
        b,
    );
}

pub fn button_press_x11(ctx: &mut WmCtxX11<'_>, e: &ButtonPressEvent) {
    let event_win = WindowId::from(e.event);
    let numlockmask = ctx.x11_runtime().numlockmask;
    let buttons_clone = ctx.core.globals().cfg.buttons.clone();
    let altcursor = ctx.core.globals().behavior.cursor_icon;
    let mut selmon_id = ctx.core.globals().selected_monitor_id();
    let focusfollowsmouse = ctx.core.globals().behavior.focus_follows_mouse;

    if let Some(clicked_mon) = ctx
        .core
        .globals()
        .monitors
        .find_monitor_for(event_win, ctx.core.globals().clients.map())
        && selmon_id != clicked_mon
        && (focusfollowsmouse || e.detail <= 3)
    {
        selmon_id = clicked_mon;
        crate::focus::select_monitor(&mut WmCtx::X11(ctx.reborrow()), clicked_mon);
    };

    // Determine the full bar position — this carries the exact target
    // (tag index, window handle, etc.) through to the button action.
    let bar_pos: BarPosition;

    if ctx.core.globals().clients.contains_key(&event_win) {
        bar_pos = BarPosition::ClientWin;
        // Only focus on button press if it's NOT a simple left/middle/right click
        // (e.g., for scroll wheel or other buttons). Simple clicks should not
        // change focus or raise windows - the user explicitly wants to interact
        // with the window without changing stacking order.
        // For focus-follows-mouse mode, we still focus since that's the expected behavior.
        if focusfollowsmouse && e.detail > 3 {
            crate::focus::focus_soft(&mut WmCtx::X11(ctx.reborrow()), Some(event_win));
            if let Some(monitor_id) = ctx.core.globals().clients.monitor_id(event_win) {
                crate::layouts::restack(&mut WmCtx::X11(ctx.reborrow()), monitor_id);
            }
        }
    } else if let Some(mon) = ctx.core.globals().monitor(selmon_id) {
        if event_win == mon.bar_win {
            let local_x = e.event_x as i32;
            let position = mon.bar_position_at_x(&ctx.core, local_x);

            if position == BarPosition::StatusText {
                let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                crate::bar::handle_status_text_click(
                    &mut wm_ctx,
                    e.root_x as i32,
                    e.root_y as i32,
                    e.detail,
                    crate::util::clean_mask(e.state.into(), numlockmask),
                );
                return;
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

    let clean_state = crate::util::clean_mask(e.state.into(), numlockmask);
    let client_binding_matched = bar_pos == BarPosition::ClientWin
        && buttons_clone.iter().any(|button| {
            button.matches(bar_pos)
                && button.button.as_u8() == e.detail
                && crate::util::clean_mask(button.mask, numlockmask) == clean_state
        });

    // Client button grabs use GrabMode::SYNC. Plain clicks should be replayed to
    // the client after WM processing, but WM-owned modified clicks (e.g. Super+drag)
    // must stay consumed by the WM so the initial press is not handed back to the
    // client before the drag grab begins.
    let conn = ctx.x11.conn;
    let _ = conn.allow_events(
        if client_binding_matched {
            Allow::ASYNC_POINTER
        } else {
            Allow::REPLAY_POINTER
        },
        CURRENT_TIME,
    );
    let _ = conn.flush();

    if bar_pos == BarPosition::Root
        && let Some(mon) = ctx.core.globals().monitor(selmon_id)
        && mon.sel.is_some()
        && let AltCursor::Resize(dir) = altcursor
    {
        crate::mouse::reset_cursor_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime);
        let btn = MouseButton::from_u8(e.detail).unwrap_or(MouseButton::Left);
        let mut x11_ctx = ctx.reborrow();
        if btn == MouseButton::Right {
            crate::backend::x11::mouse::move_mouse_x11(&mut x11_ctx, btn, None);
        } else if btn == MouseButton::Left {
            if dir == crate::types::ResizeDirection::Top {
                crate::backend::x11::mouse::move_mouse_x11(&mut x11_ctx, btn, None);
            } else {
                crate::mouse::resize_mouse_directional(&mut x11_ctx, Some(dir), btn);
            }
        }
        return;
    };

    if let Some(btn) = MouseButton::from_u8(e.detail) {
        let target_window = ctx.core.globals().clients.contains_key(&event_win).then_some(event_win);
        crate::bar::dispatch_configured_button(
            &mut WmCtx::X11(ctx.reborrow()),
            bar_pos,
            target_window,
            btn,
            e.root_x as i32,
            e.root_y as i32,
            clean_state,
            numlockmask,
        );
    }
}

//TODO: should this be called handle_client_message?
pub fn client_message(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let showsystray = ctx.core.globals().cfg.show_systray;
    let systray_win = ctx.systray.as_ref().map(|s| s.win).unwrap_or_default();
    let net_system_tray_op = ctx.x11_runtime.netatom.system_tray_op;
    let net_wm_state = ctx.x11_runtime.netatom.wm_state;
    let net_active_window = ctx.x11_runtime.netatom.active_window;
    let event_win = WindowId::from(e.window);

    if showsystray && event_win == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(ctx, e);
        }
        return;
    };

    if !ctx.core.globals().clients.contains_key(&event_win) {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(ctx, e, event_win);
    } else if e.type_ == net_active_window {
        handle_active_window(ctx, event_win);
    };
}

pub fn configure_notify(ctx: &mut WmCtxX11<'_>, e: &ConfigureNotifyEvent) {
    let event_win = WindowId::from(e.window);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        return;
    };

    ctx.core.globals_mut().cfg.screen_width = e.width as i32;
    ctx.core.globals_mut().cfg.screen_height = e.height as i32;

    crate::monitor::update_geom(&mut WmCtx::X11(ctx.reborrow()));
    crate::focus::focus_soft_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, None);
    crate::layouts::arrange(&mut WmCtx::X11(ctx.reborrow()), None);
}

pub fn configure_request(ctx: &mut WmCtxX11<'_>, e: &ConfigureRequestEvent) {
    let event_win = WindowId::from(e.window);
    if ctx.core.globals().clients.contains_key(&event_win) {
        crate::client::configure_x11(&mut ctx.core, &ctx.x11, event_win);
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

pub fn destroy_notify(ctx: &mut WmCtxX11<'_>, e: &DestroyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if ctx.core.globals().clients.contains_key(&event_win) {
        let mut tmp = ctx.reborrow();
        unmanage(&mut tmp, event_win, true);
    } else if let Some(icon) =
        crate::systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        // Remove the icon from the systray list and client map, then resize
        // the bar and redraw the systray — matching the C code's sequence of
        // removesystrayicon(c) → resizebar_win(selmon) → updatesystray().
        crate::systray::remove_systray_icon(&mut ctx.core, ctx.systray.as_deref_mut(), icon);
        // Get monitor reference for resize_bar_win
        let selmon_idx = ctx.core.globals().selected_monitor_id();
        if let Some(mon) = ctx.core.globals().monitor(selmon_idx).cloned() {
            crate::bar::resize_bar_win(
                &ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                ctx.systray.as_deref(),
                &mon,
            );
        }
        crate::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
    };
}

/// Handle EnterNotify events for focus-follows-mouse behavior.
///
/// This is the Rust equivalent of the C code's `enternotify` and `handle_floating_focus`.
/// The key insight is that when floating windows overlap, we must use `get_cursor_client_win`
/// (which calls XQueryPointer) to get the actual topmost window under the cursor,
/// rather than just using the event window which could be a hidden window below.
pub fn enter_notify(ctx: &mut WmCtxX11<'_>, e: &EnterNotifyEvent) {
    let focusfollowsfloatmouse = ctx.core.globals().behavior.focus_follows_float_mouse;
    let event_win = WindowId::from(e.event);
    let entering_root = event_win == WindowId::from(ctx.x11_runtime.root);

    // 1. Filter out invalid crossing events (grab/ungrab, inferior notify)
    if (e.mode != NotifyMode::NORMAL || e.detail == NotifyDetail::INFERIOR) && !entering_root {
        return;
    }

    // 2. Snapshot selection state before any changes
    let selected_monitor = ctx.core.globals().selected_monitor();
    let selected_window = selected_monitor.sel;
    let is_floating_sel = {
        let is_floating = selected_window
            .and_then(|w| ctx.core.client(w))
            .map(|c| c.is_floating)
            .unwrap_or(false);
        let has_tiling = selected_monitor.is_tiling_layout();
        is_floating || !has_tiling
    };
    let entering_client = ctx.core.globals().clients.contains_key(&event_win);

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
        // Case 1: Entering root while sel is floating
        if entering_root {
            if crate::mouse::hover_resize_mouse(ctx) {
                return;
            }
        // Case 2: Entering a different client while sel is floating
        } else if entering_client && Some(event_win) != selected_window {
            let resized = crate::mouse::hover_resize_mouse(ctx);
            if focusfollowsfloatmouse {
                if resized {
                    return;
                }
                if let Some(newc) = crate::backend::x11::mouse::get_cursor_client_win_with_conn(
                    &ctx.core,
                    ctx.x11.conn,
                    ctx.x11_runtime.root,
                ) && Some(newc) != selected_window
                {
                    crate::focus::focus_soft_x11(
                        &mut ctx.core,
                        &ctx.x11,
                        ctx.x11_runtime,
                        Some(newc),
                    );
                }
            }
            return;
        }
    }

    // 4. Determine what's actually under the cursor
    let topmost_win_under_cursor = crate::backend::x11::mouse::get_cursor_client_win_with_conn(
        &ctx.core,
        ctx.x11.conn,
        ctx.x11_runtime.root,
    );

    // 5. Handle focus switching based on shared policy.
    crate::focus::hover_focus_target(
        &mut WmCtx::X11(ctx.reborrow()),
        topmost_win_under_cursor,
        entering_root,
        Some((e.root_x as i32, e.root_y as i32)),
    );
}

pub fn expose(ctx: &mut WmCtxX11<'_>, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    let event_win = WindowId::from(e.window);
    if let Some(monitor_id) = ctx
        .core
        .globals()
        .monitors
        .find_monitor_for(event_win, ctx.core.globals().clients.map())
    {
        let is_bar_win = ctx
            .core
            .globals()
            .monitors
            .get(monitor_id)
            .is_some_and(|m| event_win == m.bar_win);
        if is_bar_win {
            crate::bar::x11::draw_bar(
                &mut ctx.core,
                ctx.x11_runtime,
                ctx.systray.as_deref(),
                monitor_id,
            );
        }
    };
}

pub fn focus_in(ctx: &mut WmCtxX11<'_>, _e: &FocusInEvent) {
    if let Some(selected_window) = ctx.core.selected_client() {
        crate::client::set_focus_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, selected_window);
    };
}

pub fn mapping_notify(ctx: &mut WmCtxX11<'_>, _e: &MappingNotifyEvent) {
    crate::keyboard::grab_keys_x11(&ctx.core, &ctx.x11, ctx.x11_runtime);
}

pub fn map_request(ctx: &mut WmCtxX11<'_>, e: &MapRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) =
        crate::systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        crate::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
        return;
    };

    if !ctx.core.globals().clients.contains_key(&event_win)
        && !is_override_redirect(&ctx.core, &ctx.x11, event_win)
    {
        let (geo, border_width) = get_win_geometry(&ctx.core, &ctx.x11, event_win);
        let mut tmp = ctx.reborrow();
        crate::backend::x11::lifecycle::manage(&mut tmp, event_win, geo, border_width);
    };
}

/// Handle mouse motion events for bar gesture detection and focus-follows-mouse.
pub fn motion_notify(ctx: &mut WmCtxX11<'_>, e: &MotionNotifyEvent) {
    let event_win = WindowId::from(e.event);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        let root_y = e.root_y as i32;
        let selmon = ctx.core.globals().selected_monitor();
        let in_bar = selmon.showbar
            && root_y >= selmon.bar_y
            && root_y < selmon.bar_y + ctx.core.globals().cfg.bar_height;
        if !in_bar && selmon.gesture != Gesture::None {
            crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
        }
        return;
    }

    let root_x = e.root_x as i32;
    let root_y = e.root_y as i32;

    // Handle focus-follows-mouse monitor switching
    if ctx.core.globals().behavior.focus_follows_mouse
        && crate::focus::select_monitor_at_pointer(&mut WmCtx::X11(ctx.reborrow()), (root_x, root_y))
    {
        return;
    }

    // Early-out: cursor is below the bar area.
    let (monitor_y, bar_height, current_gesture) = {
        let mon = ctx.core.globals().selected_monitor();
        (
            mon.monitor_rect.y,
            ctx.core.globals().cfg.bar_height,
            mon.gesture,
        )
    };

    if root_y >= monitor_y + bar_height {
        if crate::mouse::handle_floating_resize_hover(
            &mut WmCtx::X11(ctx.reborrow()),
            root_x,
            root_y,
            true,
        ) {
            return;
        }
        if crate::mouse::handle_sidebar_hover(&mut WmCtx::X11(ctx.reborrow()), root_x, root_y) {
            return;
        }
        crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
        if matches!(
            ctx.core.globals().behavior.cursor_icon,
            AltCursor::Resize(crate::types::ResizeDirection::Left)
        ) {
            crate::mouse::reset_cursor_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime);
        }
        return;
    };

    // Cache tag-strip width only when we are actually in the bar hot path.
    ctx.core.globals_mut().tags.width = crate::tags::get_tag_width(&ctx.core);

    let pos = crate::bar::update_hover(&mut WmCtx::X11(ctx.reborrow()), root_x, root_y, false, false);
    if matches!(pos, Some(BarPosition::StatusText | BarPosition::Root) | None)
        && current_gesture != Gesture::None
    {
        crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
    }
}

pub fn property_notify(ctx: &mut WmCtxX11<'_>, e: &PropertyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) =
        crate::systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        crate::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
        return;
    };

    if ctx.core.globals().clients.contains_key(&event_win) {
        match e.atom {
            x if x == u32::from(AtomEnum::WM_NORMAL_HINTS) => {
                if let Some(c) = ctx.core.globals_mut().clients.get_mut(&event_win) {
                    c.size_hints_valid = 0;
                }
            }
            x if x == u32::from(AtomEnum::WM_HINTS) => {
                crate::client::update_wm_hints(ctx, event_win);
                ctx.core.bar.mark_dirty();
            }
            _ => {}
        }

        let net_wm_name = ctx.x11_runtime.netatom.wm_name;
        if e.atom == u32::from(AtomEnum::WM_NAME) || e.atom == net_wm_name {
            crate::client::update_title_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, event_win);
        }
    };
}

pub fn resize_request(ctx: &mut WmCtxX11<'_>, e: &ResizeRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(_icon) =
        crate::systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        crate::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
    };
}

pub fn unmap_notify(ctx: &mut WmCtxX11<'_>, e: &UnmapNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if ctx.core.globals().clients.contains_key(&event_win) {
        if e.response_type & 0x80 != 0 {
            crate::client::set_client_state(
                &ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                event_win,
                crate::client::WM_STATE_WITHDRAWN,
            );
        } else {
            let mut tmp = ctx.reborrow();
            unmanage(&mut tmp, event_win, false);
        }
    } else if let Some(_icon) =
        crate::systray::win_to_systray_icon(&ctx.core, ctx.systray.as_deref(), event_win)
    {
        // Systray icons sometimes unmap without destroying; re-map them.
        crate::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref_mut(),
        );
    };
}

pub fn leave_notify(ctx: &mut WmCtxX11<'_>, _e: &LeaveNotifyEvent) {
    crate::bar::clear_hover(&mut WmCtx::X11(ctx.reborrow()));
}

fn handle_systray_dock_request(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = WindowId::from(data[2]);
    if icon_win == WindowId::default() {
        return;
    };

    let selmon_id = ctx.core.globals().selected_monitor_id();
    let systray_win_opt = ctx.systray.as_ref().map(|s| s.win);
    let statusescheme_bg_pixel = ctx.x11_runtime.statusscheme.bg.color.pixel as u32;

    let Some(systray_win) = systray_win_opt else {
        return;
    };

    let (geo, border_width) = {
        let conn = ctx.x11.conn;
        let x11_icon_win: Window = icon_win.into();
        conn.get_geometry(x11_icon_win)
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
            ))
    };

    let client = Client {
        win: icon_win,
        geo,
        old_geo: geo,
        old_border_width: border_width,
        border_width: 0,
        is_floating: true,
        tags: crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY),
        monitor_id: selmon_id,
        ..Default::default()
    };

    {
        ctx.core.globals_mut().clients.insert(icon_win, client);
        if let Some(ref mut systray) = ctx.systray {
            systray.icons.insert(0, icon_win);
        }
    };

    crate::backend::x11::update_size_hints_x11(&mut ctx.core, &ctx.x11, icon_win);
    crate::systray::update_systray_icon_geom(&mut ctx.core, &ctx.x11, icon_win, geo.w, geo.h);

    let conn = ctx.x11.conn;
    let x11_icon_win: Window = icon_win.into();
    let x11_systray_win: Window = systray_win.into();

    let _ = conn.change_save_set(SetMode::INSERT, x11_icon_win);

    let mask =
        EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE | EventMask::RESIZE_REDIRECT;
    let _ = conn.change_window_attributes(
        x11_icon_win,
        &ChangeWindowAttributesAux::new().event_mask(mask),
    );

    let _ = conn.reparent_window(x11_icon_win, x11_systray_win, 0, 0);

    let _ = conn.change_window_attributes(
        x11_icon_win,
        &ChangeWindowAttributesAux::new().background_pixel(statusescheme_bg_pixel),
    );

    let _ = conn.flush();

    send_xembed_event(
        ctx,
        icon_win,
        systray_win,
        XEMBED_EMBEDDED_NOTIFY,
        0,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    send_xembed_event(
        ctx,
        icon_win,
        systray_win,
        XEMBED_FOCUS_IN,
        0,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    send_xembed_event(
        ctx,
        icon_win,
        systray_win,
        XEMBED_WINDOW_ACTIVATE,
        0,
        XEMBED_EMBEDDED_VERSION as i64,
    );
    send_xembed_event(
        ctx,
        icon_win,
        systray_win,
        XEMBED_MODALITY_ON,
        0,
        XEMBED_EMBEDDED_VERSION as i64,
    );

    if let Some(mon) = ctx.core.globals().monitor(selmon_id).cloned() {
        crate::bar::resize_bar_win(
            &ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.systray.as_deref(),
            &mon,
        );
    };

    crate::systray::update_systray(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        ctx.systray.as_deref_mut(),
    );
    crate::client::set_client_state(&ctx.core, &ctx.x11, ctx.x11_runtime, icon_win, 1);
}

fn handle_net_wm_state(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent, win: WindowId) {
    let data = e.data.as_data32();
    let fullscreen_action = data[0];

    if fullscreen_action == 1 {
        crate::client::set_fullscreen_x11(ctx, win, true);
    } else if fullscreen_action == 0 {
        crate::client::set_fullscreen_x11(ctx, win, false);
    };
}

fn handle_active_window(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let is_hidden = ctx.core.globals().clients.is_hidden(win);
    if is_hidden {
        crate::client::show(&mut WmCtx::X11(ctx.reborrow()), win);
    };

    if let Some(c) = ctx.core.client(win) {
        let monitor_id = c.monitor_id;
        crate::focus::select_monitor_for_client(&mut WmCtx::X11(ctx.reborrow()), win);
        crate::focus::focus_soft(&mut WmCtx::X11(ctx.reborrow()), Some(win));
        crate::layouts::restack(&mut WmCtx::X11(ctx.reborrow()), monitor_id);
    };
}
