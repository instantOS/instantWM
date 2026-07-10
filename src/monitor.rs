//! Monitor management via the `MonitorManager` struct.
//!
//! This module encapsulates monitor state and logic, providing a clean API
//! for monitor-related operations.

use crate::backend::BackendOutputInfo;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::core_state::RuntimeConfig;
use crate::focus::{focus, unfocus_win};
use crate::types::*;
use std::collections::HashMap;

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

    pub fn find_monitor_at_pointer(&self, ptr: Point) -> Option<MonitorId> {
        let rect = Rect {
            x: ptr.x,
            y: ptr.y,
            w: 1,
            h: 1,
        };
        self.find_id_by_rect(&rect)
    }
}

// -----------------------------------------------------------------------------
// Orchestration Logic (Free functions that coordinate multiple managers)
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct ClientTransferOutcome {
    is_scratchpad: bool,
    needs_arrange: bool,
}

fn transfer_client_model(
    model: &mut crate::model::WmModel,
    win: WindowId,
    target_mon: MonitorId,
) -> Option<ClientTransferOutcome> {
    let client = model.clients.get(&win)?;
    let is_scratchpad = client.is_scratchpad();
    let target_tags = if is_scratchpad {
        crate::types::TagMask::EMPTY
    } else {
        model
            .monitors
            .get(target_mon)
            .map(|m| m.selected_tags())
            .unwrap_or(crate::types::TagMask::single(1).unwrap_or(crate::types::TagMask::EMPTY))
    };
    let target_tag_idx = model
        .monitors
        .get(target_mon)
        .and_then(|m| m.current_tag_number());

    model.detach(win);
    model.detach_z_order(win);
    let client = model.clients.get_mut(&win)?;
    client.monitor_id = target_mon;
    if !is_scratchpad {
        client.set_tag_mask(target_tags);
        client.reset_sticky(target_tag_idx);
    }
    let needs_arrange = !client.mode.is_floating();
    model.attach(win);
    model.attach_z_order_top(win);
    Some(ClientTransferOutcome {
        is_scratchpad,
        needs_arrange,
    })
}

pub fn transfer_client(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    if ctx.core_mut().model_mut().monitors.sel_idx() == target_mon {
        return;
    }

    if ctx.core_mut().model_mut().clients.contains_key(&win) {
        unfocus_win(ctx, win, true);
    }

    let Some(outcome) = transfer_client_model(ctx.core_mut().model_mut(), win, target_mon) else {
        return;
    };
    if let WmCtx::X11(x11) = ctx {
        crate::backend::x11::set_client_tag_prop(x11.core.state(), &x11.x11, x11.x11_runtime, win);
    }

    focus(ctx, None);

    if outcome.needs_arrange {
        ctx.core_mut().queue_layout_for_all_monitors_urgent();
    }

    if outcome.is_scratchpad {
        handle_scratchpad_transfer(ctx, win, target_mon);
    }
}

pub fn focus_monitor(ctx: &mut WmCtx, direction: MonitorDirection) {
    let target = {
        let mgr = &ctx.core_mut().model_mut().monitors;
        if mgr.monitors.len() <= 1 {
            return;
        }
        match find_monitor_by_direction(&mgr.monitors, mgr.selected_monitor_idx, direction) {
            Some(id) => id,
            None => return,
        }
    };

    if target == ctx.core_mut().model_mut().monitors.sel_idx() {
        return;
    }

    if let Some(win) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .sel()
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, win, false);
    }

    ctx.core_mut().model_mut().monitors.set_sel_idx(target);
    focus(ctx, None);
}

pub fn focus_n_mon(ctx: &mut WmCtx, index: MonitorId) {
    let target = {
        let mgr = &ctx.core_mut().model_mut().monitors;
        if mgr.monitors.len() <= 1 {
            return;
        }
        MonitorId(index.index().min(mgr.monitors.len() - 1))
    };

    if let Some(win) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .sel()
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, win, false);
    }

    ctx.core_mut().model_mut().monitors.set_sel_idx(target);
    focus(ctx, None);
}

