//! Monitor management with dependency injection support.
//!
//! This module provides monitor management functionality using a dependency-injected
//! API via the `WmCtx` context type. All public functions accept explicit context
//! rather than accessing global state directly.
//!
//! # API Usage
//!
//! ```ignore
//! use crate::contexts::WmCtx;
//! use crate::monitor::{win_to_mon_with_ctx, focus_mon};
//!
//! // Find which monitor a window belongs to
//! let target = win_to_mon_with_ctx(&ctx, some_window);
//!
//! // Focus next/previous monitor
//! use crate::types::MonitorDirection;
//! focus_mon(&mut ctx, MonitorDirection::NEXT);  // Focus next monitor
//! ```
//!
//! # Benefits of Dependency Injection
//!
//! - **Testability**: Functions can be tested with mock state
//! - **Reduced coupling**: No hidden dependencies on global state
//! - **Thread safety**: Easier to reason about with explicit state passing
//! - **Flexibility**: Can work with temporary state without affecting globals

use crate::bar::x11::update_bar_pos_with_bh;
use crate::client::{attach, attach_stack, detach, detach_stack, set_client_tag_prop, unfocus_win};
use crate::contexts::WmCtx;
use crate::focus::warp_cursor_to_client;
use crate::types::*;
use x11rb::protocol::xproto::Window;

#[cfg(feature = "xinerama")]
use x11rb::protocol::xinerama;

/// Remove a monitor and clean up its resources.
///
/// This function uses dependency injection by accepting a WmCtx
/// instead of accessing global state directly.
pub fn cleanup_monitor(ctx: &mut WmCtx, mon_id: MonitorId) {
    if mon_id >= ctx.g.monitors.len() {
        return;
    }

    let barwin = ctx.g.monitor(mon_id).map(|m| m.barwin).unwrap_or_default();

    ctx.g.remove_monitor(mon_id);

    if barwin != WindowId::default() {
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            let x11_barwin: Window = barwin.into();
            let _ = x11rb::protocol::xproto::unmap_window(conn, x11_barwin);
            let _ = x11rb::protocol::xproto::destroy_window(conn, x11_barwin);
        }
    }
}

/// Find which monitor a window belongs to, using explicit context.
///
/// This is the dependency-injected version that accepts a `WmCtx`.
///
/// # Arguments
/// * `ctx` - WM context with access to monitors, clients, and X11 connection
/// * `w` - The window to find the monitor for
///
/// # Returns
/// * `Some(monitor_id)` - The monitor ID the window belongs to
/// * `None` - If no monitor could be determined
pub fn win_to_mon_with_ctx(ctx: &WmCtx, w: WindowId) -> Option<MonitorId> {
    let root_win = WindowId::from(ctx.g.cfg.root);
    if w == root_win {
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            if let Some((x, y)) = get_root_ptr_with_conn_and_root(conn, ctx.g.cfg.root) {
                let rect = Rect { x, y, w: 1, h: 1 };
                return crate::types::find_monitor_by_rect(&ctx.g.monitors, &rect)
                    .or(Some(ctx.g.selmon_id()));
            }
        }
        return if ctx.g.monitors.is_empty() {
            None
        } else {
            Some(ctx.g.selmon_id())
        };
    }

    for (i, m) in ctx.g.monitors_iter() {
        if w == m.barwin {
            return Some(i);
        }
    }

    if ctx.g.clients.contains_key(&w) {
        let win = w;
        return ctx.g.clients.get(&win).and_then(|c| c.mon_id);
    }

    if ctx.g.monitors.is_empty() {
        None
    } else {
        Some(ctx.g.selmon_id())
    }
}

