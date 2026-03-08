//! Monitor management via the `MonitorManager` struct.
//!
//! This module encapsulates monitor state and logic, providing a clean API
//! for monitor-related operations.

use crate::backend::x11::X11BackendRef;
use crate::client::{attach, attach_stack, detach, detach_stack, set_client_tag_prop};
use crate::contexts::WmCtx;
use crate::focus::{focus_soft, unfocus_win};
use crate::types::*;
use std::collections::HashMap;
use x11rb::protocol::xproto::Window;

use x11rb::protocol::xinerama;

/// Manages the collection of monitors and the current selection.
#[derive(Default)]
pub struct MonitorManager {
    pub monitors: Vec<Monitor>,
    pub selected_monitor_idx: usize,
}

impl MonitorManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -------------------------------------------------------------------------
    // Data Accessors
    // -------------------------------------------------------------------------

    pub fn sel_idx(&self) -> usize {
        self.selected_monitor_idx
    }

    pub fn set_sel_idx(&mut self, idx: usize) {
        if idx < self.monitors.len() {
            self.selected_monitor_idx = idx;
        }
    }

    pub fn get(&self, idx: usize) -> Option<&Monitor> {
        self.monitors.get(idx)
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Monitor> {
        self.monitors.get_mut(idx)
    }

    pub fn sel(&self) -> Option<&Monitor> {
        self.monitors.get(self.selected_monitor_idx)
    }

    pub fn sel_unchecked(&self) -> &Monitor {
        self.monitors
            .get(self.selected_monitor_idx)
            .expect("no monitors")
    }

    pub fn sel_mut(&mut self) -> Option<&mut Monitor> {
        self.monitors.get_mut(self.selected_monitor_idx)
    }

    pub fn sel_mut_unchecked(&mut self) -> &mut Monitor {
        self.monitors
            .get_mut(self.selected_monitor_idx)
            .expect("no monitors")
    }

    pub fn count(&self) -> usize {
        self.monitors.len()
    }

    pub fn len(&self) -> usize {
        self.monitors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    pub fn clear(&mut self) {
        self.monitors.clear();
        self.selected_monitor_idx = 0;
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &Monitor)> {
        self.monitors.iter().enumerate()
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &Monitor> {
        self.monitors.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (usize, &mut Monitor)> {
        self.monitors.iter_mut().enumerate()
    }

    pub fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut Monitor> {
        self.monitors.iter_mut()
    }

    pub fn push(&mut self, mut m: Monitor) -> usize {
        let id = self.monitors.len();
        m.monitor_id = id;
        self.monitors.push(m);
        id
    }

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn set_monitor(&mut self, idx: usize, m: Monitor) {
        if idx < self.monitors.len() {
            self.monitors[idx] = m;
        }
    }

    pub fn win_to_mon(
        &self,
        w: WindowId,
        root: Window,
        clients: &HashMap<WindowId, Client>,
        x11: Option<X11BackendRef<'_>>,
    ) -> Option<usize> {
        if w == WindowId::from(root) {
            if let Some(conn) = x11 {
                if let Some((x, y)) = get_root_ptr_with_conn_and_root(conn.conn, root) {
                    let rect = Rect { x, y, w: 1, h: 1 };
                    return crate::types::find_monitor_by_rect(&self.monitors, &rect)
                        .or(Some(self.selected_monitor_idx));
                }
            }
            return if self.monitors.is_empty() {
                None
            } else {
                Some(self.selected_monitor_idx)
            };
        }

        for (i, m) in self.iter() {
            if w == m.bar_win {
                return Some(i);
            }
        }

        if let Some(c) = clients.get(&w) {
            return c.monitor_id;
        }

        if self.monitors.is_empty() {
            None
        } else {
            Some(self.selected_monitor_idx)
        }
    }
}

// -----------------------------------------------------------------------------
// Orchestration Logic (Free functions that coordinate multiple managers)
// -----------------------------------------------------------------------------

pub fn cleanup_monitor(ctx: &mut WmCtx, monitor_id: usize) {
    if monitor_id >= ctx.g_mut().monitors.len() {
        return;
    }

    let bar_win = ctx
        .g
        .monitors
        .get(monitor_id)
        .map(|m| m.bar_win)
        .unwrap_or_default();

    // Remove and fix up IDs
    ctx.g_mut().monitors.monitors.remove(monitor_id);
    for (i, m) in ctx.g_mut().monitors.monitors.iter_mut().enumerate() {
        m.monitor_id = i;
    }

    // Adjust selected index
    if ctx.g_mut().monitors.selected_monitor_idx == monitor_id {
        ctx.g_mut().monitors.selected_monitor_idx = 0;
    } else if ctx.g_mut().monitors.selected_monitor_idx > monitor_id {
        ctx.g_mut().monitors.selected_monitor_idx -= 1;
    }

    if bar_win != WindowId::default() {
        if let WmCtx::X11(x11) = ctx {
            let x11_bar_win: Window = bar_win.into();
            let _ = x11rb::protocol::xproto::unmap_window(x11.x11.conn, x11_bar_win);
            let _ = x11rb::protocol::xproto::destroy_window(x11.x11.conn, x11_bar_win);
        }
    }
}

pub fn transfer_client(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    if ctx.g_mut().monitors.sel_idx() == target_mon {
        return;
    }

    let (is_scratchpad, target_tags) = {
        let client = match ctx.g_mut().clients.get(&win) {
            Some(c) => c,
            None => return,
        };
        let is_scratchpad = client.is_scratchpad();
        let tags = if !is_scratchpad {
            ctx.g
                .monitors
                .get(target_mon)
                .map(|m| m.selected_tags())
                .unwrap_or(1)
        } else {
            0
        };
        (is_scratchpad, tags)
    };

    if ctx.g_mut().clients.contains(&win) {
        unfocus_win(ctx, win, true);
    }

    detach(ctx, win);
    detach_stack(ctx, win);

    if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
        client.monitor_id = Some(target_mon);
        if !is_scratchpad {
            client.tags = target_tags;
        }
    }

    if !is_scratchpad {
        crate::tags::reset_sticky_win(ctx.core_mut(), win);
    }

    attach(ctx, win);
    attach_stack(ctx, win);
    if let WmCtx::X11(x11) = ctx {
        set_client_tag_prop(&mut x11.core, &x11.x11, x11.x11_runtime, win);
    }

    focus_soft(ctx, None);

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

pub fn focus_monitor(ctx: &mut WmCtx, direction: MonitorDirection) {
    let target = {
        let mgr = &ctx.g_mut().monitors;
        if mgr.monitors.len() <= 1 {
            return;
        }
        match find_monitor_by_direction(&mgr.monitors, mgr.selected_monitor_idx, direction) {
            Some(id) => id,
            None => return,
        }
    };

    if target == ctx.g_mut().monitors.sel_idx() {
        return;
    }

    if let Some(win) = ctx.g_mut().monitors.sel().and_then(|m| m.sel) {
        unfocus_win(ctx, win, false);
    }

    ctx.g_mut().monitors.set_sel_idx(target);
    focus_soft(ctx, None);
}

pub fn focus_n_mon(ctx: &mut WmCtx, index: i32) {
    let target = {
        let mgr = &ctx.g_mut().monitors;
        if mgr.monitors.len() <= 1 {
            return;
        }
        (index as usize).min(mgr.monitors.len() - 1)
    };

    if let Some(win) = ctx.g_mut().monitors.sel().and_then(|m| m.sel) {
        unfocus_win(ctx, win, false);
    }

    ctx.g_mut().monitors.set_sel_idx(target);
    focus_soft(ctx, None);
}

pub fn move_to_monitor_and_follow(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx.g_mut().monitors.sel().and_then(|m| m.sel) {
        Some(w) => w,
        None => return,
    };

    crate::tags::send_to_monitor(ctx, direction);

    if let Some(monitor_id) = ctx.g_mut().clients.get(&c_win).and_then(|c| c.monitor_id) {
        ctx.g_mut().monitors.set_sel_idx(monitor_id);
    }

    focus_soft(ctx, Some(c_win));

    if let WmCtx::X11(x11) = ctx {
        let x11_win: Window = c_win.into();
        let _ = x11rb::protocol::xproto::configure_window(
            x11.x11.conn,
            x11_win,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
        );
    }
    ctx.warp_cursor_to_client(c_win);
}

pub fn update_geom(ctx: &mut WmCtx) -> bool {
    if let Some(result) = update_from_xinerama(ctx) {
        return result;
    }

    let sw = ctx.g_mut().cfg.screen_width.max(1);
    let sh = ctx.g_mut().cfg.screen_height.max(1);

    if ctx.g_mut().monitors.is_empty() {
        init_single_monitor(ctx, sw, sh)
    } else {
        update_single_monitor(ctx, sw, sh)
    }
}

// -----------------------------------------------------------------------------
// Internal Helpers
// -----------------------------------------------------------------------------

fn handle_scratchpad_transfer(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    let Some(client) = ctx.g_mut().clients.get(&win) else {
        return;
    };
    if !client.is_scratchpad() || client.issticky {
        return;
    }

    let sp_name = client.scratchpad_name.clone();
    let current_mon = ctx.g_mut().monitors.sel_idx();

    if let Some(selected_window) = ctx.g_mut().monitors.get(current_mon).and_then(|m| m.sel) {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.g_mut().monitors.set_sel_idx(target_mon);

    crate::scratchpad::scratchpad_show_name(ctx, &sp_name);

    if let Some(selected_window) = ctx.g_mut().monitors.get(target_mon).and_then(|m| m.sel) {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.g_mut().monitors.set_sel_idx(current_mon);

    focus_soft(ctx, None);
}

fn init_single_monitor(ctx: &mut WmCtx, sw: i32, h: i32) -> bool {
    let template = ctx.g_mut().cfg.tag_template.clone();
    let mut mon = Monitor::new_with_values(
        ctx.g_mut().cfg.mfact,
        ctx.g_mut().cfg.nmaster,
        ctx.g_mut().cfg.show_bar,
        ctx.g_mut().cfg.topbar,
    );
    mon.init_tags(&template);
    ctx.g_mut().monitors.push(mon);
    let bar_height = ctx.g_mut().cfg.bar_height;
    if let Some(m) = ctx.g_mut().monitors.get_mut(0) {
        m.num = 0;
        m.monitor_rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h: h,
        };
        m.work_rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h: h,
        };
        m.update_bar_position(bar_height);
    }
    ctx.g_mut().monitors.set_sel_idx(0);
    true
}

fn update_single_monitor(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let needs_update = ctx
        .g
        .monitors
        .get(0)
        .map(|m| m.monitor_rect.w != sw || m.monitor_rect.h != sh)
        .unwrap_or(false);
    if !needs_update {
        return false;
    }

    let bar_height = ctx.g_mut().cfg.bar_height;
    if let Some(m) = ctx.g_mut().monitors.get_mut(0) {
        m.monitor_rect.w = sw;
        m.monitor_rect.h = sh;
        m.work_rect.w = sw;
        m.work_rect.h = sh;
        m.update_bar_position(bar_height);
    }
    true
}

fn update_from_xinerama(ctx: &mut WmCtx) -> Option<bool> {
    let conn = match ctx {
        WmCtx::X11(x11) => x11.x11.conn,
        WmCtx::Wayland(_) => return None,
    };
    let is_active = xinerama::is_active(conn).ok()?.reply().ok()?;
    if is_active.state == 0 {
        return None;
    }

    let screens = xinerama::query_screens(conn).ok()?.reply().ok()?;
    // conn borrow ends here; the rest only needs ctx
    let mut unique = Vec::new();
    for s in &screens.screen_info {
        let info = Rect {
            x: s.x_org as i32,
            y: s.y_org as i32,
            w: s.width as i32,
            h: s.height as i32,
        };
        if !unique
            .iter()
            .any(|u: &Rect| u.x == info.x && u.y == info.y && u.w == info.w && u.h == info.h)
        {
            unique.push(info);
        }
    }

    let new_count = unique.len();
    let old_count = ctx.g_mut().monitors.count();

    // Ensure count
    let template = ctx.g_mut().cfg.tag_template.clone();
    let (mfact, nmaster, showbar, topbar) = (
        ctx.g_mut().cfg.mfact,
        ctx.g_mut().cfg.nmaster,
        ctx.g_mut().cfg.show_bar,
        ctx.g_mut().cfg.topbar,
    );
    while ctx.g_mut().monitors.count() < new_count {
        let mut mon = Monitor::new_with_values(mfact, nmaster, showbar, topbar);
        mon.init_tags(&template);
        ctx.g_mut().monitors.push(mon);
    }

    let mut dirty = new_count > old_count;
    let bar_height = ctx.g_mut().cfg.bar_height;

    for (i, info) in unique.iter().enumerate() {
        if let Some(m) = ctx.g_mut().monitors.get_mut(i) {
            if m.monitor_rect.x != info.x
                || m.monitor_rect.y != info.y
                || m.monitor_rect.w != info.w
                || m.monitor_rect.h != info.h
            {
                m.num = i as i32;
                m.monitor_rect = *info;
                m.work_rect = *info;
                m.update_bar_position(bar_height);
                dirty = true;
            }
        }
    }

    if new_count < old_count {
        for i in (new_count..old_count).rev() {
            let clients_to_move: Vec<WindowId> = ctx
                .g
                .clients
                .values()
                .filter(|c| c.monitor_id == Some(i))
                .map(|c| c.win)
                .collect();
            for win in clients_to_move {
                detach(ctx, win);
                detach_stack(ctx, win);
                if let Some(c) = ctx.g_mut().clients.get_mut(&win) {
                    c.monitor_id = Some(0);
                }
                attach(ctx, win);
                attach_stack(ctx, win);
                dirty = true;
            }
            cleanup_monitor(ctx, i);
        }
    }

    if dirty {
        ctx.g_mut().monitors.set_sel_idx(0);
        let (x11_conn, root) = match ctx {
            WmCtx::X11(x11) => (
                Some(X11BackendRef::new(x11.x11.conn, x11.x11.screen_num)),
                WindowId::from(x11.x11_runtime.root),
            ),
            WmCtx::Wayland(_) => (None, WindowId::default()),
        };
        let clients = ctx.g().clients.clone();
        let root_u32: u32 = root.into();
        if let Some(m) = ctx
            .g_mut()
            .monitors
            .win_to_mon(root, root_u32, &clients, x11_conn)
        {
            ctx.g_mut().monitors.set_sel_idx(m);
        }
    }

    Some(dirty)
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
