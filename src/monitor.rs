//! Monitor management via the `MonitorManager` struct.
//!
//! This module encapsulates monitor state and logic, providing a clean API
//! for monitor-related operations.

use crate::backend::x11::set_client_tag_prop;
use crate::backend::{BackendOps, BackendOutputInfo, BackendVrrSupport};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::focus::{focus_soft, unfocus_win};
use crate::globals::Globals;
use crate::globals::RuntimeConfig;
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

fn destroy_monitor_bar_x11(ctx: &mut WmCtx, bar_win: WindowId) {
    if bar_win != WindowId::default()
        && let WmCtx::X11(x11) = ctx
    {
        let x11_bar_win: Window = bar_win.into();
        let _ = x11rb::protocol::xproto::unmap_window(x11.x11.conn, x11_bar_win);
        let _ = x11rb::protocol::xproto::destroy_window(x11.x11.conn, x11_bar_win);
    }
}

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

    destroy_monitor_bar_x11(ctx, bar_win);
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
    ctx.core_mut().globals_mut().detach_z_order(win);

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
    ctx.core_mut().globals_mut().attach_z_order_top(win);
    if let WmCtx::X11(x11) = ctx {
        set_client_tag_prop(&x11.core, &x11.x11, x11.x11_runtime, win);
    }

    focus_soft(ctx, None);

    let needs_arrange = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| !c.mode.is_floating())
        .unwrap_or(false);
    if needs_arrange {
        ctx.core_mut()
            .globals_mut()
            .queue_layout_for_all_monitors_urgent();
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

    refresh_monitor_layout(ctx);
}

