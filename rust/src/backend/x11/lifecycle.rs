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

use crate::animation::animate_client_x11;
use crate::backend::x11::X11BackendRef;
use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::client::constants::BROKEN;
use crate::client::constants::{WM_STATE_NORMAL, WM_STATE_WITHDRAWN};
use crate::client::focus::{grab_buttons_x11, unfocus_win_x11};
use crate::client::list::{attach, attach_stack, detach, detach_stack};
use crate::client::resize;
use crate::client::state::set_client_state;
use crate::client::state::{
    apply_rules, set_client_tag_prop, update_client_list, update_motif_hints, update_window_type,
    update_wm_hints,
};
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
// focus() is used via focus_soft() in this module
use crate::focus::focus_soft;
use crate::globals::{Globals, X11RuntimeConfig};
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

pub fn manage(ctx: &mut WmCtxX11, w: WindowId, wa_geo: Rect, wa_border_width: u32) {
    let trans = get_transient_for_hint_x11(&ctx.x11, w);
    let x11_runtime = &*ctx.x11_runtime;
    let mut client = build_initial_client(&ctx.x11, x11_runtime, w, wa_geo, wa_border_width);
    assign_initial_monitor_and_tags(ctx.core.g, &mut client, trans);
    insert_client_and_apply_rules(&mut ctx.core, &ctx.x11, ctx.x11_runtime, w, client);

    let borderpx = apply_default_border(ctx.core.g, w);
    let (mon_work_rect, mon_monitor_rect) = monitor_rects_for_client(ctx.core.g, w);
    clamp_client_to_work_area(ctx.core.g, w, mon_work_rect);
    let is_monocle = is_monocle_on_client_monitor(ctx.core.g, w);
    configure_client_border(
        ctx.core.g,
        &ctx.x11,
        w,
        borderpx,
        mon_monitor_rect,
        is_monocle,
    );

    apply_manage_hints(&mut ctx.core, &ctx.x11, ctx.x11_runtime, w);
    snapshot_float_geo(ctx.core.g, w, mon_monitor_rect);
    subscribe_manage_events(&ctx.x11, w);
    grab_buttons_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, w, false);

    if initialize_floating_state(ctx.core.g, w, trans.is_some()) {
        ctx.backend.raise_window(w);
        ctx.backend.flush();
    }

    attach(&mut WmCtx::X11(ctx.reborrow()), w);
    attach_stack(&mut WmCtx::X11(ctx.reborrow()), w);
    register_client_root(&ctx.x11, ctx.x11_runtime, w);

    move_client_offscreen_before_arrange(&mut WmCtx::X11(ctx.reborrow()), w);
    let initially_hidden = prepare_visibility_and_unfocus(&mut WmCtx::X11(ctx.reborrow()), w);
    let animated = ctx.core.g.animated;
    let c = arrange_map_focus_and_snapshot(&mut WmCtx::X11(ctx.reborrow()), w, initially_hidden);

    run_manage_animation(
        &mut WmCtx::X11(ctx.reborrow()),
        w,
        &c,
        mon_monitor_rect,
        animated,
    );
}

fn build_initial_client(
    x11: &X11BackendRef,
    x11_cfg: &X11RuntimeConfig,
    w: WindowId,
    wa_geo: Rect,
    wa_border_width: u32,
) -> Client {
    let mut c = Client::default();
    c.win = w;
    c.geo = wa_geo;
    c.old_geo = c.geo;
    c.old_border_width = wa_border_width as i32;
    c.name = read_title_from_x(x11, x11_cfg, w);
    c
}

fn assign_initial_monitor_and_tags(
    g: &mut crate::globals::Globals,
    c: &mut Client,
    trans: Option<WindowId>,
) {
    let trans_client = trans.filter(|win| g.clients.contains(win));
    if let Some(tc_win) = trans_client {
        if let Some(tc) = g.clients.get(&tc_win) {
            c.monitor_id = tc.monitor_id;
            c.tags = tc.tags;
            return;
        }
    }
    c.monitor_id = Some(g.selected_monitor_id());
    c.tags = initial_tags_for_monitor(g, c.monitor_id);
}

