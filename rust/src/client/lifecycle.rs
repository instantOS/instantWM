//! Client lifecycle: adopting and releasing X11 windows.
//!
//! Note on title initialization: `update_title` writes into `globals.clients`,
//! so we cannot use it before the client is inserted.  Instead we call the
//! private `read_title_from_x` helper (which returns a `String`) and store the
//! result directly on the local `Client` before insertion.
//!
//! # The two entry points
//!
//! * [`manage`]   – called when the WM first sees a window (either at startup
//!                  via `QueryTree`, or at runtime via a `MapRequest` event).
//!                  Builds a [`Client`], attaches it to the correct monitor and
//!                  linked lists, applies rules/hints, and arranges the monitor.
//!
//! * [`unmanage`] – called when a window is destroyed or deliberately withdrawn.
//!                  Detaches it from every list, optionally restores X11 state
//!                  (border, event mask, WM_STATE), and re-focuses.
//!
//! # Monitor assignment
//!
//! A new window inherits its monitor from its transient-for parent when one
//! exists; otherwise it goes to the currently selected monitor.  After
//! [`crate::client::state::apply_rules`] runs, the assignment may be overridden
//! again by a matching rule.
//!
//! # Animation
//!
//! When the global `animated` flag is set, newly managed windows slide in from
//! 70 px above their final position.  Fullscreen windows skip the animation.

use crate::animation::animate_client;
use crate::client::constants::BROKEN;
use crate::client::constants::{WM_STATE_NORMAL, WM_STATE_WITHDRAWN};
use crate::client::focus::{grab_buttons, unfocus_win};
use crate::client::geometry::{client_height, client_width, resize_client, update_size_hints_win};
use crate::client::list::{attach, attach_stack, detach, detach_stack};
use crate::client::state::set_client_state;
use crate::client::state::{
    apply_rules, set_client_tag_prop, update_client_list, update_motif_hints, update_window_type,
    update_wm_hints,
};
use crate::contexts::WmCtx;
use crate::focus::focus;
use crate::globals::get_x11;
use crate::layouts::arrange;
use crate::types::{Client, Rect};
use std::cmp::max;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

// ---------------------------------------------------------------------------
// manage
// ---------------------------------------------------------------------------

