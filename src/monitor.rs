//! Monitor management via the `MonitorManager` struct.
//!
//! This module encapsulates monitor state and logic, providing a clean API
//! for monitor-related operations.

use crate::backend::BackendOps;
use crate::backend::x11::set_client_tag_prop;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::focus::{focus_soft, unfocus_win};
use crate::layouts::arrange;
use crate::types::*;
use std::collections::HashMap;
use x11rb::protocol::xproto::Window;

use x11rb::protocol::xinerama;

/// Manages the collection of monitors and the current selection.
#[derive(Default)]
pub struct MonitorManager {
    pub monitors: Vec<Monitor>,
    pub selected_monitor_idx: MonitorId,
}

impl MonitorManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -------------------------------------------------------------------------
    // Data Accessors
    // -------------------------------------------------------------------------

    pub fn sel_idx(&self) -> MonitorId {
        self.selected_monitor_idx
    }

    pub fn set_sel_idx(&mut self, idx: MonitorId) {
        if idx.index() < self.monitors.len() {
            self.selected_monitor_idx = idx;
        }
    }

    pub fn get(&self, idx: MonitorId) -> Option<&Monitor> {
        self.monitors.get(idx.index())
    }

    pub fn get_mut(&mut self, idx: MonitorId) -> Option<&mut Monitor> {
        self.monitors.get_mut(idx.index())
    }

    pub fn sel(&self) -> Option<&Monitor> {
        self.monitors.get(self.selected_monitor_idx.index())
    }

    pub fn sel_unchecked(&self) -> &Monitor {
        self.monitors
            .get(self.selected_monitor_idx.index())
            .expect("no monitors")
    }

    pub fn sel_mut(&mut self) -> Option<&mut Monitor> {
        self.monitors.get_mut(self.selected_monitor_idx.index())
    }

    pub fn sel_mut_unchecked(&mut self) -> &mut Monitor {
        self.monitors
            .get_mut(self.selected_monitor_idx.index())
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
        self.selected_monitor_idx = MonitorId(0);
    }

    pub fn iter(&self) -> impl Iterator<Item = (MonitorId, &Monitor)> {
        self.monitors
            .iter()
            .enumerate()
            .map(|(idx, monitor)| (MonitorId(idx), monitor))
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &Monitor> {
        self.monitors.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (MonitorId, &mut Monitor)> {
        self.monitors
            .iter_mut()
            .enumerate()
            .map(|(idx, monitor)| (MonitorId(idx), monitor))
    }

    pub fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut Monitor> {
        self.monitors.iter_mut()
    }

    pub fn push(&mut self, mut m: Monitor) -> MonitorId {
        let id = MonitorId(self.monitors.len());
        m.monitor_id = id;
        self.monitors.push(m);
        id
    }

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn set_monitor(&mut self, idx: MonitorId, m: Monitor) {
        if idx.index() < self.monitors.len() {
            self.monitors[idx.index()] = m;
        }
    }

    pub fn find_monitor_for(
        &self,
        w: WindowId,
        clients: &HashMap<WindowId, Client>,
    ) -> Option<MonitorId> {
        for (i, m) in self.iter() {
            if w == m.bar_win {
                return Some(i);
            }
        }

        if let Some(c) = clients.get(&w) {
            return Some(c.monitor_id);
        }

        None
    }

    pub fn find_id_by_rect(&self, rect: &Rect) -> Option<MonitorId> {
        crate::types::find_monitor_by_rect(&self.monitors, rect).or(Some(self.selected_monitor_idx))
    }

    pub fn find_by_rect(&self, rect: &Rect) -> Option<&Monitor> {
        self.find_id_by_rect(rect).and_then(|id| self.get(id))
    }

    pub fn find_monitor_at_pointer(&self, ptr: (i32, i32)) -> Option<MonitorId> {
        let rect = Rect {
            x: ptr.0,
            y: ptr.1,
            w: 1,
            h: 1,
        };
        self.find_id_by_rect(&rect)
    }
}

// -----------------------------------------------------------------------------
// Orchestration Logic (Free functions that coordinate multiple managers)
// -----------------------------------------------------------------------------

pub fn cleanup_monitor(ctx: &mut WmCtx, monitor_id: MonitorId) {
    if monitor_id.index() >= ctx.core_mut().globals_mut().monitors.len() {
        return;
    }

    let bar_win = ctx
        .core()
        .globals()
        .monitors
        .get(monitor_id)
        .map(|m| m.bar_win)
        .unwrap_or_default();

    // Remove and fix up IDs
    ctx.core_mut()
        .globals_mut()
        .monitors
        .monitors
        .remove(monitor_id.index());
    for (i, m) in ctx
        .core_mut()
        .globals_mut()
        .monitors
        .monitors
        .iter_mut()
        .enumerate()
    {
        m.monitor_id = MonitorId(i);
    }

    // Adjust selected index
    if ctx.core_mut().globals_mut().monitors.selected_monitor_idx == monitor_id {
        ctx.core_mut().globals_mut().monitors.selected_monitor_idx = MonitorId(0);
    } else if ctx.core_mut().globals_mut().monitors.selected_monitor_idx > monitor_id {
        let current = ctx
            .core_mut()
            .globals_mut()
            .monitors
            .selected_monitor_idx
            .index();
        ctx.core_mut().globals_mut().monitors.selected_monitor_idx = MonitorId(current - 1);
    }

    // Fix up client monitor references
    let target = ctx.core_mut().globals_mut().monitors.selected_monitor_idx;
    for client in ctx.core_mut().globals_mut().clients.values_mut() {
        if client.monitor_id == monitor_id {
            client.monitor_id = target;
        } else if client.monitor_id > monitor_id {
            client.monitor_id = MonitorId(client.monitor_id.index() - 1);
        }
    }

    if bar_win != WindowId::default()
        && let WmCtx::X11(x11) = ctx
    {
        let x11_bar_win: Window = bar_win.into();
        let _ = x11rb::protocol::xproto::unmap_window(x11.x11.conn, x11_bar_win);
        let _ = x11rb::protocol::xproto::destroy_window(x11.x11.conn, x11_bar_win);
    }
}

pub fn transfer_client(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    if ctx.core_mut().globals_mut().monitors.sel_idx() == target_mon {
        return;
    }

    let (is_scratchpad, target_tags) = {
        let client = match ctx.client(win) {
            Some(c) => c,
            None => return,
        };
        let is_scratchpad = client.is_scratchpad();
        let tags = if !is_scratchpad {
            ctx.core()
                .globals()
                .monitors
                .get(target_mon)
                .map(|m| m.selected_tags())
                .unwrap_or(crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY))
        } else {
            crate::types::TagMask::EMPTY
        };
        (is_scratchpad, tags)
    };

    if ctx.core_mut().globals_mut().clients.contains_key(&win) {
        unfocus_win(ctx, win, true);
    }

    ctx.core_mut().globals_mut().detach(win);
    ctx.core_mut().globals_mut().detach_stack(win);

    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.monitor_id = target_mon;
        if !is_scratchpad {
            client.set_tag_mask(target_tags);
        }
    }

    if !is_scratchpad {
        crate::tags::reset_sticky_win(ctx.core_mut(), win);
    }

    ctx.core_mut().globals_mut().attach(win);
    ctx.core_mut().globals_mut().attach_stack(win);
    if let WmCtx::X11(x11) = ctx {
        set_client_tag_prop(&x11.core, &x11.x11, x11.x11_runtime, win);
    }

    focus_soft(ctx, None);

    let needs_arrange = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| !c.is_floating)
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
        let mgr = &ctx.core_mut().globals_mut().monitors;
        if mgr.monitors.len() <= 1 {
            return;
        }
        match find_monitor_by_direction(&mgr.monitors, mgr.selected_monitor_idx, direction) {
            Some(id) => id,
            None => return,
        }
    };

    if target == ctx.core_mut().globals_mut().monitors.sel_idx() {
        return;
    }

    if let Some(win) = ctx
        .core_mut()
        .globals_mut()
        .monitors
        .sel()
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, win, false);
    }

    ctx.core_mut().globals_mut().monitors.set_sel_idx(target);
    focus_soft(ctx, None);
}