/// Transfer a client to a different monitor.
///
/// Detaches the client from its current monitor, updates its monitor
/// assignment and tags, then reattaches it to the target monitor.
/// Handles special cases like scratchpads and sticky windows.
pub fn transfer_client(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    if ctx.g.selmon_id() == target_mon {
        return;
    }

    let (is_scratchpad, target_tags) = {
        let client = match ctx.g.clients.get(&win) {
            Some(c) => c,
            None => return,
        };
        let is_sp = client.tags == SCRATCHPAD_MASK;
        let tags = if !is_sp {
            ctx.g
                .monitors
                .get(target_mon)
                .map(|m| m.tagset[m.seltags as usize])
                .unwrap_or(1)
        } else {
            0
        };
        (is_sp, tags)
    };

    if ctx.g.clients.contains_key(&win) {
        unfocus_win(ctx, win, true);
    }

    detach(ctx, win);
    detach_stack(ctx, win);

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.mon_id = Some(target_mon);
        if !is_scratchpad {
            client.tags = target_tags;
        }
    }

    if !is_scratchpad {
        crate::tags::reset_sticky_win(ctx, win);
    }

    attach(ctx, win);
    attach_stack(ctx, win);
    set_client_tag_prop(ctx, win);

    crate::focus::focus_soft(ctx, None);

    let needs_arrange = ctx
        .g
        .clients
        .get(&win)
        .map(|c| !c.isfloating)
        .unwrap_or(false);
    if needs_arrange {
        crate::layouts::arrange(ctx, None);
    }

    if is_scratchpad {
        handle_scratchpad_transfer(ctx, win, target_mon);
    }
}

/// Handle scratchpad-specific logic during monitor transfer.
fn handle_scratchpad_transfer(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    let Some(client) = ctx.g.clients.get(&win) else {
        return;
    };
    if !client.is_scratchpad() || client.issticky {
        return;
    }

    let sp_name = client.scratchpad_name.clone();
    let current_mon = ctx.g.selmon_id();

    // Unfocus on current monitor and switch to target
    if let Some(sel_win) = get_selected_client_win(&ctx.g, current_mon) {
        unfocus_win(ctx, sel_win, false);
    }
    ctx.g.set_selmon(target_mon);

    // Show the scratchpad on the target monitor
    crate::scratchpad::scratchpad_show_name(ctx, &sp_name);

    // Unfocus on target monitor and switch back
    if let Some(sel_win) = get_selected_client_win(&ctx.g, target_mon) {
        unfocus_win(ctx, sel_win, false);
    }
    ctx.g.set_selmon(current_mon);

    crate::focus::focus_soft(ctx, None);
}

/// Change focus to the next or previous monitor.
///
/// This function uses dependency injection by accepting a WmCtx
/// instead of accessing global state directly.
///
/// # Arguments
/// * `ctx` - WM context with mutable access to monitor state
/// * `direction` - Direction to move (Next or Previous monitor)
pub fn focus_mon(ctx: &mut WmCtx, direction: MonitorDirection) {
    if ctx.g.monitors.len() <= 1 {
        return;
    }

    let target = match find_monitor_by_direction(&ctx.g.monitors, ctx.g.selmon_id(), direction) {
        Some(id) => id,
        None => return,
    };

    if target == ctx.g.selmon_id() {
        return;
    }

    let old_id = ctx.g.selmon_id();
    if let Some(win) = ctx.g.monitor(old_id).and_then(|m| m.sel) {
        unfocus_win(ctx, win, false);
    }

    ctx.g.set_selmon(target);

    crate::focus::focus_soft(ctx, None);
}

/// Change focus to a specific monitor by index.
///
/// This function uses dependency injection by accepting a WmCtx
/// instead of accessing global state directly.
///
/// # Arguments
/// * `ctx` - WM context with mutable access to monitor state
/// * `index` - Target monitor index (clamped to available monitors)
pub fn focus_n_mon(ctx: &mut WmCtx, index: i32) {
    if ctx.g.monitors.len() <= 1 {
        return;
    }

    let target = (index as usize).min(ctx.g.monitors.len() - 1);

    let old_id = ctx.g.selmon_id();
    if let Some(win) = ctx.g.monitor(old_id).and_then(|m| m.sel) {
        unfocus_win(ctx, win, false);
    }

    ctx.g.set_selmon(target);

    crate::focus::focus_soft(ctx, None);
}

pub fn follow_mon(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx.g.selmon().and_then(|m| m.sel) {
        Some(w) => w,
        None => return,
    };

    crate::tags::send_to_monitor(ctx, direction);

    if let Some(mon_id) = ctx.g.clients.get(&c_win).and_then(|c| c.mon_id) {
        ctx.g.set_selmon(mon_id);
    }

    crate::focus::focus_soft(ctx, Some(c_win));

    if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
        let x11_win: Window = c_win.into();
        let _ = x11rb::protocol::xproto::configure_window(
            conn,
            x11_win,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
        );
    }

    warp_cursor_to_client(ctx, c_win);
}