pub fn refresh_monitor_layout(ctx: &mut WmCtx) -> bool {
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

fn output_layout_extent(outputs: &[BackendOutputInfo]) -> (i32, i32) {
    let w = outputs
        .iter()
        .map(|o| o.rect.x.saturating_add(o.rect.w))
        .max()
        .unwrap_or(1)
        .max(1);
    let h = outputs
        .iter()
        .map(|o| o.rect.y.saturating_add(o.rect.h))
        .max()
        .unwrap_or(1)
        .max(1);
    (w, h)
}

fn sync_runtime_screen_size(
    cfg: &mut RuntimeConfig,
    layout_width: i32,
    layout_height: i32,
) -> bool {
    if cfg.screen_width != layout_width || cfg.screen_height != layout_height {
        cfg.screen_width = layout_width;
        cfg.screen_height = layout_height;
        true
    } else {
        false
    }
}

fn make_monitor_for_output(
    i: usize,
    output: &BackendOutputInfo,
    pool: &mut Vec<Option<Monitor>>,
    old_to_new: &mut [Option<MonitorId>],
    template: &[TagNames],
    showbar: bool,
    topbar: bool,
    globals: &Globals,
    changed: &mut bool,
) -> Monitor {
    let (bh, hp, sm) = scaled_monitor_ui_metrics(globals, output.scale);
    match take_matching_monitor(pool, i, output) {
        Some((j, mut m)) => {
            let geom_changed = m.monitor_rect != output.rect
                || m.name != output.name
                || (m.ui_scale - output.scale).abs() > f64::EPSILON
                || m.bar_height != bh
                || m.horizontal_padding != hp
                || m.startmenu_size != sm;
            if geom_changed {
                *changed = true;
            }
            old_to_new[j] = Some(MonitorId(i));
            m.apply_output_layout(
                i,
                output.name.clone(),
                output.rect,
                output.scale,
                bh,
                hp,
                sm,
            );
            m
        }
        None => {
            *changed = true;
            let mut m = Monitor::new_with_values(showbar, topbar);
            m.init_tags(template);
            m.apply_output_layout(
                i,
                output.name.clone(),
                output.rect,
                output.scale,
                bh,
                hp,
                sm,
            );
            m
        }
    }
}

fn destroy_bars_for_removed_monitors(ctx: &mut WmCtx, pool: &mut [Option<Monitor>]) {
    for slot in pool.iter_mut() {
        if let Some(m) = slot.as_ref() {
            destroy_monitor_bar_x11(ctx, m.bar_win);
        }
    }
}

fn remap_client_monitor_ids(g: &mut Globals, old_to_new: &[Option<MonitorId>]) {
    for client in g.clients.values_mut() {
        let oi = client.monitor_id.index();
        if oi < old_to_new.len() {
            if let Some(nid) = old_to_new[oi] {
                client.monitor_id = nid;
            } else {
                client.monitor_id = MonitorId(0);
            }
        } else {
            client.monitor_id = MonitorId(0);
        }
    }
}

fn remap_selected_monitor_after_sync(
    sel_idx: MonitorId,
    old_to_new: &[Option<MonitorId>],
    new_len: usize,
) -> MonitorId {
    let old_sel_idx = sel_idx.index();
    let new_sel = if old_sel_idx < old_to_new.len() {
        old_to_new[old_sel_idx].unwrap_or(MonitorId(0))
    } else {
        MonitorId(0)
    };
    if new_sel.index() < new_len {
        new_sel
    } else {
        MonitorId(0)
    }
}

fn notify_monitor_layout_changed(ctx: &mut WmCtx, changed: bool) {
    if !changed {
        return;
    }
    ctx.core_mut().globals_mut().queue_layout_for_all_monitors();
    ctx.core_mut().bar.mark_dirty();
    if let Some(ptr) = ctx.pointer_location()
        && let Some(m) = ctx.core().globals().monitors.find_monitor_at_pointer(ptr)
    {
        ctx.core_mut().globals_mut().monitors.set_sel_idx(m);
    }
}

/// Match an existing monitor to this output: prefer stable output name, then Xinerama / slot
/// alignment for unnamed monitors.
fn take_matching_monitor(
    pool: &mut [Option<Monitor>],
    output_index: usize,
    output: &BackendOutputInfo,
) -> Option<(usize, Monitor)> {
    if !output.name.is_empty() {
        for j in 0..pool.len() {
            if let Some(m) = pool[j].as_ref()
                && m.name == output.name
            {
                let mon = pool[j].take().expect("checked");
                return Some((j, mon));
            }
        }
    }
    if output_index < pool.len()
        && let Some(m) = pool[output_index].as_ref()
    {
        let xin = output.name.starts_with("XINERAMA-");
        let slot_unlabeled = m.name.is_empty() && !output.name.is_empty();
        let both_empty = m.name.is_empty() && output.name.is_empty();
        if (xin && (m.name.is_empty() || m.name == output.name)) || slot_unlabeled || both_empty {
            let mon = pool[output_index].take().expect("checked");
            return Some((output_index, mon));
        }
    }
    None
}

/// Rebuilds the monitor list from backend outputs, preserving workspace state by **output name**
/// (with Xinerama / unnamed-slot fallbacks). Remaps `Client::monitor_id` when indices shift.
fn sync_monitors_from_outputs(ctx: &mut WmCtx, outputs: Vec<BackendOutputInfo>) -> bool {
    if outputs.is_empty() {
        return false;
    }

    let template = ctx.core().globals().cfg.tag_template.clone();
    let (showbar, topbar) = (
        ctx.core().globals().cfg.show_bar,
        ctx.core().globals().cfg.top_bar,
    );

    let (layout_width, layout_height) = output_layout_extent(&outputs);
    let mut changed = sync_runtime_screen_size(
        &mut ctx.core_mut().globals_mut().cfg,
        layout_width,
        layout_height,
    );

    let old_count = ctx.core().globals().monitors.len();
    let sel_idx = ctx.core().globals().monitors.selected_monitor_idx;

    if old_count != outputs.len() {
        changed = true;
    }

    let olds = std::mem::take(&mut ctx.core_mut().globals_mut().monitors.monitors);
    let mut old_to_new: Vec<Option<MonitorId>> = vec![None; olds.len()];
    let mut pool: Vec<Option<Monitor>> = olds.into_iter().map(Some).collect();

    let globals = ctx.core().globals();
    let mut new_monitors = Vec::with_capacity(outputs.len());
    for (i, output) in outputs.iter().enumerate() {
        let mon = make_monitor_for_output(
            i,
            output,
            &mut pool,
            &mut old_to_new,
            &template,
            showbar,
            topbar,
            globals,
            &mut changed,
        );
        new_monitors.push(mon);
    }

    destroy_bars_for_removed_monitors(ctx, &mut pool);

    for (i, m) in new_monitors.iter_mut().enumerate() {
        m.monitor_id = MonitorId(i);
    }
    ctx.core_mut().globals_mut().monitors.monitors = new_monitors;
    let new_len = ctx.core().globals().monitors.len();

    {
        let g = ctx.core_mut().globals_mut();
        remap_client_monitor_ids(g, &old_to_new);
        g.monitors.selected_monitor_idx =
            remap_selected_monitor_after_sync(sel_idx, &old_to_new, new_len);
    }

    notify_monitor_layout_changed(ctx, changed);
    changed
}

fn update_from_outputs(ctx: &mut WmCtx, outputs: Vec<BackendOutputInfo>) -> bool {
    sync_monitors_from_outputs(ctx, outputs)
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
    if !client.is_scratchpad() || client.is_sticky {
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

    let outputs: Vec<BackendOutputInfo> = unique
        .into_iter()
        .enumerate()
        .map(|(i, rect)| BackendOutputInfo {
            name: format!("XINERAMA-{i}"),
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        })
        .collect();

    Some(sync_monitors_from_outputs(
        &mut WmCtx::X11(x11.reborrow()),
        outputs,
    ))
}

pub fn reorder_client(ctx: &mut WmCtx, win: WindowId, direction: VerticalDirection) {
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
        .map(|c| c.mode.is_floating())
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();

    if let Some(mon) = ctx.core_mut().globals_mut().monitors.get_mut(selmon_id)
        && let Some(pos) = mon.clients.iter().position(|&w| w == win)
    {
        match direction {
            VerticalDirection::Up => {
                if pos > 0 {
                    mon.clients.swap(pos, pos - 1);
                } else {
                    let last = mon.clients.pop();
                    if let Some(last_win) = last {
                        mon.clients.insert(1, last_win);
                    }
                }
            }
            VerticalDirection::Down => {
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
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selmon_id);
}