/// Adopt `w` as a managed client window.
///
/// `wa_*` arguments come directly from the `GetWindowAttributesReply` /
/// `GetGeometryReply` of the window at the time the `MapRequest` arrives.
///
pub fn manage(ctx: &mut WmCtx, w: Window, wa_geo: Rect, wa_border_width: u32) {
    // -------------------------------------------------------------------------
    // 1. Build the initial Client struct.
    // -------------------------------------------------------------------------
    let mut c = Client::default();
    c.win = w;
    c.geo = wa_geo;
    c.old_geo = c.geo;
    c.old_border_width = wa_border_width as i32;
    // Read the window title before insertion so that apply_rules can match on
    // rule.title from the very first moment the client exists in the map.
    c.name = read_title_from_x(ctx, w);

    // -------------------------------------------------------------------------
    // 2. Assign the initial monitor (from transient parent or selmon).
    // -------------------------------------------------------------------------
    let trans = get_transient_for_hint_ctx(ctx, w);

    {
        let trans_client = trans.and_then(|win| {
            if ctx.g.clients.contains_key(&win) {
                Some(win)
            } else {
                None
            }
        });

        if let Some(tc_win) = trans_client {
            if let Some(tc) = ctx.g.clients.get(&tc_win) {
                c.mon_id = tc.mon_id;
                c.tags = tc.tags;
            }
        } else {
            c.mon_id = Some(ctx.g.selmon);
        }
    }

    // -------------------------------------------------------------------------
    // 3. Insert into the global client map and run rule matching.
    // -------------------------------------------------------------------------

    // Seed the cached is_hidden flag from the live WM_STATE property.
    // This handles windows that were already in the iconic state before
    // the WM started (e.g. restored from a previous session).
    c.is_hidden =
        crate::client::visibility::get_state(w) == crate::client::constants::WM_STATE_ICONIC;

    ctx.g.clients.insert(w, c.clone());

    apply_rules(w);

    // -------------------------------------------------------------------------
    // 4. Apply the global default border width.
    // -------------------------------------------------------------------------
    let borderpx = ctx.g.cfg.borderpx;
    if let Some(client) = ctx.g.clients.get_mut(&w) {
        client.border_width = borderpx;
    }

    // -------------------------------------------------------------------------
    // 5. Clamp the initial position to the monitor work-area.
    // -------------------------------------------------------------------------
    let (_mon_showbar, mon_work_rect, mon_monitor_rect) = {
        let mon_id = ctx.g.clients.get(&w).and_then(|c| c.mon_id);
        mon_id
            .and_then(|mid| ctx.g.monitors.get(mid))
            .map(|m| (m.showbar, m.work_rect, m.monitor_rect))
            .unwrap_or((false, Rect::default(), Rect::default()))
    };

    {
        if let Some(client) = ctx.g.clients.get_mut(&w) {
            if client.geo.x + client_width(client) > mon_work_rect.x + mon_work_rect.w {
                client.geo.x = mon_work_rect.x + mon_work_rect.w - client_width(client);
            }
            if client.geo.y + client_height(client) > mon_work_rect.y + mon_work_rect.h {
                client.geo.y = mon_work_rect.y + mon_work_rect.h - client_height(client);
            }
            client.geo.x = max(client.geo.x, mon_work_rect.x);
            client.geo.y = max(client.geo.y, mon_work_rect.y);
        }
    }

    // -------------------------------------------------------------------------
    // 6. Configure the X11 window: border width and border colour.
    // -------------------------------------------------------------------------
    let is_monocle = {
        let mon_id = ctx.g.clients.get(&w).and_then(|c| c.mon_id);
        mon_id
            .and_then(|mid| ctx.g.monitors.get(mid))
            .map(|mon| !mon.is_tiling_layout())
            .unwrap_or(false)
    };

    let bh = ctx.g.cfg.bh;
    let conn = ctx.x11.conn;

    {
        let (isfloating, client_width, client_height) = ctx
            .g
            .clients
            .get(&w)
            .map(|c| (c.isfloating, c.geo.w, c.geo.h))
            .unwrap_or((false, 0, 0));

        // In monocle mode, borderless windows that fill the monitor get no
        // border even if borderpx > 0.
        let border_width = if !isfloating
            && is_monocle
            && client_width > mon_monitor_rect.w - 30
            && client_height > mon_monitor_rect.h - 30 - bh
        {
            0
        } else {
            borderpx
        };

        if let Some(client) = ctx.g.clients.get_mut(&w) {
            client.border_width = border_width;
        }

        let _ = conn.configure_window(
            w,
            &ConfigureWindowAux::new().border_width(border_width as u32),
        );

        if let Some(ref scheme) = ctx.g.cfg.borderscheme {
            let pixel = scheme.normal.bg.pixel();
            let _ = conn.change_window_attributes(
                w,
                &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
            );
        }
        let _ = conn.flush();
    }

    // -------------------------------------------------------------------------
    // 7. Read and apply all X11 hints/properties.
    // -------------------------------------------------------------------------
    crate::client::focus::configure(ctx, w);
    update_window_type(ctx, w);
    update_size_hints_win(ctx, w);
    update_wm_hints(ctx, w);
    read_client_info(ctx, w);
    set_client_tag_prop(w);
    update_motif_hints(ctx, w);

    // -------------------------------------------------------------------------
    // 8. Store the floating geometry snapshot.
    // -------------------------------------------------------------------------
    {
        if let Some(client) = ctx.g.clients.get_mut(&w) {
            client.float_geo.x = client.geo.x;
            // When the window's y is above the monitor origin (can happen with
            // multi-monitor setups), offset by the monitor's y.
            client.float_geo.y = if client.geo.y >= mon_monitor_rect.y {
                client.geo.y
            } else {
                client.geo.y + mon_monitor_rect.y
            };
            client.float_geo.w = client.geo.w;
            client.float_geo.h = client.geo.h;
        }
    }

    // -------------------------------------------------------------------------
    // 9. Subscribe to the events we care about on this window.
    // -------------------------------------------------------------------------
    {
        let mask = EventMask::ENTER_WINDOW
            | EventMask::FOCUS_CHANGE
            | EventMask::PROPERTY_CHANGE
            | EventMask::STRUCTURE_NOTIFY;
        let _ =
            conn.change_window_attributes(w, &ChangeWindowAttributesAux::new().event_mask(mask));
    }

    grab_buttons(ctx, w, false);

    // -------------------------------------------------------------------------
    // 10. Determine the initial floating state.
    // -------------------------------------------------------------------------
    let isfixed = ctx.g.clients.get(&w).map(|c| c.isfixed).unwrap_or(false);

    let mut should_raise = false;
    {
        if let Some(client) = ctx.g.clients.get_mut(&w) {
            if !client.isfloating {
                client.isfloating = trans.is_some() || isfixed;
                client.oldstate = client.isfloating as i32;
            }
            should_raise = client.isfloating;
        }
    }

    // Floating windows start on top.
    if should_raise {
        let _ = conn.configure_window(w, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    // -------------------------------------------------------------------------
    // 11. Insert into both linked lists and register with the root.
    // -------------------------------------------------------------------------
    attach(w);
    attach_stack(w);

    {
        let _ = conn.change_property32(
            PropMode::APPEND,
            ctx.g.cfg.root,
            ctx.g.cfg.netatom.client_list,
            AtomEnum::WINDOW,
            &[w],
        );
        let _ = conn.flush();
    }

    // -------------------------------------------------------------------------
    // 12. Move the window off-screen initially, then arrange + map it.
    //     (Avoids a visible "flash" at (0,0) before the layout runs.)
    // -------------------------------------------------------------------------
    let (screen_width, client_x, client_y, client_width, client_height) = {
        ctx.g
            .clients
            .get(&w)
            .map(|client| {
                (
                    ctx.g.cfg.sw,
                    client.geo.x,
                    client.geo.y,
                    client.geo.w,
                    client.geo.h,
                )
            })
            .unwrap_or((0, 0, 0, 0, 0))
    };

    {
        let _ = conn.configure_window(
            w,
            &ConfigureWindowAux::new()
                .x(client_x + 2 * screen_width)
                .y(client_y)
                .width(client_width as u32)
                .height(client_height as u32),
        );
        let _ = conn.flush();
    }

    let initially_hidden = ctx.g.clients.get(&w).map(|c| c.is_hidden).unwrap_or(false);

    if !initially_hidden {
        set_client_state(w, WM_STATE_NORMAL);
    }

    // Unfocus the previously selected window before reassigning sel.
    let sel_win = ctx.g.monitors.get(ctx.g.selmon).and_then(|mon| mon.sel);

    if let Some(sel_win) = sel_win {
        unfocus_win(ctx, sel_win, false);
    }

    // Re-snapshot c from the map (apply_rules / hints may have changed it).
    let c = ctx.g.clients.get(&w).cloned().unwrap_or_default();

    let animated = ctx.g.animated;
    if let Some(mon_id) = c.mon_id {
        if let Some(mon) = ctx.g.monitors.get_mut(mon_id) {
            mon.sel = Some(w);
        }
    }

    if let Some(mon_id) = c.mon_id {
        arrange(ctx, Some(mon_id));
    }

    if !initially_hidden {
        let _ = conn.map_window(w);
        let _ = conn.flush();
    }

    focus(ctx, None);

    // -------------------------------------------------------------------------
    // 13. Entrance animation.
    // -------------------------------------------------------------------------
    if animated && !c.is_fullscreen {
        // Place the window 70 px above its target so the animation slides it down.
        resize_client(
            ctx,
            w,
            &Rect {
                x: c.geo.x,
                y: c.geo.y - 70,
                w: c.geo.w,
                h: c.geo.h,
            },
        );
        animate_client(
            ctx,
            w,
            &Rect {
                x: c.geo.x,
                y: c.geo.y + 70,
                w: 0,
                h: 0,
            },
            7,
            0,
        );

        let is_tiling = c
            .mon_id
            .and_then(|mid| ctx.g.monitors.get(mid))
            .map(|mon| mon.is_tiling_layout())
            .unwrap_or(false);

        if !is_tiling {
            let _ =
                conn.configure_window(w, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
            let _ = conn.flush();
        } else if c.geo.w > mon_monitor_rect.w - 30 || c.geo.h > mon_monitor_rect.h - 30 {
            if let Some(mon_id) = c.mon_id {
                arrange(ctx, Some(mon_id));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// unmanage
// ---------------------------------------------------------------------------

/// Release `win` from WM management.
///
/// `destroyed` should be `true` when this is called in response to a
/// `DestroyNotify` event (the X server has already destroyed the window; any
/// attempt to configure it will fail).  When `false` (e.g. a `UnmapNotify`
/// from a deliberately withdrawn window) we restore the border width and clear
/// the event mask / WM_STATE.
///
pub fn unmanage(ctx: &mut WmCtx, win: Window, destroyed: bool) {
    let mon_id = ctx.g.clients.get(&win).and_then(|c| c.mon_id);

    // Clear overlay and fullscreen references so those code paths don't hold
    // dangling window IDs after the client is gone.
    {
        for mon in &mut ctx.g.monitors {
            if mon.overlay == Some(win) {
                mon.overlay = None;
            }
            if mon.fullscreen == Some(win) {
                mon.fullscreen = None;
            }
        }
    }

    detach(win);
    detach_stack(win);

    if !destroyed {
        let conn = ctx.x11.conn;
        let old_bw = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.old_border_width)
            .unwrap_or(0);

        let _ = conn.grab_server();

        // Stop receiving events so we don't get confused during cleanup.
        let _ = conn.change_window_attributes(
            win,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
        );

        // Restore the original border width the application expects.
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().border_width(old_bw as u32));

        // Release button grabs.
        let _ = conn.ungrab_button(ButtonIndex::from(0u8), win, ModMask::from(0u16));

        set_client_state(win, WM_STATE_WITHDRAWN);

        let _ = conn.flush();
        let _ = conn.ungrab_server();
    }

    // Remove from the global map.
    ctx.g.clients.remove(&win);

    focus(ctx, None);
    update_client_list();

    if let Some(mid) = mon_id {
        arrange(ctx, Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the window title string directly from the X server without going
/// through the global client map.  Used during [`manage`] before the new
/// [`Client`] has been inserted.
///
/// Prefers `_NET_WM_NAME` (UTF-8) over the legacy `WM_NAME` property.
fn read_title_from_x(ctx: &WmCtx, win: Window) -> String {
    let conn = ctx.x11.conn;
    let net_wm_name = ctx.g.cfg.netatom.wm_name;

    for atom in [
        net_wm_name,
        x11rb::protocol::xproto::AtomEnum::WM_NAME.into(),
    ] {
        if atom == 0 {
            continue;
        }
        let Ok(cookie) = conn.get_property(
            false,
            win,
            atom,
            x11rb::protocol::xproto::AtomEnum::ANY,
            0,
            1024,
        ) else {
            continue;
        };
        let Ok(reply) = cookie.reply() else { continue };

        if reply.format != 8 || reply.value.is_empty() {
            continue;
        }

        let len = reply
            .value
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(reply.value.len());

        let title = String::from_utf8_lossy(&reply.value[..len]).into_owned();
        if !title.is_empty() {
            return title;
        }
    }

    BROKEN.to_string()
}

/// Get the `WM_TRANSIENT_FOR` hint for `w`.
///
/// Returns the parent window ID, or `None` when the property is absent or
/// the window is not a transient.
pub fn get_transient_for_hint(w: Window) -> Option<Window> {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else {
        return None;
    };

    conn.get_property(false, w, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
}

/// Read the `_NET_CLIENT_INFO` property from `w` and restore tags / monitor.
///
/// This is used to persist client state across WM restarts: when the WM starts
/// up it re-manages all existing windows, and this call recovers the tag
/// assignment and monitor that were set in the previous session.
fn read_client_info(ctx: &mut WmCtx, w: Window) {
    let conn = ctx.x11.conn;

    let client_info_atom = ctx.g.cfg.netatom.client_info;

    let Ok(cookie) = conn.get_property(false, w, client_info_atom, AtomEnum::CARDINAL, 0, 2) else {
        return;
    };
    let Ok(reply) = cookie.reply() else { return };
    let Some(mut data) = reply.value32() else {
        return;
    };

    let tags = data.next().unwrap_or(0);
    let mon_num = data.next().unwrap_or(0);

    let target_mon = ctx.g.monitors.iter().position(|m| m.num as u32 == mon_num);

    if let Some(client) = ctx.g.clients.get_mut(&w) {
        client.tags = tags;
        if let Some(mid) = target_mon {
            client.mon_id = Some(mid);
        }
    }
}

fn get_transient_for_hint_ctx(ctx: &WmCtx, w: Window) -> Option<Window> {
    let conn = ctx.x11.conn;

    conn.get_property(false, w, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
}
