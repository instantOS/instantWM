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
use crate::backend::BackendKind;
use crate::backend::BackendOps;
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
// focus() is used via focus_soft() in this module
use crate::globals::Globals;
use crate::layouts::arrange;
use crate::types::{Client, Rect, WindowId};
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
pub fn manage(ctx: &mut WmCtx, w: WindowId, wa_geo: Rect, wa_border_width: u32) {
    if !manage_preconditions_met(ctx) {
        return;
    }

    let trans = get_transient_for_hint_ctx(ctx, w);
    let mut client = build_initial_client(ctx, w, wa_geo, wa_border_width);
    assign_initial_monitor_and_tags(ctx, &mut client, trans);
    insert_client_and_apply_rules(ctx, w, client);

    let borderpx = apply_default_border(ctx, w);
    let (mon_work_rect, mon_monitor_rect) = monitor_rects_for_client(ctx, w);
    clamp_client_to_work_area(ctx, w, mon_work_rect);
    configure_client_border(
        ctx,
        w,
        borderpx,
        mon_monitor_rect,
        is_monocle_on_client_monitor(ctx, w),
    );

    apply_manage_hints(ctx, w);
    snapshot_float_geo(ctx, w, mon_monitor_rect);
    subscribe_manage_events(ctx, w);
    grab_buttons(ctx, w, false);

    if initialize_floating_state(ctx, w, trans.is_some()) {
        ctx.backend.raise_window(w);
        ctx.backend.flush();
    }

    attach(ctx, w);
    attach_stack(ctx, w);
    register_client_root(ctx, w);

    move_client_offscreen_before_arrange(ctx, w);
    let initially_hidden = prepare_visibility_and_unfocus(ctx, w);
    let animated = ctx.g.animated;
    let c = arrange_map_focus_and_snapshot(ctx, w, initially_hidden);

    run_manage_animation(ctx, w, &c, mon_monitor_rect, animated);
}

fn manage_preconditions_met(ctx: &WmCtx) -> bool {
    ctx.backend_kind() != BackendKind::Wayland && ctx.x11_conn().is_some()
}

fn build_initial_client(ctx: &WmCtx, w: WindowId, wa_geo: Rect, wa_border_width: u32) -> Client {
    let mut c = Client::default();
    c.win = w;
    c.geo = wa_geo;
    c.old_geo = c.geo;
    c.old_border_width = wa_border_width as i32;
    c.name = read_title_from_x(ctx, w);
    c
}

fn assign_initial_monitor_and_tags(ctx: &WmCtx, c: &mut Client, trans: Option<WindowId>) {
    let trans_client = trans.filter(|win| ctx.g.clients.contains(win));
    if let Some(tc_win) = trans_client {
        if let Some(tc) = ctx.g.clients.get(&tc_win) {
            c.mon_id = tc.mon_id;
            c.tags = tc.tags;
            return;
        }
    }
    c.mon_id = Some(ctx.g.selmon_id());
    c.tags = initial_tags_for_monitor(ctx.g, c.mon_id);
}

fn insert_client_and_apply_rules(ctx: &mut WmCtx, w: WindowId, mut c: Client) {
    c.is_hidden =
        crate::client::visibility::get_state(ctx, w) == crate::client::constants::WM_STATE_ICONIC;
    ctx.g.clients.insert(w, c);
    apply_rules(ctx, w);
}

fn apply_default_border(ctx: &mut WmCtx, w: WindowId) -> i32 {
    let borderpx = ctx.g.cfg.borderpx;
    if let Some(client) = ctx.g.clients.get_mut(&w) {
        client.border_width = borderpx;
        client.old_border_width = borderpx;
    }
    borderpx
}

fn monitor_rects_for_client(ctx: &WmCtx, w: WindowId) -> (Rect, Rect) {
    let mon_id = ctx.g.clients.get(&w).and_then(|c| c.mon_id);
    mon_id
        .and_then(|mid| ctx.g.monitor(mid))
        .map(|m| (m.work_rect, m.monitor_rect))
        .unwrap_or((Rect::default(), Rect::default()))
}

fn clamp_client_to_work_area(ctx: &mut WmCtx, w: WindowId, mon_work_rect: Rect) {
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

fn is_monocle_on_client_monitor(ctx: &WmCtx, w: WindowId) -> bool {
    let mon_id = ctx.g.clients.get(&w).and_then(|c| c.mon_id);
    mon_id
        .and_then(|mid| ctx.g.monitor(mid))
        .map(|mon| !mon.is_tiling_layout())
        .unwrap_or(false)
}

fn configure_client_border(
    ctx: &mut WmCtx,
    w: WindowId,
    borderpx: i32,
    mon_monitor_rect: Rect,
    is_monocle: bool,
) {
    let bh = ctx.g.cfg.bar_height;
    let (isfloating, client_width, client_height) = ctx
        .g
        .clients
        .get(&w)
        .map(|c| (c.isfloating, c.geo.w, c.geo.h))
        .unwrap_or((false, 0, 0));

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

    ctx.backend.set_border_width(w, border_width);

    let x11_win: Window = w.into();
    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        if let Some(ref scheme) = ctx.g.cfg.borderscheme {
            let pixel = scheme.normal.bg.pixel();
            let _ = conn.change_window_attributes(
                x11_win,
                &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
            );
        }
        let _ = conn.flush();
    }
}