pub fn move_to_monitor_and_follow(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .sel()
        .and_then(|m| m.sel)
    {
        Some(w) => w,
        None => return,
    };

    crate::tags::send_to_monitor(ctx, direction);

    if let Some(monitor_id) = ctx.core().model().clients.monitor_id(c_win) {
        ctx.core_mut()
            .state_mut()
            .model
            .monitors
            .set_sel_idx(monitor_id);
    }

    focus(ctx, Some(c_win));

    ctx.window_backend().raise_window_visual_only(c_win);
    ctx.warp_cursor_to_client(c_win);
}

pub fn apply_monitor_config(ctx: &mut WmCtx) {
    let monitors_cfg = ctx.core().config().monitors.clone();

    // Apply wildcard first as fallback
    if let Some(wildcard_cfg) = monitors_cfg.get("*") {
        ctx.output_backend().set_monitor_config("*", wildcard_cfg);
    }

    // Apply specific configs
    for (name, config) in monitors_cfg {
        if name != "*" {
            ctx.output_backend().set_monitor_config(&name, &config);
        }
    }

    refresh_monitor_layout(ctx);
}

pub fn refresh_monitor_layout(ctx: &mut WmCtx) -> bool {
    // Try the backend's get_outputs first (uses XRandR on X11, native on Wayland)
    let outputs = ctx.output_backend().get_outputs();
    if outputs.len() > 1 || (outputs.len() == 1 && outputs[0].name != "X11") {
        return sync_monitors_from_outputs(ctx, outputs);
    }

    // Fall back to Xinerama for X11
    if let WmCtx::X11(x11) = ctx
        && let Some(result) = update_from_xinerama(x11)
    {
        return result;
    }

    // Final fallback to single monitor
    let sw = ctx
        .core_mut()
        .state_mut()
        .config
        .derived
        .display
        .width
        .max(1);
    let sh = ctx
        .core_mut()
        .state_mut()
        .config
        .derived
        .display
        .height
        .max(1);

    if ctx.core_mut().model_mut().monitors.is_empty() {
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
    if cfg.derived.display.width != layout_width || cfg.derived.display.height != layout_height {
        cfg.derived.display.width = layout_width;
        cfg.derived.display.height = layout_height;
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
    config: &RuntimeConfig,
    changed: &mut bool,
) -> Monitor {
    let (bh, hp, sm) = scaled_monitor_ui_metrics(config, output.scale);
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
            crate::backend::x11::monitor_helpers::destroy_monitor_bar_x11(ctx, m.bar_win);
        }
    }
}