pub fn focus_n_mon(ctx: &mut WmCtx, index: i32) {
    let target = {
        let mgr = &ctx.core_mut().globals_mut().monitors;
        if mgr.monitors.len() <= 1 {
            return;
        }
        MonitorId((index as usize).min(mgr.monitors.len() - 1))
    };

    if let Some(win) = ctx
        .core_mut()
        .globals_mut()
        .monitors
        .sel()
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, win, false);
    }

    ctx.core_mut().globals_mut().monitors.set_sel_idx(target);
    focus_soft(ctx, None);
}

pub fn move_to_monitor_and_follow(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx
        .core_mut()
        .globals_mut()
        .monitors
        .sel()
        .and_then(|m| m.sel)
    {
        Some(w) => w,
        None => return,
    };

    crate::tags::send_to_monitor(ctx, direction);

    if let Some(monitor_id) = ctx.core().globals().clients.monitor_id(c_win) {
        ctx.core_mut()
            .globals_mut()
            .monitors
            .set_sel_idx(monitor_id);
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

pub fn apply_monitor_config(ctx: &mut WmCtx) {
    let monitors_cfg = ctx.core().globals().cfg.monitors.clone();

    // Apply wildcard first as fallback
    if let Some(wildcard_cfg) = monitors_cfg.get("*") {
        ctx.backend().set_monitor_config("*", wildcard_cfg);
    }

    // Apply specific configs
    for (name, config) in monitors_cfg {
        if name != "*" {
            ctx.backend().set_monitor_config(&name, &config);
        }
    }

    ctx.core_mut().globals_mut().dirty.monitor_config = false;
    update_geom(ctx);
}

pub fn update_geom(ctx: &mut WmCtx) -> bool {
    // Try the backend's get_outputs first (uses XRandR on X11, native on Wayland)
    let outputs = ctx.backend().get_outputs();
    if outputs.len() > 1 || (outputs.len() == 1 && outputs[0].name != "X11") {
        return update_from_outputs(ctx, outputs);
    }

    // Fall back to Xinerama for X11
    if let WmCtx::X11(x11) = ctx
        && let Some(result) = update_from_xinerama(x11)
    {
        return result;
    }

    // Final fallback to single monitor
    let sw = ctx.core_mut().globals_mut().cfg.screen_width.max(1);
    let sh = ctx.core_mut().globals_mut().cfg.screen_height.max(1);

    if ctx.core_mut().globals_mut().monitors.is_empty() {
        init_single_monitor(ctx, sw, sh)
    } else {
        update_single_monitor(ctx, sw, sh)
    }
}

fn update_from_outputs(ctx: &mut WmCtx, outputs: Vec<crate::backend::BackendOutputInfo>) -> bool {
    let mut changed = false;
    let old_count = ctx.core().globals().monitors.len();

    if old_count != outputs.len() {
        changed = true;
    }

    let layout_width = outputs
        .iter()
        .map(|output| output.rect.x.saturating_add(output.rect.w))
        .max()
        .unwrap_or(1)
        .max(1);
    let layout_height = outputs
        .iter()
        .map(|output| output.rect.y.saturating_add(output.rect.h))
        .max()
        .unwrap_or(1)
        .max(1);

    {
        let cfg = &mut ctx.core_mut().globals_mut().cfg;
        if cfg.screen_width != layout_width || cfg.screen_height != layout_height {
            cfg.screen_width = layout_width;
            cfg.screen_height = layout_height;
            changed = true;
        }
    }

    let mut new_monitors = Vec::new();
    for (i, output) in outputs.into_iter().enumerate() {
        let mut m = Monitor::new_with_values(
            ctx.core().globals().cfg.mfact,
            ctx.core().globals().cfg.nmaster,
            ctx.core().globals().cfg.show_bar,
            ctx.core().globals().cfg.top_bar,
        );
        m.num = i as i32;
        m.monitor_rect = output.rect;
        m.work_rect = output.rect;
        m.name = output.name;
        let (bar_height, horizontal_padding, startmenu_size) =
            scaled_monitor_ui_metrics(ctx.core().globals(), output.scale);
        m.set_ui_metrics(output.scale, bar_height, horizontal_padding, startmenu_size);
        m.init_tags(&ctx.core().globals().cfg.tag_template);
        m.update_bar_position(m.bar_height);
        new_monitors.push(m);
    }

    // Preserve existing tags/clients if possible
    for (i, new_m) in new_monitors.iter_mut().enumerate() {
        if let Some(old_m) = ctx.core().globals().monitors.get(MonitorId(i)) {
            new_m.tags = old_m.tags.clone();
            new_m.clients = old_m.clients.clone();
            new_m.stack = old_m.stack.clone();
            new_m.sel = old_m.sel;

            if old_m.monitor_rect.w != new_m.monitor_rect.w
                || old_m.monitor_rect.h != new_m.monitor_rect.h
                || old_m.monitor_rect.x != new_m.monitor_rect.x
                || old_m.monitor_rect.y != new_m.monitor_rect.y
                || old_m.bar_height != new_m.bar_height
                || old_m.horizontal_padding != new_m.horizontal_padding
                || old_m.startmenu_size != new_m.startmenu_size
                || (old_m.ui_scale - new_m.ui_scale).abs() > f64::EPSILON
            {
                changed = true;
            }
        }
    }

    ctx.core_mut().globals_mut().monitors.monitors = new_monitors;
    if ctx.core().globals().monitors.selected_monitor_idx.index()
        >= ctx.core().globals().monitors.len()
    {
        ctx.core_mut().globals_mut().monitors.selected_monitor_idx = MonitorId(0);
    }

    if changed {
        ctx.core_mut().globals_mut().dirty.layout = true;
        // The bar renderer also needs a poke
        ctx.core_mut().bar.mark_dirty();
    }

    changed
}

fn scaled_i32(value: i32, scale: f64) -> i32 {
    if value <= 0 {
        return 0;
    }
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    ((value as f64) * scale).round() as i32
}

fn scaled_monitor_ui_metrics(globals: &crate::globals::Globals, scale: f64) -> (i32, i32, i32) {
    (
        scaled_i32(globals.cfg.bar_height, scale).max(1),
        scaled_i32(globals.cfg.horizontal_padding, scale).max(1),
        scaled_i32(globals.cfg.startmenusize, scale).max(1),
    )
}

// -----------------------------------------------------------------------------
// Internal Helpers
// -----------------------------------------------------------------------------

fn handle_scratchpad_transfer(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    let Some(client) = ctx.client(win) else {
        return;
    };
    if !client.is_scratchpad() || client.issticky {
        return;
    }

    let sp_name = client.scratchpad_name.clone();
    let current_mon = ctx.core_mut().globals_mut().monitors.sel_idx();

    if let Some(selected_window) = ctx
        .core_mut()
        .globals_mut()
        .monitors
        .get(current_mon)
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.core_mut()
        .globals_mut()
        .monitors
        .set_sel_idx(target_mon);

    let _ = crate::floating::scratchpad_show_name(ctx, &sp_name);

    if let Some(selected_window) = ctx
        .core_mut()
        .globals_mut()
        .monitors
        .get(target_mon)
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.core_mut()
        .globals_mut()
        .monitors
        .set_sel_idx(current_mon);

    focus_soft(ctx, None);
}

fn init_single_monitor(ctx: &mut WmCtx, sw: i32, h: i32) -> bool {
    let template = ctx.core_mut().globals_mut().cfg.tag_template.clone();
    let mut mon = Monitor::new_with_values(
        ctx.core_mut().globals_mut().cfg.mfact,
        ctx.core_mut().globals_mut().cfg.nmaster,
        ctx.core_mut().globals_mut().cfg.show_bar,
        ctx.core_mut().globals_mut().cfg.top_bar,
    );
    mon.init_tags(&template);
    ctx.core_mut().globals_mut().monitors.push(mon);
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().globals(), 1.0);
    if let Some(m) = ctx.core_mut().globals_mut().monitors.get_mut(MonitorId(0)) {
        m.num = 0;
        m.monitor_rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h,
        };
        m.work_rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h,
        };
        m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
        m.update_bar_position(bar_height);
    }
    ctx.core_mut()
        .globals_mut()
        .monitors
        .set_sel_idx(MonitorId(0));
    true
}