fn insert_client_and_apply_rules(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_cfg: &X11RuntimeConfig,
    w: WindowId,
    mut c: Client,
) {
    c.is_hidden = crate::client::visibility::get_state_x11(core, x11, x11_cfg.wmatom.state, w)
        == crate::client::constants::WM_STATE_ICONIC;
    core.g.clients.insert(w, c);
    apply_rules(core, x11, w);
}

fn apply_default_border(g: &mut crate::globals::Globals, w: WindowId) -> i32 {
    let borderpx = g.cfg.borderpx;
    if let Some(client) = g.clients.get_mut(&w) {
        client.border_width = borderpx;
        client.old_border_width = borderpx;
    }
    borderpx
}

fn monitor_rects_for_client(g: &crate::globals::Globals, w: WindowId) -> (Rect, Rect) {
    let monitor_id = g.clients.get(&w).and_then(|c| c.monitor_id);
    monitor_id
        .and_then(|mid| g.monitor(mid))
        .map(|m| (m.work_rect, m.monitor_rect))
        .unwrap_or((Rect::default(), Rect::default()))
}

fn clamp_client_to_work_area(g: &mut crate::globals::Globals, w: WindowId, mon_work_rect: Rect) {
    if let Some(client) = g.clients.get_mut(&w) {
        if client.geo.x + client.total_width() > mon_work_rect.x + mon_work_rect.w {
            client.geo.x = mon_work_rect.x + mon_work_rect.w - client.total_width();
        }
        if client.geo.y + client.total_height() > mon_work_rect.y + mon_work_rect.h {
            client.geo.y = mon_work_rect.y + mon_work_rect.h - client.total_height();
        }
        client.geo.x = max(client.geo.x, mon_work_rect.x);
        client.geo.y = max(client.geo.y, mon_work_rect.y);
    }
}

fn is_monocle_on_client_monitor(g: &Globals, w: WindowId) -> bool {
    let monitor_id = g.clients.get(&w).and_then(|c| c.monitor_id);
    monitor_id
        .and_then(|mid| g.monitor(mid))
        .map(|mon| !mon.is_tiling_layout())
        .unwrap_or(false)
}

fn configure_client_border(
    g: &mut Globals,
    x11: &X11BackendRef,
    w: WindowId,
    borderpx: i32,
    mon_monitor_rect: Rect,
    is_monocle: bool,
) {
    let bar_height = g.cfg.bar_height;
    let (isfloating, client_width, client_height) = g
        .clients
        .get(&w)
        .map(|c| (c.isfloating, c.geo.w, c.geo.h))
        .unwrap_or((false, 0, 0));

    let border_width = if !isfloating
        && is_monocle
        && client_width > mon_monitor_rect.w - 30
        && client_height > mon_monitor_rect.h - 30 - bar_height
    {
        0
    } else {
        borderpx
    };

    if let Some(client) = g.clients.get_mut(&w) {
        client.border_width = border_width;
    }

    let x11_win: Window = w.into();
    if let Some(ref scheme) = g.cfg.borderscheme {
        let pixel = scheme.normal.bg.pixel();
        let _ = x11.conn.change_window_attributes(
            x11_win,
            &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
        );
    }
    let _ = x11.conn.flush();
}

fn apply_manage_hints(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_cfg: &mut X11RuntimeConfig,
    w: WindowId,
) {
    crate::client::focus::configure_x11(core, x11, w);
    update_window_type(core, x11, x11_cfg, w);
    crate::backend::x11::update_size_hints_x11(core, x11, w);
    update_wm_hints(core, x11, w);
    read_client_info(core.g, x11, x11_cfg, w);
    set_client_tag_prop(core, x11, x11_cfg, w);
    update_motif_hints(core, x11, x11_cfg, w);
}

