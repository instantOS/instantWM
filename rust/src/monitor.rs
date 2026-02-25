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
//! use crate::monitor::{create_monitor_with_ctx, win_to_mon_with_ctx, focus_mon};
//!
//! // Create a monitor from context
//! let mon = create_monitor_with_ctx(&ctx);
//!
//! // Find which monitor a window belongs to
//! let target = win_to_mon_with_ctx(&ctx, some_window);
//!
//! // Focus next/previous monitor
//! focus_mon(&mut ctx, 1);  // Focus next monitor
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

/// Create a new monitor with explicit values.
///
/// This function is useful when you need to create a monitor with
/// specific configuration values different from the defaults.
pub fn create_monitor_with_values(
    mfact: f32,
    nmaster: i32,
    showbar: bool,
    topbar: bool,
) -> Monitor {
    Monitor::new_with_values(mfact, nmaster, showbar, topbar)
}

/// Create a new monitor with default values from context.
///
/// This is the dependency-injected version that accepts a `WmCtx`.
pub fn create_monitor_with_ctx(ctx: &WmCtx) -> Monitor {
    Monitor::new_with_values(
        ctx.g.cfg.mfact,
        ctx.g.cfg.nmaster,
        ctx.g.cfg.showbar,
        ctx.g.cfg.topbar,
    )
}

/// Remove a monitor and clean up its resources.
///
/// This function uses dependency injection by accepting a WmCtx
/// instead of accessing global state directly.
pub fn cleanup_monitor(ctx: &mut WmCtx, mon_id: MonitorId) {
    if mon_id >= ctx.g.monitors.len() {
        return;
    }

    let barwin = ctx.g.monitors[mon_id].barwin;

    ctx.g.monitors.remove(mon_id);

    if ctx.g.selmon == mon_id {
        ctx.g.selmon = 0;
    } else if ctx.g.selmon > mon_id {
        ctx.g.selmon -= 1;
    }

    if barwin != 0 {
        {
            let conn = ctx.x11.conn;
            let _ = x11rb::protocol::xproto::unmap_window(conn, barwin);
            let _ = x11rb::protocol::xproto::destroy_window(conn, barwin);
        }
    }
}

/// Get the monitor ID in the given direction from the selected monitor.
///
/// This function uses dependency injection by accepting references to
/// monitor state instead of accessing global state.
///
/// # Arguments
/// * `monitors` - Slice of all monitors
/// * `selmon` - Currently selected monitor ID
/// * `dir` - Direction (> 0 for next, < 0 for previous)
///
/// # Returns
/// * `Some(monitor_id)` - The target monitor ID
/// * `None` - If there are no monitors
pub fn dir_to_mon(monitors: &[Monitor], selmon: MonitorId, dir: i32) -> Option<MonitorId> {
    find_monitor_by_direction(monitors, selmon, dir)
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
            return rect_to_mon(&ctx.g.monitors, ctx.g.selmon, &Rect { x, y, w: 1, h: 1 });
        }
        return if ctx.g.monitors.is_empty() {
            None
        } else {
            Some(ctx.g.selmon)
        };
    }

    for (i, m) in ctx.g.monitors.iter().enumerate() {
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
        Some(ctx.g.selmon)
    }
}

pub fn send_mon(_ctx: &mut WmCtx, c_win: Window, target_mon_id: MonitorId) {
    let g = get_globals_mut();

    let current_mon_id = g.selmon;

    if current_mon_id == target_mon_id {
        return;
    }

    let (is_scratchpad, target_tags) = {
        let client = match g.clients.get(&c_win) {
            Some(c) => c,
            None => return,
        };
        let is_sp = client.tags == SCRATCHPAD_MASK;
        let tags = if !is_sp {
            g.monitors
                .get(target_mon_id)
                .map(|m| m.tagset[m.seltags as usize])
                .unwrap_or(1)
        } else {
            0
        };
        (is_sp, tags)
    };

    if let Some(_win) = get_win_to_client(c_win) {
        let x11 = get_x11();
        let mut g = get_globals_mut();
        let mut ctx = WmCtx::new(g, x11.as_conn());
        unfocus_win(&mut ctx, c_win, true);
    }

    detach(c_win);
    detach_stack(c_win);

    {
        let g = get_globals_mut();
        if let Some(client) = g.clients.get_mut(&c_win) {
            client.mon_id = Some(target_mon_id);

            if !is_scratchpad {
                client.tags = target_tags;
            }
        }
    }

    if !is_scratchpad {
        let x11 = get_x11();
        let mut g = get_globals_mut();
        let mut ctx = WmCtx::new(g, x11.as_conn());
        // Get client data first, then call reset_sticky
        let mon_id = ctx.g.clients.get(&c_win).and_then(|c| c.mon_id);
        if mon_id.is_some() {
            // Create a temporary client reference for reset_sticky
            let client_opt = ctx.g.clients.get_mut(&c_win);
            if client_opt.is_some() {
                // We need to get the window and call reset_sticky on it directly
                drop(client_opt);
                // Call reset_sticky with just the window
                crate::tags::reset_sticky_win(&mut ctx, c_win);
            }
        }
    }

    attach(c_win);
    attach_stack(c_win);
    set_client_tag_prop(c_win);

    {
        let x11 = get_x11();
        let mut g = get_globals_mut();
        let mut ctx = WmCtx::new(g, x11.as_conn());
        focus(&mut ctx, None);
    }

    {
        let g = get_globals();
        if let Some(c) = g.clients.get(&c_win) {
            if !c.isfloating {
                let x11 = get_x11();
                let mut g = get_globals_mut();
                let mut ctx = WmCtx::new(g, x11.as_conn());
                crate::layouts::arrange(&mut ctx, None);
            }
        }
    }

    if is_scratchpad {
        let g = get_globals();
        if let Some(c) = g.clients.get(&c_win) {
            if c.is_scratchpad() && !c.issticky {
                {
                    let x11 = get_x11();
                    let mut g = get_globals_mut();
                    let mut ctx = WmCtx::new(g, x11.as_conn());
                    let sel = ctx.g.selmon;
                    if let Some(win) = get_selected_client_win(sel) {
                        unfocus_win(&mut ctx, win, false);
                    }
                    ctx.g.selmon = target_mon_id;
                }

                let sp_name = {
                    let g = get_globals();
                    g.clients.get(&c_win).map(|c| c.scratchpad_name.clone())
                };

                if let Some(name) = sp_name {
                    let x11 = get_x11();
                    let mut g = get_globals_mut();
                    let mut ctx = WmCtx::new(g, x11.as_conn());
                    crate::scratchpad::scratchpad_show_name(&mut ctx, &name);
                }

                {
                    let x11 = get_x11();
                    let mut g = get_globals_mut();
                    let mut ctx = WmCtx::new(g, x11.as_conn());
                    let sel = ctx.g.selmon;
                    if let Some(win) = get_selected_client_win(sel) {
                        unfocus_win(&mut ctx, win, false);
                    }
                    ctx.g.selmon = current_mon_id;
                }

                let x11 = get_x11();
                let mut g = get_globals_mut();
                let mut ctx = WmCtx::new(g, x11.as_conn());
                focus(&mut ctx, None);
            }
        }
    }
}