fn update_single_monitor(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().globals(), 1.0);
    let needs_update = ctx
        .core()
        .globals()
        .monitors
        .get(MonitorId(0))
        .map(|m| {
            m.monitor_rect.w != sw
                || m.monitor_rect.h != sh
                || m.bar_height != bar_height
                || m.horizontal_padding != horizontal_padding
                || m.startmenu_size != startmenu_size
        })
        .unwrap_or(false);
    if !needs_update {
        return false;
    }

    if let Some(m) = ctx.core_mut().globals_mut().monitors.get_mut(MonitorId(0)) {
        m.monitor_rect.w = sw;
        m.monitor_rect.h = sh;
        m.work_rect.w = sw;
        m.work_rect.h = sh;
        m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
        m.update_bar_position(bar_height);
    }
    true
}

fn update_from_xinerama(x11: &mut WmCtxX11) -> Option<bool> {
    let conn = x11.x11.conn;
    let is_active = xinerama::is_active(conn).ok()?.reply().ok()?;
    if is_active.state == 0 {
        return None;
    }

    let screens = xinerama::query_screens(conn).ok()?.reply().ok()?;
    // conn borrow ends here; the rest only needs x11
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

    // Borrow g in a limited scope for monitor updates
    let (
        old_count,
        _template,
        _mfact,
        _nmaster,
        _showbar,
        _topbar,
        _bar_height,
        _horizontal_padding,
        _startmenu_size,
        any_changed,
    ) = {
        let g = x11.core.globals_mut();
        let old_count = g.monitors.count();
        let mut any_changed = false;

        // Ensure count
        let template = g.cfg.tag_template.clone();
        let (mfact, nmaster, showbar, topbar) =
            (g.cfg.mfact, g.cfg.nmaster, g.cfg.show_bar, g.cfg.top_bar);
        while g.monitors.count() < new_count {
            let mut mon = Monitor::new_with_values(mfact, nmaster, showbar, topbar);
            mon.init_tags(&template);
            g.monitors.push(mon);
        }

        let bar_height = g.cfg.bar_height;
        let horizontal_padding = g.cfg.horizontal_padding;
        let startmenu_size = g.cfg.startmenusize;

        for (i, info) in unique.iter().enumerate() {
            if let Some(m) = g.monitors.get_mut(MonitorId(i)) {
                let geometry_changed = m.monitor_rect.x != info.x
                    || m.monitor_rect.y != info.y
                    || m.monitor_rect.w != info.w
                    || m.monitor_rect.h != info.h;
                let metrics_changed = m.bar_height != bar_height
                    || m.horizontal_padding != horizontal_padding
                    || m.startmenu_size != startmenu_size
                    || (m.ui_scale - 1.0).abs() > f64::EPSILON;
                if geometry_changed || metrics_changed {
                    any_changed = true;
                    m.num = i as i32;
                    m.monitor_rect = *info;
                    m.work_rect = *info;
                    m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
                    m.update_bar_position(bar_height);
                }
            }
        }
        (
            old_count,
            template,
            mfact,
            nmaster,
            showbar,
            topbar,
            bar_height,
            horizontal_padding,
            startmenu_size,
            any_changed,
        )
    };

    let mut dirty = new_count > old_count || any_changed;

    if new_count < old_count {
        // Get clients while not holding mutable borrow
        let clients_map = x11.core.globals().clients.map().clone();
        for i in (new_count..old_count).rev() {
            let clients_to_move: Vec<WindowId> = clients_map
                .values()
                .filter(|c| c.monitor_id == MonitorId(i))
                .map(|c| c.win)
                .collect();
            // Create temporary WmCtx wrapper for each iteration
            let mut wm_ctx = WmCtx::X11(x11.reborrow());
            for win in clients_to_move {
                wm_ctx.core_mut().globals_mut().detach(win);
                wm_ctx.core_mut().globals_mut().detach_stack(win);
                if let Some(c) = wm_ctx.client_mut(win) {
                    c.monitor_id = MonitorId(0);
                }
                wm_ctx.core_mut().globals_mut().attach(win);
                wm_ctx.core_mut().globals_mut().attach_stack(win);
                dirty = true;
            }
            cleanup_monitor(&mut wm_ctx, MonitorId(i));
        }
    }

    if dirty {
        x11.core.globals_mut().monitors.set_sel_idx(MonitorId(0));
        if let Ok(cookie) =
            x11rb::protocol::xproto::query_pointer(x11.x11.conn, x11.x11_runtime.root)
            && let Ok(reply) = cookie.reply()
        {
            let ptr = (reply.root_x as i32, reply.root_y as i32);
            if let Some(m) = x11.core.globals_mut().monitors.find_monitor_at_pointer(ptr) {
                x11.core.globals_mut().monitors.set_sel_idx(m);
            }
        }
    }

    Some(dirty)
}

pub enum Direction {
    Up,
    Down,
}

pub fn reorder_client(ctx: &mut WmCtx, win: WindowId, direction: Direction) {
    let tiled_count = {
        let g = ctx.core_mut().globals_mut();
        g.selected_monitor().tiled_client_count(g.clients.map())
    };
    if tiled_count < 2 {
        return;
    }

    let is_floating = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| c.is_floating)
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();

    if let Some(mon) = ctx.core_mut().globals_mut().monitors.get_mut(selmon_id)
        && let Some(pos) = mon.clients.iter().position(|&w| w == win)
    {
        match direction {
            Direction::Up => {
                if pos > 0 {
                    mon.clients.swap(pos, pos - 1);
                } else {
                    let last = mon.clients.pop();
                    if let Some(last_win) = last {
                        mon.clients.insert(1, last_win);
                    }
                }
            }
            Direction::Down => {
                if pos + 1 < mon.clients.len() {
                    mon.clients.swap(pos, pos + 1);
                } else {
                    let first = mon.clients.remove(0);
                    mon.clients.push(first);
                }
            }
        }
    }

    focus_soft(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}