fn apply_manage_hints(ctx: &mut WmCtx, w: WindowId) {
    crate::client::focus::configure(ctx, w);
    update_window_type(ctx, w);
    update_size_hints_win(ctx, w);
    update_wm_hints(ctx, w);
    read_client_info(ctx, w);
    set_client_tag_prop(ctx, w);
    update_motif_hints(ctx, w);
}

fn snapshot_float_geo(ctx: &mut WmCtx, w: WindowId, mon_monitor_rect: Rect) {
    if let Some(client) = ctx.g.clients.get_mut(&w) {
        client.float_geo.x = client.geo.x;
        client.float_geo.y = if client.geo.y >= mon_monitor_rect.y {
            client.geo.y
        } else {
            client.geo.y + mon_monitor_rect.y
        };
        client.float_geo.w = client.geo.w;
        client.float_geo.h = client.geo.h;
    }
}

fn subscribe_manage_events(ctx: &WmCtx, w: WindowId) {
    let mask = EventMask::ENTER_WINDOW
        | EventMask::FOCUS_CHANGE
        | EventMask::PROPERTY_CHANGE
        | EventMask::STRUCTURE_NOTIFY;
    let x11_win: Window = w.into();
    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        let _ = conn
            .change_window_attributes(x11_win, &ChangeWindowAttributesAux::new().event_mask(mask));
    }
}

fn initialize_floating_state(ctx: &mut WmCtx, w: WindowId, has_transient_parent: bool) -> bool {
    let isfixed = ctx.g.clients.get(&w).map(|c| c.isfixed).unwrap_or(false);
    let mut should_raise = false;
    if let Some(client) = ctx.g.clients.get_mut(&w) {
        if !client.isfloating {
            client.isfloating = has_transient_parent || isfixed;
            client.oldstate = client.isfloating as i32;
        }
        should_raise = client.isfloating;
    }
    should_raise
}

fn register_client_root(ctx: &mut WmCtx, w: WindowId) {
    let x11_win: Window = w.into();
    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        let _ = conn.change_property32(
            PropMode::APPEND,
            ctx.g.cfg.root,
            ctx.g.cfg.netatom.client_list,
            AtomEnum::WINDOW,
            &[x11_win],
        );
        let _ = conn.flush();
    }
}

fn move_client_offscreen_before_arrange(ctx: &mut WmCtx, w: WindowId) {
    let (screen_width, client_x, client_y, client_width, client_height) = ctx
        .g
        .clients
        .get(&w)
        .map(|client| {
            (
                ctx.g.cfg.screen_width,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
            )
        })
        .unwrap_or((0, 0, 0, 0, 0));

    ctx.backend.resize_window(
        w,
        Rect {
            x: client_x + 2 * screen_width,
            y: client_y,
            w: client_width,
            h: client_height,
        },
    );
    ctx.backend.flush();
}

fn prepare_visibility_and_unfocus(ctx: &mut WmCtx, w: WindowId) -> bool {
    let initially_hidden = ctx.g.clients.get(&w).map(|c| c.is_hidden).unwrap_or(false);
    if !initially_hidden {
        set_client_state(ctx, w, WM_STATE_NORMAL);
    }
    if let Some(sel_win) = ctx.g.selected_win() {
        unfocus_win(ctx, sel_win, false);
    }
    initially_hidden
}

fn arrange_map_focus_and_snapshot(ctx: &mut WmCtx, w: WindowId, initially_hidden: bool) -> Client {
    let mut c = ctx.g.clients.get(&w).cloned().unwrap_or_default();
    if let Some(mon_id) = c.mon_id {
        arrange(ctx, Some(mon_id));
    }
    if !initially_hidden {
        ctx.backend.map_window(w);
        ctx.backend.flush();
    }
    crate::focus::focus_soft(ctx, None);
    c = ctx.g.clients.get(&w).cloned().unwrap_or_default();
    c
}

