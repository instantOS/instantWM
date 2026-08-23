//! Monitor management via the `MonitorManager` struct.
//!
//! This module encapsulates monitor state and logic, providing a clean API
//! for monitor-related operations.

use crate::backend::BackendOutputInfo;
use crate::contexts::WmCtx;
use crate::core_state::{DerivedState, EffectiveConfig};
use crate::focus::refresh_focus_after_selection;
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
            if w == m.bar_win || w == m.bottom_bar_win {
                return Some(i);
            }
        }

        if let Some(c) = clients.get(&w) {
            return self.contains(c.monitor_id).then_some(c.monitor_id);
        }

        None
    }

    /// Find the monitor with the largest intersection with `rect`.
    pub fn id_intersecting_rect(&self, rect: Rect) -> Option<MonitorId> {
        let mut best = None;
        let mut max_area = 0;
        for (id, monitor) in self.iter() {
            let area = monitor
                .monitor_rect
                .intersection(&rect)
                .map_or(0, |intersection| intersection.area());
            if area > max_area {
                max_area = area;
                best = Some(id);
            }
        }
        best
    }

    /// Find the adjacent monitor in spatial order, wrapping at either end.
    pub fn id_in_direction(
        &self,
        current: MonitorId,
        direction: MonitorDirection,
    ) -> Option<MonitorId> {
        let current_position = self.position_of(current)?;
        let target_position = if direction.is_next() {
            (current_position + 1) % self.len()
        } else if current_position == 0 {
            self.len().checked_sub(1)?
        } else {
            current_position - 1
        };
        self.id_at_position(target_position)
    }

    pub fn find_id_by_rect(&self, rect: &Rect) -> Option<MonitorId> {
        self.id_intersecting_rect(*rect)
            .or_else(|| self.selected_monitor().map(Monitor::id))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferFocus {
    /// Keep keyboard focus on the currently selected monitor. If the moved
    /// client was focused, select and focus a replacement there.
    Preserve,
    /// Select the destination monitor and focus the transferred client.
    FollowWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferFocusEffect {
    None,
    FocusSourceReplacement {
        moved_window: WindowId,
    },
    FocusTransferredWindow {
        previous_focus: Option<WindowId>,
        target_monitor: MonitorId,
        moved_window: WindowId,
    },
}

fn transfer_focus_effect(
    policy: TransferFocus,
    selected_monitor_before: MonitorId,
    focused_before: Option<WindowId>,
    outcome: crate::model::ClientTransferOutcome,
    moved_window: WindowId,
) -> TransferFocusEffect {
    match policy {
        TransferFocus::Preserve
            if selected_monitor_before == outcome.source_monitor && outcome.was_selected =>
        {
            TransferFocusEffect::FocusSourceReplacement { moved_window }
        }
        TransferFocus::Preserve => TransferFocusEffect::None,
        TransferFocus::FollowWindow => TransferFocusEffect::FocusTransferredWindow {
            previous_focus: focused_before,
            target_monitor: outcome.target_monitor,
            moved_window,
        },
    }
}

/// Transfer a managed client and complete all related focus and layout work as
/// one transaction.
///
/// Callers choose the focus policy up front instead of repairing focus after
/// the transfer. This is important on X11, where mutating the model before a
/// normal `focus()` call can make the requested window appear already focused
/// even though the backend still points at the old window.
pub fn transfer_client(
    ctx: &mut WmCtx,
    win: WindowId,
    target_mon: MonitorId,
    focus_policy: TransferFocus,
) -> Option<crate::model::ClientTransferOutcome> {
    let selected_monitor_before = ctx.core().model().selected_monitor_id();
    let focused_before = ctx.core().model().selected_win();
    let outcome = ctx
        .core_mut()
        .mutate_selection(|model| model.move_client_to_monitor(win, target_mon));
    let outcome = outcome?;

    ctx.sync_client_tag_props(win);

    match transfer_focus_effect(
        focus_policy,
        selected_monitor_before,
        focused_before,
        outcome,
        win,
    ) {
        TransferFocusEffect::None => {}
        TransferFocusEffect::FocusSourceReplacement { .. } => {
            refresh_focus_after_selection(ctx, focused_before, None);
        }
        TransferFocusEffect::FocusTransferredWindow {
            previous_focus,
            target_monitor,
            moved_window,
        } => {
            ctx.core_mut().select_monitor(target_monitor);
            ctx.core_mut()
                .select_on_monitor(target_monitor, Some(moved_window));
            ctx.update_ewmh_desktop_props();
            refresh_focus_after_selection(ctx, previous_focus, Some(moved_window));
        }
    }

    // Refresh the two monitors whose client sets changed. Floating transfers do
    // not arrange (`move_client_to_monitor` sets `needs_arrange = false`), so
    // this unconditional refresh is what actually updates the bar/geometry for
    // moved floating clients; callers must not assume the queue below covers it.
    ctx.core_mut()
        .queue_layout_for_monitor_urgent(outcome.source_monitor);
    ctx.core_mut()
        .queue_layout_for_monitor_urgent(outcome.target_monitor);

    if outcome.needs_arrange {
        ctx.core_mut().queue_layout_for_all_monitors_urgent();
    }

    if outcome.is_scratchpad {
        crate::floating::scratchpad::show_transferred_scratchpad(ctx, win, outcome.target_monitor);
    }

    Some(outcome)
}

pub fn focus_monitor(ctx: &mut WmCtx, direction: MonitorDirection) {
    let target = {
        let mgr = &ctx.core().model().monitors;
        if mgr.len() <= 1 {
            return;
        }
        match mgr.id_in_direction(mgr.selected(), direction) {
            Some(id) => id,
            None => return,
        }
    };

    crate::focus::select_monitor(ctx, target);
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

    crate::focus::select_monitor(ctx, target);
}

pub fn move_to_monitor_and_follow(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx.core().model().selected_win() {
        Some(w) => w,
        None => return,
    };

    crate::tags::send_to_monitor(ctx, direction);

    let previous_focus = ctx.core().model().selected_win();
    if let Some(monitor_id) = ctx
        .core()
        .model()
        .client(c_win)
        .map(|client| client.monitor_id)
    {
        ctx.core_mut().select_monitor(monitor_id);
    }

    refresh_focus_after_selection(ctx, previous_focus, Some(c_win));

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
    // Try the backend's primary output discovery first (XRandR on X11,
    // native protocol state on Wayland).
    let outputs = ctx.output_backend().get_outputs();
    if outputs.len() > 1 || (outputs.len() == 1 && outputs[0].name != "X11") {
        return sync_monitors_from_outputs(ctx, outputs);
    }

    // Legacy fallback discovery (Xinerama on X11; None elsewhere).
    if let Some(outputs) = ctx.output_backend().query_fallback_outputs() {
        return sync_monitors_from_outputs(ctx, outputs);
    }

    // Final fallback to single monitor
    let sw = ctx.core_mut().state_mut().derived.display.width.max(1);
    let sh = ctx.core_mut().state_mut().derived.display.height.max(1);

    if ctx.core_mut().model_mut().monitors.is_empty() {
        init_single_monitor(ctx, sw, sh)
    } else {
        update_single_monitor(ctx, sw, sh)
    }
}

fn output_layout_extent(outputs: &[BackendOutputInfo]) -> Size {
    let width = outputs
        .iter()
        .map(|o| o.rect.x.saturating_add(o.rect.w))
        .max()
        .unwrap_or(1)
        .max(1);
    let height = outputs
        .iter()
        .map(|o| o.rect.y.saturating_add(o.rect.h))
        .max()
        .unwrap_or(1)
        .max(1);
    Size::new(width, height)
}

fn sync_runtime_screen_size(derived: &mut DerivedState, layout_size: Size) -> bool {
    if derived.display.width != layout_size.w || derived.display.height != layout_size.h {
        derived.display.width = layout_size.w;
        derived.display.height = layout_size.h;
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
        ctx.core_mut().select_monitor(m);
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
        let reassigned = model.reassign_client_monitor(win, survivor);
        debug_assert!(reassigned, "orphaned managed client must be re-homeable");
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
    let previous_focus = ctx.core().model().selected_win();

    let template = ctx.core().config().tag_template.clone();
    let show_bar = ctx.core().config().bar.show;
    let show_bottom_bar = ctx.core().config().bar.show_bottom;

    let layout_size = output_layout_extent(&outputs);
    let mut changed = sync_runtime_screen_size(ctx.core_mut().derived_mut(), layout_size);

    // Pre-compute per-output UI metrics while we hold an immutable config borrow.
    let metrics: Vec<(i32, i32, i32)> = outputs
        .iter()
        .map(|o| scaled_monitor_ui_metrics(ctx.core().config(), ctx.core().derived(), o.scale))
        .collect();

    let reconciliation = ctx.core_mut().mutate_selection(|model| {
        reconcile_monitor_model(
            model,
            &outputs,
            &metrics,
            &template,
            show_bar,
            show_bottom_bar,
        )
    });
    changed |= reconciliation.changed;

    for bar_win in reconciliation.removed_bar_windows {
        ctx.destroy_monitor_bar_window(bar_win);
    }

    notify_monitor_layout_changed(ctx, changed);
    if ctx.core().model().selected_win() != previous_focus {
        refresh_focus_after_selection(ctx, previous_focus, None);
    }
    changed
}

#[derive(Debug)]
struct MonitorReconciliation {
    changed: bool,
    removed_bar_windows: Vec<WindowId>,
}

/// Reconcile backend output descriptions with the authoritative monitor graph.
///
/// This operation owns stable-ID reuse, new-monitor construction, and client
/// rehoming. It returns backend resources that the orchestration layer must
/// destroy rather than performing backend I/O while mutating the model.
fn reconcile_monitor_model(
    model: &mut crate::model::WmModel,
    outputs: &[BackendOutputInfo],
    metrics: &[(i32, i32, i32)],
    tag_template: &[crate::types::monitor::TagNames],
    show_bar: bool,
    show_bottom_bar: bool,
) -> MonitorReconciliation {
    debug_assert_eq!(outputs.len(), metrics.len());
    let mut changed = model.monitors.len() != outputs.len();

    // Drain old monitors into a pool. They keep their stable ids + workspace
    // state; matched ones are reused, the rest are dropped after the rebuild.
    let old_monitors = model.monitors.drain();
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
                let id = model.monitors.allocate_id();
                let mut m = Monitor::new_with_values(show_bar);
                m.show_bottom_bar = show_bottom_bar;
                m.monitor_id = id;
                m.init_tags(tag_template);
                apply_output_to_monitor(&mut m, i, output, bh, hp, sm);
                new_monitors.push(m);
            }
        }
    }

    // Collect the orphaned monitors' bar windows as cleanup work for the
    // caller. The default id is the "no bar" placeholder, not a real window.
    let removed_bar_windows = pool
        .into_iter()
        .flatten()
        .flat_map(|monitor| [monitor.bar_win, monitor.bottom_bar_win])
        .filter(|window| *window != WindowId::default())
        .collect();

    // Restore the rebuilt list. The selection is preserved if its monitor still
    // exists; otherwise the manager falls back to the first monitor.
    model.monitors.restore(new_monitors);

    // Re-home any clients whose monitor was removed onto the first survivor.
    if let Some(survivor) = model.monitors.first() {
        rehome_orphaned_clients(model, survivor);
    }

    MonitorReconciliation {
        changed,
        removed_bar_windows,
    }
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

fn scaled_monitor_ui_metrics(
    config: &EffectiveConfig,
    derived: &DerivedState,
    scale: f64,
) -> (i32, i32, i32) {
    (
        scaled_i32(derived.bar_height, scale).max(1),
        scaled_i32(derived.bar_horizontal_padding, scale).max(1),
        scaled_i32(config.bar.startmenu_size, scale).max(1),
    )
}

// -----------------------------------------------------------------------------
// Internal Helpers
// -----------------------------------------------------------------------------

fn init_single_monitor(ctx: &mut WmCtx, sw: i32, h: i32) -> bool {
    let template = ctx.core_mut().config_mut().tag_template.clone();
    let mut mon = Monitor::new_with_values(ctx.core_mut().config_mut().bar.show);
    mon.init_tags(&template);
    let id = ctx.core_mut().model_mut().monitors.push(mon);
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().config(), ctx.core().derived(), 1.0);
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
        m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
        m.set_bar_height(bar_height);
    }
    ctx.core_mut().select_monitor(id);
    true
}

