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

use crate::bar::x11::update_bar_pos;
use crate::client::{
    attach, attach_stack, detach, detach_stack, set_client_tag_prop, unfocus_win,
    win_to_client as get_win_to_client,
};
use crate::contexts::WmCtx;
use crate::focus::{focus, warp_cursor_to_client};
use crate::globals::{get_globals, get_globals_mut, get_x11};
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

    let barwin = ctx.g.monitor(mon_id).map(|m| m.barwin).unwrap_or(0);

    ctx.g.remove_monitor(mon_id);

    if barwin != 0 {
        {
            let conn = ctx.x11.conn;
            let _ = x11rb::protocol::xproto::unmap_window(conn, barwin);
            let _ = x11rb::protocol::xproto::destroy_window(conn, barwin);
        }
    }
}

/// Find which monitor a rectangle intersects with the most.
///
/// This function uses dependency injection by accepting references to
/// monitor state instead of accessing global state.
///
/// # Arguments
/// * `monitors` - Slice of all monitors
/// * `selmon` - Currently selected monitor ID (fallback)
/// * `rect` - The rectangle to check
///
/// # Returns
/// * `Some(monitor_id)` - The monitor with maximum intersection area
/// * `None` - If there are no monitors
//TODO: get rid of this function, use find_monitor_by_rect instead
pub fn rect_to_mon(monitors: &[Monitor], selmon: MonitorId, rect: &Rect) -> Option<MonitorId> {
    find_monitor_by_rect(monitors, rect).or(Some(selmon))
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
pub fn win_to_mon_with_ctx(ctx: &WmCtx, w: Window) -> Option<MonitorId> {
    if w == ctx.g.cfg.root {
        if let Some((x, y)) = get_root_ptr_with_conn(ctx.x11.conn) {
            return rect_to_mon(
                &ctx.g.monitors,
                ctx.g.selmon_id(),
                &Rect { x, y, w: 1, h: 1 },
            );
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

    if let Some(win) = get_win_to_client(w) {
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
pub fn transfer_client(ctx: &mut WmCtx, win: Window, target_mon: MonitorId) {
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

    if get_win_to_client(win).is_some() {
        unfocus_win(ctx, win, true);
    }

    detach(win);
    detach_stack(win);

    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.mon_id = Some(target_mon);
        if !is_scratchpad {
            client.tags = target_tags;
        }
    }

    if !is_scratchpad {
        crate::tags::reset_sticky_win(ctx, win);
    }

    attach(win);
    attach_stack(win);
    set_client_tag_prop(ctx, win);

    focus(ctx, None);

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
fn handle_scratchpad_transfer(ctx: &mut WmCtx, win: Window, target_mon: MonitorId) {
    let Some(client) = ctx.g.clients.get(&win) else {
        return;
    };
    if !client.is_scratchpad() || client.issticky {
        return;
    }

    let sp_name = client.scratchpad_name.clone();
    let current_mon = ctx.g.selmon_id();

    // Unfocus on current monitor and switch to target
    if let Some(sel_win) = get_selected_client_win(current_mon) {
        unfocus_win(ctx, sel_win, false);
    }
    ctx.g.set_selmon(target_mon);

    // Show the scratchpad on the target monitor
    crate::scratchpad::scratchpad_show_name(ctx, &sp_name);

    // Unfocus on target monitor and switch back
    if let Some(sel_win) = get_selected_client_win(target_mon) {
        unfocus_win(ctx, sel_win, false);
    }
    ctx.g.set_selmon(current_mon);

    focus(ctx, None);
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

    focus(ctx, None);
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

    focus(ctx, None);
}

pub fn follow_mon(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx.g.selmon().and_then(|m| m.sel) {
        Some(w) => w,
        None => return,
    };

    crate::tags::tag_mon(ctx, direction);

    if let Some(mon_id) = ctx.g.clients.get(&c_win).and_then(|c| c.mon_id) {
        ctx.g.set_selmon(mon_id);
    }

    focus(ctx, Some(c_win));

    {
        let conn = ctx.x11.conn;
        let _ = x11rb::protocol::xproto::configure_window(
            conn,
            c_win,
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
fn ensure_monitor_count(count: usize) {
    let g = get_globals_mut();
    let (mfact, nmaster, showbar, topbar) =
        (g.cfg.mfact, g.cfg.nmaster, g.cfg.showbar, g.cfg.topbar);
    let template = g.cfg.tag_template.clone();
    while g.monitors.len() < count {
        let mut mon = Monitor::new_with_values(mfact, nmaster, showbar, topbar);
        mon.init_tags(&template);
        g.push_monitor(mon);
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
fn move_clients_to_mon0(removed_mon_id: usize) -> bool {
    let clients_to_move: Vec<Window> = {
        let g = get_globals();
        g.clients
            .values()
            .filter(|c| c.mon_id == Some(removed_mon_id))
            .map(|c| c.win)
            .collect()
    };

    let mut dirty = false;
    for win in clients_to_move {
        dirty = true;
        detach(win);
        detach_stack(win);

        let g = get_globals_mut();
        if let Some(ref mut c) = g.clients.get_mut(&win) {
            c.mon_id = Some(0);
        }

        attach(win);
        attach_stack(win);
    }

    dirty
}

/// Handle removal of monitors that are no longer present.
fn cleanup_removed_monitors(start_idx: usize, x11: &crate::globals::X11Connection) -> bool {
    let mut dirty = false;

    for i in (start_idx..get_globals().monitors.len()).rev() {
        // NOTE: monitors.len() is re-evaluated each iteration as cleanup_monitor shrinks the vec
        dirty = move_clients_to_mon0(i) || dirty;

        let g = get_globals_mut();
        // selmon fixup is handled inside cleanup_monitor → remove_monitor
        let mut ctx = WmCtx::new(g, x11.as_conn());
        cleanup_monitor(&mut ctx, i);
    }

    dirty
}

/// Initialize a single monitor with the given dimensions.
fn init_single_monitor(sw: i32, sh: i32) -> bool {
    let g = get_globals_mut();
    let (mfact, nmaster, showbar, topbar) =
        (g.cfg.mfact, g.cfg.nmaster, g.cfg.showbar, g.cfg.topbar);
    let template = g.cfg.tag_template.clone();
    let mut mon = Monitor::new_with_values(mfact, nmaster, showbar, topbar);
    mon.init_tags(&template);
    g.push_monitor(mon);
    if let Some(ref mut m) = g.monitors.first_mut() {
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
        update_bar_pos(m);
    }
    g.set_selmon(0);
    true
}

/// Update single monitor dimensions if changed.
fn update_single_monitor(sw: i32, sh: i32) -> bool {
    let needs_update = get_globals()
        .monitors
        .first()
        .map(|m| m.monitor_rect.w != sw || m.monitor_rect.h != sh)
        .unwrap_or(false);

    if !needs_update {
        return false;
    }

    let g = get_globals_mut();
    if let Some(ref mut m) = g.monitors.first_mut() {
        m.monitor_rect.w = sw;
        m.monitor_rect.h = sh;
        m.work_rect.w = sw;
        m.work_rect.h = sh;
        update_bar_pos(m);
    }
    true
}

/// Update monitor geometries from Xinerama screens.
#[cfg(feature = "xinerama")]
fn update_from_xinerama(
    x11: &crate::globals::X11Connection,
    conn: &x11rb::rust_connection::RustConnection,
) -> Option<bool> {
    let unique = get_unique_screens(conn)?;
    let new_count = unique.len();
    let old_count = get_globals().monitors.len();

    // Add new monitors if needed
    ensure_monitor_count(new_count);

    // Update existing monitor geometries
    let mut dirty = new_count > old_count;
    let mut monitors_need_bar_update: Vec<usize> = Vec::new();

    for (i, info) in unique.iter().enumerate() {
        let g = get_globals_mut();
        if let Some(m) = g.monitor_mut(i) {
            if update_monitor_geometry(m, i, info) {
                dirty = true;
                monitors_need_bar_update.push(i);
            }
        }
    }

    // Update bar positions for changed monitors
    for idx in &monitors_need_bar_update {
        let g = get_globals_mut();
        if let Some(m) = g.monitor_mut(*idx) {
            update_bar_pos(m);
        }
    }

    // Cleanup removed monitors
    if new_count < old_count {
        dirty = cleanup_removed_monitors(new_count, x11) || dirty;
    }

    // Reset selection to first monitor and try to find better one
    if dirty {
        let g = get_globals_mut();
        g.set_selmon(0);
        // Create a temporary context to find the monitor for the root window
        let ctx = WmCtx::new(g, x11.as_conn());
        if let Some(m) = win_to_mon_with_ctx(&ctx, x11.screen_num as u32) {
            ctx.g.set_selmon(m);
        }
    }

    Some(dirty)
}

pub fn update_geom() -> bool {
    let dirty;

    #[cfg(feature = "xinerama")]
    {
        let x11 = get_x11();

        if let Some(ref conn) = x11.conn {
            if let Some(result) = update_from_xinerama(x11, conn) {
                return result;
            }
        }
    }

    // Fallback to single monitor
    let g = get_globals();
    let (sw, sh) = (g.cfg.sw, g.cfg.sh);

    if g.monitors.is_empty() {
        dirty = init_single_monitor(sw, sh);
    } else {
        dirty = update_single_monitor(sw, sh);
    }

    dirty
}

/// Get the root pointer position using an explicit connection.
///
/// This is the dependency-injected version that accepts an X11 connection.
fn get_root_ptr_with_conn(conn: &x11rb::rust_connection::RustConnection) -> Option<(i32, i32)> {
    let g = get_globals();
    if let Ok(cookie) = x11rb::protocol::xproto::query_pointer(conn, g.cfg.root) {
        if let Ok(reply) = cookie.reply() {
            return Some((reply.root_x as i32, reply.root_y as i32));
        }
    }
    None
}

fn get_root_ptr() -> Option<(i32, i32)> {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        return get_root_ptr_with_conn(conn);
    }
    None
}

fn get_selected_client(mon_id: MonitorId) -> Option<Client> {
    let g = get_globals();
    g.monitor(mon_id)
        .and_then(|mon| mon.sel)
        .and_then(|win| g.clients.get(&win).cloned())
}

fn get_selected_client_win(mon_id: MonitorId) -> Option<Window> {
    get_globals().monitor(mon_id).and_then(|m| m.sel)
}