fn run_manage_animation(
    ctx: &mut WmCtx,
    w: WindowId,
    c: &Client,
    mon_monitor_rect: Rect,
    animated: bool,
) {
    if !animated || c.is_fullscreen {
        return;
    }

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
    ctx.backend.flush();
    animate_client(
        ctx,
        w,
        &Rect {
            x: c.geo.x,
            y: c.geo.y,
            w: 0,
            h: 0,
        },
        7,
        0,
    );

    let is_tiling = c
        .mon_id
        .and_then(|mid| ctx.g.monitor(mid))
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(false);

    if !is_tiling {
        ctx.backend.raise_window(w);
        ctx.backend.flush();
    } else if c.geo.w > mon_monitor_rect.w - 30 || c.geo.h > mon_monitor_rect.h - 30 {
        if let Some(mon_id) = c.mon_id {
            arrange(ctx, Some(mon_id));
        }
    }
}

/// Initial tag mask for a newly managed client on `mon_id`.
///
/// This mirrors DWM semantics: a new client appears on all tags currently
/// visible on its target monitor.
pub fn initial_tags_for_monitor(g: &Globals, mon_id: Option<usize>) -> u32 {
    mon_id
        .and_then(|mid| g.monitor(mid))
        .map(|m| m.selected_tags())
        .filter(|tags| *tags != 0)
        .unwrap_or(1)
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
pub fn unmanage(ctx: &mut WmCtx, win: WindowId, destroyed: bool) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let mon_id = ctx.g.clients.get(&win).and_then(|c| c.mon_id);

    // Clear overlay and fullscreen references so those code paths don't hold
    // dangling window IDs after the client is gone.
    {
        for (_id, mon) in ctx.g.monitors_iter_mut() {
            if mon.overlay == Some(win) {
                mon.overlay = None;
            }
            if mon.fullscreen == Some(win) {
                mon.fullscreen = None;
            }
        }
    }

    detach(ctx, win);
    detach_stack(ctx, win);

    if !destroyed {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            ctx.g.clients.remove(&win);
            crate::focus::focus_soft(ctx, None);
            update_client_list(ctx);
            if let Some(mid) = mon_id {
                arrange(ctx, Some(mid));
            }
            return;
        };
        let x11_win: Window = win.into();
        let old_bw = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.old_border_width)
            .unwrap_or(0);

        {
            let _ = conn.grab_server();

            // Stop receiving events so we don't get confused during cleanup.
            let _ = conn.change_window_attributes(
                x11_win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );

            // Restore the original border width the application expects.
            // Use the backend abstraction so the call is backend-agnostic.
            ctx.backend.set_border_width(win, old_bw);

            // Release button grabs.
            let _ = conn.ungrab_button(ButtonIndex::from(0u8), x11_win, ModMask::from(0u16));
        }

        set_client_state(ctx, win, WM_STATE_WITHDRAWN);

        let _ = conn.ungrab_server();
        ctx.backend.flush();
    }

    // Remove from the global map.
    ctx.g.clients.remove(&win);

    crate::focus::focus_soft(ctx, None);
    update_client_list(ctx);

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
fn read_title_from_x(ctx: &WmCtx, win: WindowId) -> String {
    if ctx.backend_kind() == BackendKind::Wayland {
        return BROKEN.to_string();
    }
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return BROKEN.to_string();
    };
    let x11_win: Window = win.into();
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
            x11_win,
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

/// Read the `_NET_CLIENT_INFO` property from `w` and restore tags / monitor.
///
/// This is used to persist client state across WM restarts: when the WM starts
/// up it re-manages all existing windows, and this call recovers the tag
/// assignment and monitor that were set in the previous session.
fn read_client_info(ctx: &mut WmCtx, w: WindowId) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };
    let x11_win: Window = w.into();

    let client_info_atom = ctx.g.cfg.netatom.client_info;

    let Ok(cookie) = conn.get_property(false, x11_win, client_info_atom, AtomEnum::CARDINAL, 0, 2)
    else {
        return;
    };
    let Ok(reply) = cookie.reply() else { return };
    let Some(mut data) = reply.value32() else {
        return;
    };

    let tags = data.next().unwrap_or(0);
    let mon_num = data.next().unwrap_or(0);

    let target_mon = ctx
        .g
        .monitors_iter()
        .find(|(_i, m)| m.num as u32 == mon_num)
        .map(|(i, _)| i);

    if let Some(client) = ctx.g.clients.get_mut(&w) {
        client.tags = tags;
        if let Some(mid) = target_mon {
            client.mon_id = Some(mid);
        }
    }
}

fn get_transient_for_hint_ctx(ctx: &WmCtx, w: WindowId) -> Option<WindowId> {
    if ctx.backend_kind() == BackendKind::Wayland {
        return None;
    }
    let conn = ctx.x11_conn().map(|x11| x11.conn)?;
    let x11_win: Window = w.into();

    conn.get_property(
        false,
        x11_win,
        AtomEnum::WM_TRANSIENT_FOR,
        AtomEnum::WINDOW,
        0,
        1,
    )
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
    .map(WindowId::from)
}