/// Change focus to the next or previous monitor.
///
/// This function uses dependency injection by accepting a WmCtx
/// instead of accessing global state directly.
///
/// # Arguments
/// * `ctx` - WM context with mutable access to monitor state
/// * `direction` - Direction (> 0 for next, < 0 for previous)
pub fn focus_mon(ctx: &mut WmCtx, direction: i32) {
    if ctx.g.monitors.len() <= 1 {
        return;
    }

    let target = match find_monitor_by_direction(&ctx.g.monitors, ctx.g.selmon, direction) {
        Some(id) => id,
        None => return,
    };

    if target == ctx.g.selmon {
        return;
    }

    let old_id = ctx.g.selmon;
    if let Some(win) = ctx.g.monitors.get(old_id).and_then(|m| m.sel) {
        unfocus_win(ctx, win, false);
    }

    ctx.g.selmon = target;

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

    let mut target = 0;
    for i in 0..index as usize {
        if i + 1 < ctx.g.monitors.len() {
            target = i + 1;
        } else {
            break;
        }
    }

    let old_id = ctx.g.selmon;
    if let Some(win) = ctx.g.monitors.get(old_id).and_then(|m| m.sel) {
        unfocus_win(ctx, win, false);
    }

    ctx.g.selmon = target;

    focus(ctx, None);
}

pub fn follow_mon(ctx: &mut WmCtx, direction: i32) {
    let c_win = match ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) {
        Some(w) => w,
        None => return,
    };

    crate::tags::tag_mon(ctx, direction);

    if let Some(mon_id) = ctx.g.clients.get(&c_win).and_then(|c| c.mon_id) {
        ctx.g.selmon = mon_id;
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
    while g.monitors.len() < count {
        g.monitors
            .push(create_monitor_with_values(mfact, nmaster, showbar, topbar));
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
        dirty = move_clients_to_mon0(i) || dirty;

        let g = get_globals_mut();
        if g.selmon == i {
            g.selmon = 0;
        }

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
    g.monitors
        .push(create_monitor_with_values(mfact, nmaster, showbar, topbar));
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
    g.selmon = 0;
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
        if let Some(ref mut m) = g.monitors.get_mut(i) {
            if update_monitor_geometry(m, i, info) {
                dirty = true;
                monitors_need_bar_update.push(i);
            }
        }
    }

    // Update bar positions for changed monitors
    for idx in &monitors_need_bar_update {
        let g = get_globals_mut();
        if let Some(ref mut m) = g.monitors.get_mut(*idx) {
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
        g.selmon = 0;
        // Create a temporary context to find the monitor for the root window
        let ctx = WmCtx::new(g, x11.as_conn());
        if let Some(m) = win_to_mon_with_ctx(&ctx, x11.screen_num as u32) {
            ctx.g.selmon = m;
        }
    }

    Some(dirty)
}

pub fn update_geom() -> bool {
    eprintln!("TRACE: update_geom - start");
    let mut dirty = false;

    #[cfg(feature = "xinerama")]
    {
        eprintln!("TRACE: update_geom - before get_x11");
        let x11 = get_x11();
        eprintln!("TRACE: update_geom - after get_x11");

        if let Some(ref conn) = x11.conn {
            if let Some(result) = update_from_xinerama(x11, conn) {
                eprintln!("TRACE: update_geom - xinerama result = {}", result);
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

    eprintln!("TRACE: update_geom - end, dirty = {}", dirty);
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
    if let Some(mon) = g.monitors.get(mon_id) {
        if let Some(win) = mon.sel {
            return g.clients.get(&win).cloned();
        }
    }
    None
}

fn get_selected_client_win(mon_id: MonitorId) -> Option<Window> {
    let g = get_globals();
    g.monitors.get(mon_id).and_then(|m| m.sel)
}
