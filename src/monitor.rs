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
///
/// Each monitor is assigned a stable [`MonitorId`] when it is created. The id
/// persists across output hotplug and reordering, so references held by clients,
/// the current selection, and transient interaction state (drags, gestures) stay
/// valid without remapping. Spatial ordering is tracked separately and queried
/// via [`position_of`](Self::position_of) / [`id_at_position`](Self::id_at_position).
#[derive(Default)]
pub struct MonitorManager {
    monitors: Vec<Monitor>,
    next_id: u64,
    selected: MonitorId,
}

impl MonitorManager {
    pub fn new() -> Self {
        Self::default()
    }

    // -------------------------------------------------------------------------
    // Selection
    // -------------------------------------------------------------------------

    pub fn selected(&self) -> MonitorId {
        self.selected
    }

    pub fn set_selected(&mut self, id: MonitorId) {
        if self.contains(id) {
            self.selected = id;
        }
    }

    // -------------------------------------------------------------------------
    // Lookup by stable id
    // -------------------------------------------------------------------------

    pub fn get(&self, id: MonitorId) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.monitor_id == id)
    }

    pub fn get_mut(&mut self, id: MonitorId) -> Option<&mut Monitor> {
        self.monitors.iter_mut().find(|m| m.monitor_id == id)
    }

    pub fn contains(&self, id: MonitorId) -> bool {
        self.monitors.iter().any(|m| m.monitor_id == id)
    }

    pub fn selected_monitor(&self) -> Option<&Monitor> {
        self.get(self.selected)
    }

    pub fn selected_monitor_unchecked(&self) -> &Monitor {
        self.get(self.selected).expect("no monitors")
    }

    pub fn selected_monitor_mut(&mut self) -> Option<&mut Monitor> {
        self.get_mut(self.selected)
    }

    pub fn selected_monitor_mut_unchecked(&mut self) -> &mut Monitor {
        self.get_mut(self.selected).expect("no monitors")
    }

    // -------------------------------------------------------------------------
    // Spatial position (distinct from identity)
    // -------------------------------------------------------------------------

    /// Return the 0-based spatial position of `id` in the display order.
    pub fn position_of(&self, id: MonitorId) -> Option<usize> {
        self.monitors.iter().position(|m| m.monitor_id == id)
    }

    /// Return the [`MonitorId`] at spatial position `pos`, if any.
    pub fn id_at_position(&self, pos: usize) -> Option<MonitorId> {
        self.monitors.get(pos).map(|m| m.monitor_id)
    }

    /// Return the id of the first monitor in display order.
    pub fn first(&self) -> Option<MonitorId> {
        self.monitors.first().map(|m| m.monitor_id)
    }

    // -------------------------------------------------------------------------
    // Sizing
    // -------------------------------------------------------------------------

    pub fn len(&self) -> usize {
        self.monitors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    pub fn clear(&mut self) {
        self.monitors.clear();
        self.selected = MonitorId::default();
    }

    // -------------------------------------------------------------------------
    // Iteration (spatial order)
    // -------------------------------------------------------------------------

    pub fn iter(&self) -> impl Iterator<Item = (MonitorId, &Monitor)> {
        self.monitors.iter().map(|m| (m.monitor_id, m))
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &Monitor> {
        self.monitors.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (MonitorId, &mut Monitor)> {
        self.monitors.iter_mut().map(|m| (m.monitor_id, m))
    }

    pub fn iter_all_mut(&mut self) -> impl Iterator<Item = &mut Monitor> {
        self.monitors.iter_mut()
    }

    /// Read-only access to all monitors as a slice, in spatial order.
    pub fn as_slice(&self) -> &[Monitor] {
        &self.monitors
    }

    // -------------------------------------------------------------------------
    // Insertion
    // -------------------------------------------------------------------------

    /// Insert a monitor, assigning it a fresh stable [`MonitorId`].
    ///
    /// If this is the first monitor, it becomes the selected monitor.
    pub fn push(&mut self, mut m: Monitor) -> MonitorId {
        let id = self.alloc_id();
        m.monitor_id = id;
        let was_empty = self.monitors.is_empty();
        self.monitors.push(m);
        if was_empty {
            self.selected = id;
        }
        id
    }

    fn alloc_id(&mut self) -> MonitorId {
        let id = MonitorId::from_raw(self.next_id);
        self.next_id += 1;
        id
    }

    /// Drain all monitors out, returning them in spatial order. The id counter
    /// and selection are preserved. Used by `sync_monitors_from_outputs` to
    /// rebuild the list while keeping id allocation monotonic.
    pub(crate) fn drain(&mut self) -> Vec<Monitor> {
        std::mem::take(&mut self.monitors)
    }

    /// Restore a rebuilt monitor list. Each monitor must already carry its
    /// stable `monitor_id` (reused for matched monitors, freshly allocated for
    /// new ones). The selection is preserved if its monitor is still present,
    /// otherwise falls back to the first monitor.
    pub(crate) fn restore(&mut self, monitors: Vec<Monitor>) {
        self.monitors = monitors;
        if !self.contains(self.selected) {
            self.selected = self.first().unwrap_or_default();
        }
    }

    /// Allocate a fresh stable id without inserting a monitor.
    pub(crate) fn allocate_id(&mut self) -> MonitorId {
        self.alloc_id()
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
        crate::types::find_monitor_by_rect(self.iter(), rect).or(Some(self.selected))
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

pub fn transfer_client(ctx: &mut WmCtx, win: WindowId, target_mon: MonitorId) {
    let current_mon = ctx.core().model().clients.get(&win).map(|c| c.monitor_id);
    if current_mon == Some(target_mon) {
        return;
    }

    if ctx.core_mut().model_mut().clients.contains_key(&win) {
        unfocus_win(ctx, win, true);
    }

    let Some(outcome) = ctx
        .core_mut()
        .model_mut()
        .move_client_to_monitor(win, target_mon)
    else {
        return;
    };
    if let WmCtx::X11(x11) = ctx {
        crate::backend::x11::set_client_tag_prop(x11.core.state(), &x11.x11, x11.x11_runtime, win);
    }

    focus(ctx, None);

    // Refresh the two monitors whose client sets changed. Floating transfers do
    // not arrange (`move_client_to_monitor` sets `needs_arrange = false`), so
    // this unconditional refresh is what actually updates the bar/geometry for
    // moved floating clients; callers must not assume the queue below covers it.
    if let Some(src) = current_mon
        && src != target_mon
    {
        ctx.core_mut().queue_layout_for_monitor_urgent(src);
    }
    ctx.core_mut().queue_layout_for_monitor_urgent(target_mon);

    if outcome.needs_arrange {
        ctx.core_mut().queue_layout_for_all_monitors_urgent();
    }

    if outcome.is_scratchpad {
        handle_scratchpad_transfer(ctx, win, target_mon);
    }
}

pub fn focus_monitor(ctx: &mut WmCtx, direction: MonitorDirection) {
    let target = {
        let mgr = &ctx.core().model().monitors;
        if mgr.len() <= 1 {
            return;
        }
        match find_monitor_by_direction(mgr.iter(), mgr.selected(), direction) {
            Some(id) => id,
            None => return,
        }
    };

    if target == ctx.core().model().monitors.selected() {
        return;
    }

    if let Some(win) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .selected_monitor()
        .and_then(|m| m.selected)
    {
        unfocus_win(ctx, win, false);
    }

    ctx.core_mut().model_mut().monitors.set_selected(target);
    focus(ctx, None);
}

pub fn focus_n_mon(ctx: &mut WmCtx, position: usize) {
    let target = {
        let mgr = &ctx.core().model().monitors;
        if mgr.len() <= 1 {
            return;
        }
        match mgr.id_at_position(position.min(mgr.len() - 1)) {
            Some(id) => id,
            None => return,
        }
    };

    if let Some(win) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .selected_monitor()
        .and_then(|m| m.selected)
    {
        unfocus_win(ctx, win, false);
    }

    ctx.core_mut().model_mut().monitors.set_selected(target);
    focus(ctx, None);
}

pub fn move_to_monitor_and_follow(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .selected_monitor()
        .and_then(|m| m.selected)
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
            .set_selected(monitor_id);
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

fn apply_output_to_monitor(
    m: &mut Monitor,
    position: usize,
    output: &BackendOutputInfo,
    bh: i32,
    hp: i32,
    sm: i32,
) {
    m.apply_output_layout(
        position,
        output.name.clone(),
        output.rect,
        output.scale,
        bh,
        hp,
        sm,
    );
}

fn output_geom_changed(m: &Monitor, output: &BackendOutputInfo, bh: i32, hp: i32, sm: i32) -> bool {
    m.monitor_rect != output.rect
        || m.name != output.name
        || (m.ui_scale - output.scale).abs() > f64::EPSILON
        || m.bar_height != bh
        || m.horizontal_padding != hp
        || m.startmenu_size != sm
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
        ctx.core_mut().model_mut().monitors.set_selected(m);
    }
}

/// Match an existing monitor to this output: prefer stable output name, then
/// Xinerama / slot alignment for unnamed monitors. `position` is the spatial
/// index of the output (used only for the same-slot fallback).
fn take_matching_monitor(
    pool: &mut [Option<Monitor>],
    position: usize,
    output: &BackendOutputInfo,
) -> Option<Monitor> {
    if !output.name.is_empty()
        && let Some((_, slot)) = pool
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.as_ref().is_some_and(|m| m.name == output.name))
    {
        return Some(slot.take().unwrap());
    }
    if let Some(slot) = pool.get_mut(position)
        && let Some(m) = slot.as_ref()
    {
        let xin = output.name.starts_with("XINERAMA-");
        let slot_unlabeled = m.name.is_empty() && !output.name.is_empty();
        let both_empty = m.name.is_empty() && output.name.is_empty();
        if (xin && (m.name.is_empty() || m.name == output.name)) || slot_unlabeled || both_empty {
            return Some(slot.take().unwrap());
        }
    }
    None
}

/// Move clients whose monitor has disappeared onto a surviving monitor,
/// updating both ownership and per-monitor membership lists.
fn rehome_orphaned_clients(model: &mut crate::model::WmModel, survivor: MonitorId) {
    let stale_wins: Vec<WindowId> = model
        .clients
        .values()
        .filter(|c| !model.monitors.contains(c.monitor_id))
        .map(|c| c.win)
        .collect();

    for win in stale_wins {
        model.detach(win);
        model.detach_z_order(win);
        if let Some(client) = model.clients.get_mut(&win) {
            client.monitor_id = survivor;
        }
        model.attach(win);
        model.attach_z_order_top(win);
    }
}

/// Rebuilds the monitor list from backend outputs.
///
/// Matched monitors **keep their stable `MonitorId`** (keyed by output name,
/// with Xinerama / unnamed-slot fallbacks), so clients, the selection, and any
/// captured ids stay valid without remapping. Genuinely removed monitors have
/// their clients re-homed onto a survivor. Brand-new outputs get a fresh id.
fn sync_monitors_from_outputs(ctx: &mut WmCtx, outputs: Vec<BackendOutputInfo>) -> bool {
    if outputs.is_empty() {
        return false;
    }

    let template = ctx.core().config().tag_template.clone();
    let (show_bar, top_bar) = (ctx.core().config().bar.show, ctx.core().config().bar.top);

    let (layout_width, layout_height) = output_layout_extent(&outputs);
    let mut changed =
        sync_runtime_screen_size(ctx.core_mut().config_mut(), layout_width, layout_height);

    // Pre-compute per-output UI metrics while we hold an immutable config borrow.
    let metrics: Vec<(i32, i32, i32)> = outputs
        .iter()
        .map(|o| scaled_monitor_ui_metrics(ctx.core().config(), o.scale))
        .collect();

    let old_count = ctx.core().model().monitors.len();
    if old_count != outputs.len() {
        changed = true;
    }

    // Drain old monitors into a pool. They keep their stable ids + workspace
    // state; matched ones are reused, the rest are dropped after the rebuild.
    let old_monitors = ctx.core_mut().state_mut().model.monitors.drain();
    let mut pool: Vec<Option<Monitor>> = old_monitors.into_iter().map(Some).collect();

    let mut new_monitors = Vec::with_capacity(outputs.len());
    for (i, output) in outputs.iter().enumerate() {
        let (bh, hp, sm) = metrics[i];
        match take_matching_monitor(&mut pool, i, output) {
            Some(mut m) => {
                if output_geom_changed(&m, output, bh, hp, sm) {
                    changed = true;
                }
                // Keep the reused monitor's stable id and workspace state.
                apply_output_to_monitor(&mut m, i, output, bh, hp, sm);
                new_monitors.push(m);
            }
            None => {
                changed = true;
                let id = ctx.core_mut().state_mut().model.monitors.allocate_id();
                let mut m = Monitor::new_with_values(show_bar, top_bar);
                m.monitor_id = id;
                m.init_tags(&template);
                apply_output_to_monitor(&mut m, i, output, bh, hp, sm);
                new_monitors.push(m);
            }
        }
    }

    // Destroy orphaned monitors' bar windows.
    for slot in &mut pool {
        if let Some(m) = slot.as_ref() {
            crate::backend::x11::monitor_helpers::destroy_monitor_bar_x11(ctx, m.bar_win);
        }
    }

    // Restore the rebuilt list. The selection is preserved if its monitor still
    // exists; otherwise the manager falls back to the first monitor.
    ctx.core_mut()
        .state_mut()
        .model
        .monitors
        .restore(new_monitors);

    // Re-home any clients whose monitor was removed onto the first survivor.
    if let Some(survivor) = ctx.core().model().monitors.first() {
        rehome_orphaned_clients(&mut ctx.core_mut().state_mut().model, survivor);
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
    let current_mon = ctx.core_mut().model_mut().monitors.selected();

    if let Some(selected_window) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .get(current_mon)
        .and_then(|m| m.selected)
    {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.core_mut()
        .state_mut()
        .model
        .monitors
        .set_selected(target_mon);

    let _ = crate::floating::scratchpad_show_name(ctx, &sp_name);

    if let Some(selected_window) = ctx
        .core_mut()
        .state_mut()
        .model
        .monitors
        .get(target_mon)
        .and_then(|m| m.selected)
    {
        unfocus_win(ctx, selected_window, false);
    }
    ctx.core_mut()
        .state_mut()
        .model
        .monitors
        .set_selected(current_mon);

    focus(ctx, None);
}

fn init_single_monitor(ctx: &mut WmCtx, sw: i32, h: i32) -> bool {
    let template = ctx.core_mut().config_mut().tag_template.clone();
    let mut mon = Monitor::new_with_values(
        ctx.core_mut().config_mut().bar.show,
        ctx.core_mut().config_mut().bar.top,
    );
    mon.init_tags(&template);
    let id = ctx.core_mut().model_mut().monitors.push(mon);
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().config(), 1.0);
    if let Some(m) = ctx.core_mut().model_mut().monitors.get_mut(id) {
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
    ctx.core_mut().state_mut().model.monitors.set_selected(id);
    true
}

fn update_single_monitor(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let first_id = match ctx.core().state().model.monitors.first() {
        Some(id) => id,
        None => return false,
    };
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().config(), 1.0);
    let needs_update = ctx
        .core()
        .state()
        .model
        .monitors
        .get(first_id)
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

    if let Some(m) = ctx.core_mut().model_mut().monitors.get_mut(first_id) {
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