#[cfg(feature = "xinerama")]
fn is_unique_geom(unique: &[Rect], info: &Rect) -> bool {
    !unique
        .iter()
        .any(|u| u.x == info.x && u.y == info.y && u.w == info.w && u.h == info.h)
}

/// Get unique screen geometries from Xinerama screens.
#[cfg(feature = "xinerama")]
fn get_unique_screens(conn: &x11rb::rust_connection::RustConnection) -> Option<Vec<Rect>> {
    let is_active = xinerama::is_active(conn).ok()?.reply().ok()?;
    if is_active.state == 0 {
        return None;
    }

    let screens = xinerama::query_screens(conn).ok()?.reply().ok()?;

    let screen_info: Vec<Rect> = screens
        .screen_info
        .iter()
        .map(|s| Rect {
            x: s.x_org as i32,
            y: s.y_org as i32,
            w: s.width as i32,
            h: s.height as i32,
        })
        .collect();

    let mut unique = Vec::new();
    for info in &screen_info {
        if is_unique_geom(&unique, info) {
            unique.push(*info);
        }
    }

    Some(unique)
}

/// Ensure we have at least `count` monitors.
fn ensure_monitor_count_ctx(ctx: &mut WmCtx, count: usize) {
    let (mfact, nmaster, showbar, topbar) = (
        ctx.g.cfg.mfact,
        ctx.g.cfg.nmaster,
        ctx.g.cfg.showbar,
        ctx.g.cfg.topbar,
    );
    let template = ctx.g.cfg.tag_template.clone();
    while ctx.g.monitors.len() < count {
        let mut mon = Monitor::new_with_values(mfact, nmaster, showbar, topbar);
        mon.init_tags(&template);
        ctx.g.push_monitor(mon);
    }
}

/// Update monitor geometry if changed. Returns true if updated.
fn update_monitor_geometry(mon: &mut Monitor, idx: usize, new_rect: &Rect) -> bool {
    let needs_update = mon.monitor_rect.x != new_rect.x
        || mon.monitor_rect.y != new_rect.y
        || mon.monitor_rect.w != new_rect.w
        || mon.monitor_rect.h != new_rect.h;

    if needs_update {
        mon.num = idx as i32;
        mon.monitor_rect = *new_rect;
        mon.work_rect = *new_rect;
        true
    } else {
        false
    }
}

/// Move clients from a removed monitor to monitor 0.
fn move_clients_to_mon0_ctx(ctx: &mut WmCtx, removed_mon_id: usize) -> bool {
    let clients_to_move: Vec<WindowId> = ctx
        .g
        .clients
        .values()
        .filter(|c| c.mon_id == Some(removed_mon_id))
        .map(|c| c.win)
        .collect();

    let mut dirty = false;
    for win in clients_to_move {
        dirty = true;

        detach(ctx, win);
        detach_stack(ctx, win);

        if let Some(c) = ctx.g.clients.get_mut(&win) {
            c.mon_id = Some(0);
        }

        attach(ctx, win);
        attach_stack(ctx, win);
    }

    dirty
}

/// Handle removal of monitors that are no longer present.
fn cleanup_removed_monitors_ctx(ctx: &mut WmCtx, start_idx: usize) -> bool {
    let mut dirty = false;

    for i in (start_idx..ctx.g.monitors.len()).rev() {
        // monitors.len() is re-evaluated each iteration as cleanup_monitor shrinks the vec
        dirty = move_clients_to_mon0_ctx(ctx, i) || dirty;
        cleanup_monitor(ctx, i);
    }

    dirty
}