fn update_single_monitor(ctx: &mut WmCtx, sw: i32, sh: i32) -> bool {
    let first_id = match ctx.core().model().monitors.first() {
        Some(id) => id,
        None => return false,
    };
    let (bar_height, horizontal_padding, startmenu_size) =
        scaled_monitor_ui_metrics(ctx.core().config(), ctx.core().derived(), 1.0);
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
        m.set_ui_metrics(1.0, bar_height, horizontal_padding, startmenu_size);
        m.set_bar_height(bar_height);
    }
    true
}

#[cfg(test)]
mod transfer_focus_tests {
    use super::*;

    fn outcome(
        source: MonitorId,
        target: MonitorId,
        was_selected: bool,
    ) -> crate::model::ClientTransferOutcome {
        crate::model::ClientTransferOutcome {
            source_monitor: source,
            target_monitor: target,
            was_selected,
            is_scratchpad: false,
            needs_arrange: false,
        }
    }

    #[test]
    fn preserving_focus_does_not_unfocus_an_unselected_transfer() {
        let source = MonitorId::from_raw(1);
        let target = MonitorId::from_raw(2);
        let focused = WindowId(10);
        let moved = WindowId(11);

        assert_eq!(
            transfer_focus_effect(
                TransferFocus::Preserve,
                source,
                Some(focused),
                outcome(source, target, false),
                moved,
            ),
            TransferFocusEffect::None
        );
    }