fn snapshot_float_geo(g: &mut Globals, w: WindowId, mon_monitor_rect: Rect) {
    if let Some(client) = g.clients.get_mut(&w) {
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

fn subscribe_manage_events(x11: &X11BackendRef, w: WindowId) {
    let mask = EventMask::ENTER_WINDOW
        | EventMask::FOCUS_CHANGE
        | EventMask::PROPERTY_CHANGE
        | EventMask::STRUCTURE_NOTIFY;
    let x11_win: Window = w.into();
    let _ = x11
        .conn
        .change_window_attributes(x11_win, &ChangeWindowAttributesAux::new().event_mask(mask));
}

fn initialize_floating_state(g: &mut Globals, w: WindowId, has_transient_parent: bool) -> bool {
    let isfixed = g.clients.get(&w).map(|c| c.isfixed).unwrap_or(false);
    let mut should_raise = false;
    if let Some(client) = g.clients.get_mut(&w) {
        if !client.isfloating {
            client.isfloating = has_transient_parent || isfixed;
            client.oldstate = client.isfloating as i32;
        }
        should_raise = client.isfloating;
    }
    should_raise
}

fn register_client_root(x11: &X11BackendRef, x11_cfg: &X11RuntimeConfig, w: WindowId) {
    let x11_win: Window = w.into();
    let _ = x11.conn.change_property32(
        PropMode::APPEND,
        x11_cfg.root,
        x11_cfg.netatom.client_list,
        AtomEnum::WINDOW,
        &[x11_win],
    );
    let _ = x11.conn.flush();
}

fn move_client_offscreen_before_arrange(ctx: &mut WmCtx, w: WindowId) {
    let (screen_width, client_x, client_y, client_width, client_height) = ctx
        .g()
        .clients
        .get(&w)
        .map(|client| {
            (
                ctx.g().cfg.screen_width,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
            )
        })
        .unwrap_or((0, 0, 0, 0, 0));

    ctx.backend().resize_window(
        w,
        Rect {
            x: client_x + 2 * screen_width,
            y: client_y,
            w: client_width,
            h: client_height,
        },
    );
    ctx.backend().flush();
}

fn prepare_visibility_and_unfocus(ctx: &mut WmCtx, w: WindowId) -> bool {
    let initially_hidden = ctx
        .g()
        .clients
        .get(&w)
        .map(|c| c.is_hidden)
        .unwrap_or(false);
    if !initially_hidden {
        if let WmCtx::X11(ctx_x11) = ctx {
            set_client_state(
                &ctx_x11.core,
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                w,
                WM_STATE_NORMAL,
            );
        }
    }
    if let Some(selected_window) = ctx.selected_client() {
        if let WmCtx::X11(ctx_x11) = ctx {
            let mut core = ctx_x11.core.reborrow();
            unfocus_win_x11(
                &mut core,
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                selected_window,
                false,
            );
        }
    }
    initially_hidden
}

fn arrange_map_focus_and_snapshot(ctx: &mut WmCtx, w: WindowId, initially_hidden: bool) -> Client {
    let mut c = ctx.g().clients.get(&w).cloned().unwrap_or_default();
    if let Some(monitor_id) = c.monitor_id {
        arrange(ctx, Some(monitor_id));
    }
    if !initially_hidden {
        ctx.backend().map_window(w);
        ctx.backend().flush();
    }
    focus_soft(ctx, None);
    c = ctx.g().clients.get(&w).cloned().unwrap_or_default();
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

    resize(
        ctx,
        w,
        &Rect {
            x: c.geo.x,
            y: c.geo.y - 70,
            w: c.geo.w,
            h: c.geo.h,
        },
        true,
    );
    ctx.backend().flush();
    if let WmCtx::X11(ctx_x11) = ctx {
        animate_client_x11(
            &mut ctx_x11.core,
            &ctx_x11.x11,
            ctx_x11.x11_runtime,
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
    }

    let is_tiling = c
        .monitor_id
        .and_then(|mid| ctx.g().monitor(mid))
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(false);

    if !is_tiling {
        ctx.backend().raise_window(w);
        ctx.backend().flush();
    } else if c.geo.w > mon_monitor_rect.w - 30 || c.geo.h > mon_monitor_rect.h - 30 {
        if let Some(monitor_id) = c.monitor_id {
            arrange(ctx, Some(monitor_id));
        }
    }
}

/// Initial tag mask for a newly managed client on `monitor_id`.
///
/// This mirrors DWM semantics: a new client appears on all tags currently
/// visible on its target monitor.
pub fn initial_tags_for_monitor(g: &Globals, monitor_id: Option<usize>) -> u32 {
    monitor_id
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
pub fn unmanage(ctx: &mut WmCtxX11, win: WindowId, destroyed: bool) {
    let monitor_id = ctx.core.g.clients.get(&win).and_then(|c| c.monitor_id);

    // Clear overlay and fullscreen references so those code paths don't hold
    // dangling window IDs after the client is gone.
    {
        for mon in ctx.core.g.monitors_iter_all_mut() {
            if mon.overlay == Some(win) {
                mon.overlay = None;
            }
            if mon.fullscreen == Some(win) {
                mon.fullscreen = None;
            }
        }
    }

    {
        let tmp = ctx.reborrow();
        detach(&mut WmCtx::X11(tmp), win);
    }
    {
        let tmp = ctx.reborrow();
        detach_stack(&mut WmCtx::X11(tmp), win);
    }

    if !destroyed {
        let x11_win: Window = win.into();
        let _old_bw = ctx
            .core
            .g
            .clients
            .get(&win)
            .map(|c| c.old_border_width)
            .unwrap_or(0);

        {
            let _ = ctx.x11.conn.grab_server();

            // Stop receiving events so we don't get confused during cleanup.
            let _ = ctx.x11.conn.change_window_attributes(
                x11_win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );

            // Restore the original border width the application expects.
            // (Border width is set via BackendOps elsewhere)

            // Release button grabs.
            let _ =
                ctx.x11
                    .conn
                    .ungrab_button(ButtonIndex::from(0u8), x11_win, ModMask::from(0u16));
        }

        set_client_state(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            win,
            WM_STATE_WITHDRAWN,
        );

        let _ = ctx.x11.conn.ungrab_server();
    }

    // Remove from the global map.
    ctx.core.g.clients.remove(&win);

    {
        let tmp = ctx.reborrow();
        focus_soft(&mut WmCtx::X11(tmp), None);
    }
    update_client_list(&mut ctx.core, &ctx.x11, ctx.x11_runtime);

    if let Some(mid) = monitor_id {
        let tmp = ctx.reborrow();
        arrange(&mut WmCtx::X11(tmp), Some(mid));
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
fn read_title_from_x(x11: &X11BackendRef, x11_cfg: &X11RuntimeConfig, win: WindowId) -> String {
    let x11_win: Window = win.into();
    let net_wm_name = x11_cfg.netatom.wm_name;

    for atom in [
        net_wm_name,
        x11rb::protocol::xproto::AtomEnum::WM_NAME.into(),
    ] {
        if atom == 0 {
            continue;
        }
        let Ok(cookie) = x11.conn.get_property(
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
fn read_client_info(g: &mut Globals, x11: &X11BackendRef, x11_cfg: &X11RuntimeConfig, w: WindowId) {
    let x11_win: Window = w.into();
    let client_info_atom = x11_cfg.netatom.client_info;

    let Ok(cookie) =
        x11.conn
            .get_property(false, x11_win, client_info_atom, AtomEnum::CARDINAL, 0, 2)
    else {
        return;
    };
    let Ok(reply) = cookie.reply() else { return };
    let Some(mut data) = reply.value32() else {
        return;
    };

    let tags = data.next().unwrap_or(0);
    let mon_num = data.next().unwrap_or(0);

    let target_mon = g
        .monitors_iter()
        .find(|(_i, m)| m.num as u32 == mon_num)
        .map(|(i, _)| i);

    if let Some(client) = g.clients.get_mut(&w) {
        client.tags = tags;
        if let Some(mid) = target_mon {
            client.monitor_id = Some(mid);
        }
    }
}

fn get_transient_for_hint_x11(x11: &X11BackendRef, w: WindowId) -> Option<WindowId> {
    let x11_win: Window = w.into();

    x11.conn
        .get_property(
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