/// Initialize a single monitor with the given dimensions.
fn init_single_monitor_ctx(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let (mfact, nmaster, showbar, topbar) = (
        ctx.g.cfg.mfact,
        ctx.g.cfg.nmaster,
        ctx.g.cfg.showbar,
        ctx.g.cfg.topbar,
    );
    let template = ctx.g.cfg.tag_template.clone();
    let mut mon = Monitor::new_with_values(mfact, nmaster, showbar, topbar);
    mon.init_tags(&template);
    ctx.g.push_monitor(mon);
    let bh = ctx.g.cfg.bar_height;
    if let Some(m) = ctx.g.monitors.first_mut() {
        m.num = 0;
        m.monitor_rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h: sh,
        };
        m.work_rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h: sh,
        };
        update_bar_pos_with_bh(m, bh);
    }
    ctx.g.set_selmon(0);
    true
}

/// Update single monitor dimensions if changed.
fn update_single_monitor_ctx(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let needs_update = ctx
        .g
        .monitors
        .first()
        .map(|m| m.monitor_rect.w != sw || m.monitor_rect.h != sh)
        .unwrap_or(false);

    if !needs_update {
        return false;
    }

    let bh = ctx.g.cfg.bar_height;
    if let Some(m) = ctx.g.monitors.first_mut() {
        m.monitor_rect.w = sw;
        m.monitor_rect.h = sh;
        m.work_rect.w = sw;
        m.work_rect.h = sh;
        update_bar_pos_with_bh(m, bh);
    }
    true
}

/// Update monitor geometries from Xinerama screens.
#[cfg(feature = "xinerama")]
fn update_from_xinerama(ctx: &mut WmCtx) -> Option<bool> {
    let conn = ctx.x11_conn().map(|x11| x11.conn)?;
    let unique = get_unique_screens(conn)?;
    let new_count = unique.len();
    let old_count = ctx.g.monitors.len();

    // Add new monitors if needed
    ensure_monitor_count_ctx(ctx, new_count);

    // Update existing monitor geometries
    let mut dirty = new_count > old_count;
    let mut monitors_need_bar_update: Vec<usize> = Vec::new();

    for (i, info) in unique.iter().enumerate() {
        if let Some(m) = ctx.g.monitor_mut(i) {
            if update_monitor_geometry(m, i, info) {
                dirty = true;
                monitors_need_bar_update.push(i);
            }
        }
    }

    // Update bar positions for changed monitors
    let bh = ctx.g.cfg.bar_height;
    for idx in &monitors_need_bar_update {
        if let Some(m) = ctx.g.monitor_mut(*idx) {
            update_bar_pos_with_bh(m, bh);
        }
    }

    // Cleanup removed monitors
    if new_count < old_count {
        dirty = cleanup_removed_monitors_ctx(ctx, new_count) || dirty;
    }

    // Reset selection to first monitor and try to find better one
    if dirty {
        ctx.g.set_selmon(0);
        if let Some(m) = win_to_mon_with_ctx(ctx, WindowId::from(ctx.g.cfg.root)) {
            ctx.g.set_selmon(m);
        }
    }

    Some(dirty)
}

pub fn update_geom_ctx(ctx: &mut WmCtx) -> bool {
    #[cfg(feature = "xinerama")]
    {
        if let Some(result) = update_from_xinerama(ctx) {
            return result;
        }
    }

    // Fallback to single monitor
    let sw = ctx.g.cfg.screen_width.max(1);
    let sh = ctx.g.cfg.screen_height.max(1);

    if ctx.g.monitors.is_empty() {
        init_single_monitor_ctx(ctx, sw, sh)
    } else {
        update_single_monitor_ctx(ctx, sw, sh)
    }
}

/// Get the root pointer position for an explicit connection.
fn get_root_ptr_with_conn(
    conn: &x11rb::rust_connection::RustConnection,
    root: Window,
) -> Option<(i32, i32)> {
    get_root_ptr_with_conn_and_root(conn, root)
}

fn get_root_ptr_with_conn_and_root(
    conn: &x11rb::rust_connection::RustConnection,
    root: Window,
) -> Option<(i32, i32)> {
    if let Ok(cookie) = x11rb::protocol::xproto::query_pointer(conn, root) {
        if let Ok(reply) = cookie.reply() {
            return Some((reply.root_x as i32, reply.root_y as i32));
        }
    }
    None
}

fn get_selected_client_win(g: &crate::globals::Globals, mon_id: MonitorId) -> Option<WindowId> {
    g.monitor(mon_id).and_then(|m| m.sel)
}