fn remap_client_monitor_ids(model: &mut crate::model::WmModel, old_to_new: &[Option<MonitorId>]) {
    for client in model.clients.values_mut() {
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
    ctx.core_mut().queue_layout_for_all_monitors();
    ctx.core_mut().bar.mark_dirty();
    if let Some(ptr) = ctx.pointer_backend().pointer_location()
        && let Some(m) = ctx.core().model().monitors.find_monitor_at_pointer(ptr)
    {
        ctx.core_mut().model_mut().monitors.set_sel_idx(m);
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

    let template = ctx.core().config().tag_template.clone();
    let (showbar, topbar) = (ctx.core().config().bar.show, ctx.core().config().bar.top);

    let (layout_width, layout_height) = output_layout_extent(&outputs);
    let mut changed = sync_runtime_screen_size(
        &mut ctx.core_mut().config_mut(),
        layout_width,
        layout_height,
    );

    let old_count = ctx.core().model().monitors.len();
    let sel_idx = ctx.core().model().monitors.selected_monitor_idx;

    if old_count != outputs.len() {
        changed = true;
    }

    let olds = std::mem::take(&mut ctx.core_mut().model_mut().monitors.monitors);
    let mut old_to_new: Vec<Option<MonitorId>> = vec![None; olds.len()];
    let mut pool: Vec<Option<Monitor>> = olds.into_iter().map(Some).collect();

    let globals = ctx.core().state();
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
            &globals.config,
            &mut changed,
        );
        new_monitors.push(mon);
    }

    destroy_bars_for_removed_monitors(ctx, &mut pool);

    for (i, m) in new_monitors.iter_mut().enumerate() {
        m.monitor_id = MonitorId(i);
    }
    ctx.core_mut().model_mut().monitors.monitors = new_monitors;
    let new_len = ctx.core().model().monitors.len();

    {
        let g = ctx.core_mut().state_mut();
        remap_client_monitor_ids(&mut g.model, &old_to_new);
        g.model.monitors.selected_monitor_idx =
            remap_selected_monitor_after_sync(sel_idx, &old_to_new, new_len);
    }

    notify_monitor_layout_changed(ctx, changed);
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

fn scaled_monitor_ui_metrics(config: &RuntimeConfig, scale: f64) -> (i32, i32, i32) {
    (
        scaled_i32(config.derived.bar_height, scale).max(1),
        scaled_i32(config.derived.bar_horizontal_padding, scale).max(1),
        scaled_i32(config.bar.startmenu_size, scale).max(1),
    )
}

// -----------------------------------------------------------------------------
// Internal Helpers
// -----------------------------------------------------------------------------

fn handle_scratchpad_transfer(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    let Some(client) = ctx.core().model().clients.get(&win) else {
        return;
    };
    if !client.is_scratchpad() || client.is_sticky {
        return;
    }

    let sp_name = client.scratchpad.as_ref().unwrap().name.clone();
    let current_mon = ctx.core_mut().model_mut().monitors.sel_idx();

    if let Some(selected_window) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .get(current_mon)
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.core_mut()
        .state_mut()
        .model
        .monitors
        .set_sel_idx(target_mon);

    let _ = crate::floating::scratchpad_show_name(ctx, &sp_name);

    if let Some(selected_window) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .get(target_mon)
        .and_then(|m| m.sel)
    {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.core_mut()
        .state_mut()
        .model
        .monitors
        .set_sel_idx(current_mon);

    focus(ctx, None);
}

fn init_single_monitor(ctx: &mut WmCtx, sw: i32, h: i32) -> bool {
    let template = ctx.core_mut().config_mut().tag_template.clone();
    let mut mon = Monitor::new_with_values(
        ctx.core_mut().config_mut().bar.show,
        ctx.core_mut().config_mut().bar.top,
    );
    mon.init_tags(&template);
    ctx.core_mut().model_mut().monitors.push(mon);
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().config(), 1.0);
    if let Some(m) = ctx.core_mut().model_mut().monitors.get_mut(MonitorId(0)) {
        m.num = 0;
        let rect = Rect {
            x: 0,
            y: 0,
            w: sw,
            h,
        };
        m.monitor_rect = rect;
        m.available_rect = rect;
        m.work_rect = rect;
        m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
        m.update_bar_position(bar_height);
    }
    ctx.core_mut()
        .state_mut()
        .model
        .monitors
        .set_sel_idx(MonitorId(0));
    true
}

fn update_single_monitor(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().config(), 1.0);
    let needs_update = ctx
        .core()
        .state()
        .model
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

    if let Some(m) = ctx.core_mut().model_mut().monitors.get_mut(MonitorId(0)) {
        m.monitor_rect.w = sw;
        m.monitor_rect.h = sh;
        m.available_rect = m.monitor_rect;
        m.work_rect.w = sw;
        m.work_rect.h = sh;
        m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
        m.update_bar_position(bar_height);
    }
    true
}

fn update_from_xinerama(x11: &mut WmCtxX11) -> Option<bool> {
    let outputs = crate::backend::x11::monitor_helpers::xinerama_outputs(&x11.x11)?;
    Some(sync_monitors_from_outputs(
        &mut WmCtx::X11(x11.reborrow()),
        outputs,
    ))
}

pub fn reorder_client(ctx: &mut WmCtx, win: WindowId, direction: VerticalDirection) {
    let tiled_count = {
        let g = ctx.core_mut().state_mut();
        g.selected_monitor()
            .tiled_client_count(g.model.clients.map())
    };
    if tiled_count < 2 {
        return;
    }

    let is_floating = ctx
        .core()
        .state()
        .model
        .clients
        .get(&win)
        .map(|c| c.mode.is_floating())
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();

    if let Some(mon) = ctx.core_mut().model_mut().monitors.get_mut(selmon_id)
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

    focus(ctx, Some(win));
    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_id);
}