    #[test]
    fn preserving_focus_replaces_a_transferred_focused_window() {
        let source = MonitorId::from_raw(1);
        let target = MonitorId::from_raw(2);
        let moved = WindowId(11);

        assert_eq!(
            transfer_focus_effect(
                TransferFocus::Preserve,
                source,
                Some(moved),
                outcome(source, target, true),
                moved,
            ),
            TransferFocusEffect::FocusSourceReplacement {
                moved_window: moved
            }
        );
    }

    #[test]
    fn following_a_transfer_carries_the_previous_backend_focus() {
        let source = MonitorId::from_raw(1);
        let target = MonitorId::from_raw(2);
        let focused = WindowId(10);
        let moved = WindowId(11);

        assert_eq!(
            transfer_focus_effect(
                TransferFocus::FollowWindow,
                source,
                Some(focused),
                outcome(source, target, false),
                moved,
            ),
            TransferFocusEffect::FocusTransferredWindow {
                previous_focus: Some(focused),
                target_monitor: target,
                moved_window: moved,
            }
        );
    }

    #[test]
    fn monitor_reconciliation_returns_cleanup_work_and_rehomes_clients() {
        let mut model = crate::model::WmModel::new();
        let retained = model.monitors.push(Monitor {
            name: "retained".to_string(),
            ..Monitor::default()
        });
        let removed_bar = WindowId(90);
        let removed_bottom_bar = WindowId(91);
        let removed = model.monitors.push(Monitor {
            name: "removed".to_string(),
            bar_win: removed_bar,
            bottom_bar_win: removed_bottom_bar,
            ..Monitor::default()
        });
        let win = WindowId(42);
        model.insert_client(Client {
            win,
            monitor_id: removed,
            ..Client::default()
        });
        model.monitor_mut(removed).unwrap().clients.push(win);

        let outputs = [BackendOutputInfo {
            name: "retained".to_string(),
            rect: Rect::new(0, 0, 1920, 1080),
            scale: 1.0,
            vrr_support: crate::backend::BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        }];
        let result =
            reconcile_monitor_model(&mut model, &outputs, &[(20, 4, 30)], &[], true, false);

        assert!(result.changed);
        assert_eq!(
            result.removed_bar_windows,
            [removed_bar, removed_bottom_bar]
        );
        assert!(model.monitor(retained).is_some());
        assert_eq!(model.client(win).unwrap().monitor_id, retained);
    }
}
