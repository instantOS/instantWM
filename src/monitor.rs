//! Monitor management via the `MonitorManager` struct.
//!
//! This module encapsulates monitor state and logic, providing a clean API
//! for monitor-related operations.

use crate::backend::BackendOutputInfo;
use crate::bar::policy::TagBarPolicy;
use crate::contexts::WmCtx;
use crate::core_state::{CoreState, DerivedState, EffectiveConfig};
use crate::focus::refresh_focus_after_selection;
use crate::types::*;

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

    /// Insert a monitor, assigning it a fresh stable [`MonitorId`].
    ///
    /// If this is the first monitor, it becomes the selected monitor.
    #[cfg(test)]
    pub fn push(&mut self, mut m: Monitor) -> MonitorId {
        let id = self.allocate_id();
        m.monitor_id = id;
        let was_empty = self.monitors.is_empty();
        self.monitors.push(m);
        if was_empty {
            self.selected = id;
        }
        id
    }

    /// Allocate a fresh stable id without inserting a monitor.
    pub(crate) fn allocate_id(&mut self) -> MonitorId {
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

    pub fn find_monitor_for(&self, w: WindowId) -> Option<&Monitor> {
        self.iter()
            .map(|(_, monitor)| monitor)
            .find(|monitor| w == monitor.bar_win || w == monitor.bottom_bar_win)
            .or_else(|| self.iter_all().find(|monitor| monitor.has_client(w)))
    }

    /// Find the monitor with the largest intersection with `rect`.
    pub fn monitor_intersecting_rect(&self, rect: Rect) -> Option<&Monitor> {
        let mut best = None;
        let mut max_area = 0;
        for monitor in &self.monitors {
            let area = monitor
                .monitor_rect
                .intersection(&rect)
                .map_or(0, |intersection| intersection.area());
            if area > max_area {
                max_area = area;
                best = Some(monitor);
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

    /// Find the monitor with the largest intersection with `rect`, falling back
    /// to the currently selected monitor.
    pub fn monitor_by_rect(&self, rect: Rect) -> Option<&Monitor> {
        self.monitor_intersecting_rect(rect)
            .or_else(|| self.selected_monitor())
    }

    /// Find the monitor containing `ptr`, falling back to the currently selected monitor.
    pub fn monitor_at_pointer(&self, ptr: Point) -> Option<&Monitor> {
        self.monitor_by_rect(Rect::new(ptr.x, ptr.y, 1, 1))
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

    match focus_policy {
        TransferFocus::Preserve => {
            if selected_monitor_before == outcome.source_monitor && outcome.was_selected {
                refresh_focus_after_selection(ctx, focused_before, None);
            }
        }
        TransferFocus::FollowWindow => {
            ctx.core_mut().select_monitor(outcome.target_monitor);
            ctx.core_mut()
                .select_on_monitor(outcome.target_monitor, Some(win));
            ctx.update_ewmh_desktop_props();
            refresh_focus_after_selection(ctx, focused_before, Some(win));
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
    crate::mouse::warp::warp_pointer_to_monitor(ctx, target);
}

/// Resolve a [`MonitorSelector`] against the current monitor set.
///
/// Returns `None` for [`MonitorSelector::Any`] (callers treat that as "leave
/// alone") and for selectors that match no connected monitor.
pub fn resolve_monitor_selector(
    model: &crate::model::WmModel,
    selector: &MonitorSelector,
) -> Option<crate::types::MonitorId> {
    match selector {
        MonitorSelector::Any => None,
        MonitorSelector::Focused => model.selected_monitor().map(|monitor| monitor.id()),
        MonitorSelector::Primary => model.monitors.id_at_position(0),
        MonitorSelector::Index(position) => model.monitors.id_at_position(*position),
        MonitorSelector::Name(name) => model
            .monitors_iter()
            .find(|(_, monitor)| &monitor.name == name)
            .map(|(id, _)| id),
    }
}

pub fn move_to_monitor_and_follow(ctx: &mut WmCtx, direction: MonitorDirection) {
    let c_win = match ctx.core().model().selected_win() {
        Some(w) => w,
        None => return,
    };

    crate::tags::send_to_monitor(ctx, direction);

    let previous_focus = ctx.core().model().selected_win();
    if let Some(monitor_id) = ctx.core().model().monitor_of_client(c_win) {
        ctx.core_mut().select_monitor(monitor_id);
    }

    refresh_focus_after_selection(ctx, previous_focus, Some(c_win));

    ctx.window_backend().raise_window_visual_only(c_win);
    ctx.warp_cursor_to_client(c_win);
}

/// Sanitize the `[monitors]` config into the effective policy, project it
/// onto the backend and store it for every later reader.
pub fn apply_monitor_config(ctx: &mut WmCtx) {
    let policy = crate::output_mirror::MonitorPolicy::new(&ctx.core().config().monitors);
    ctx.apply_monitor_configs(&policy);
    ctx.core_mut().derived_mut().monitor_policy = policy;
    // Per-output tag slots are resolved at bar render time.
    ctx.request_bar_update();
    refresh_monitor_layout(ctx);
}

/// One output per logical monitor: the backend's outputs with physically
/// cloned ones folded together. Existing monitor names keep their identity
/// when outputs fold.
pub(crate) fn logical_outputs(
    outputs: Vec<BackendOutputInfo>,
    mirrors: &crate::output_mirror::MirrorMap,
    model: &crate::model::WmModel,
) -> Vec<BackendOutputInfo> {
    let preferred: std::collections::HashSet<String> = model
        .monitors_iter()
        .map(|(_, m)| m.name.clone())
        .filter(|name| !name.is_empty())
        .collect();
    crate::output_mirror::fold_cloned_outputs(outputs, mirrors, &preferred)
}

pub fn refresh_monitor_layout(ctx: &mut WmCtx) -> bool {
    let outputs = logical_outputs(
        ctx.output_backend().get_outputs(),
        &ctx.core().derived().monitor_policy.mirrors,
        ctx.core().model(),
    );
    sync_monitors_from_outputs(ctx, outputs)
}

fn output_layout_extent(outputs: &[BackendOutputInfo]) -> Rect {
    // Origin-aware bounding box: min origin to max edge, so negative positions
    // (e.g. an output placed above with y < 0) keep both their size and origin.
    let min_x = outputs.iter().map(|o| o.rect.x).min().unwrap_or(0);
    let min_y = outputs.iter().map(|o| o.rect.y).min().unwrap_or(0);
    let max_x = outputs
        .iter()
        .map(|o| o.rect.x.saturating_add(o.rect.w))
        .max()
        .unwrap_or(1);
    let max_y = outputs
        .iter()
        .map(|o| o.rect.y.saturating_add(o.rect.h))
        .max()
        .unwrap_or(1);
    Rect::new(
        min_x,
        min_y,
        max_x.saturating_sub(min_x).max(1),
        max_y.saturating_sub(min_y).max(1),
    )
}

fn sync_runtime_screen_size(derived: &mut DerivedState, layout: Rect) -> bool {
    if derived.display.x != layout.x
        || derived.display.y != layout.y
        || derived.display.width != layout.w
        || derived.display.height != layout.h
    {
        derived.display.x = layout.x;
        derived.display.y = layout.y;
        derived.display.width = layout.w;
        derived.display.height = layout.h;
        true
    } else {
        false
    }
}

fn apply_output_to_monitor(
    m: &mut Monitor,
    position: usize,
    output: &BackendOutputInfo,
    metrics: MonitorUiMetrics,
) {
    m.apply_output_layout(
        position,
        output.name.clone(),
        output.rect,
        output.scale,
        metrics,
    );
}

fn output_geom_changed(m: &Monitor, output: &BackendOutputInfo, metrics: MonitorUiMetrics) -> bool {
    m.monitor_rect != output.rect
        || m.name != output.name
        || (m.ui_scale - output.scale).abs() > f64::EPSILON
        || m.ui_metrics() != metrics
}

fn notify_monitor_layout_changed(ctx: &mut WmCtx, changed: bool) {
    if !changed {
        return;
    }
    ctx.core_mut().queue_layout_for_all_monitors();
    ctx.core_mut().bar.mark_dirty();
    let target_monitor_id = ctx
        .pointer_backend()
        .pointer_location()
        .and_then(|ptr| ctx.core().model().monitors.monitor_at_pointer(ptr))
        .map(Monitor::id);
    if let Some(id) = target_monitor_id {
        ctx.core_mut().select_monitor(id);
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

/// Move clients whose monitor has disappeared onto a surviving monitor.
///
/// A topology change must never leave an ordinary managed window reachable
/// only through tags that are not projected anywhere. Preserve the clients'
/// tag identity, but widen the survivor's current view to include every tag
/// carried in from removed monitors. This makes unplugging an output a lossless
/// operation from the user's point of view: no window is silently retagged and
/// every migrated window is immediately reachable.
fn rehome_orphaned_clients(
    model: &mut crate::model::WmModel,
    survivor: MonitorId,
    orphaned: Vec<(Client, bool)>,
) {
    if orphaned.is_empty() {
        return;
    }

    let mut reachable_tags = model
        .monitor(survivor)
        .map(Monitor::selected_tags)
        .unwrap_or(TagMask::EMPTY);
    for (client, was_selected) in orphaned {
        if !client.is_scratchpad() {
            reachable_tags = reachable_tags | client.tags;
        }
        let readopted = model.readopt_client(survivor, client, was_selected);
        debug_assert!(readopted, "orphaned managed client must be re-homeable");
    }
    if let Some(monitor) = model.monitor_mut(survivor) {
        monitor.set_selected_tags(reachable_tags);
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
    let show_bottom_bar = ctx.core().config().bar.show_bottom;

    let layout_size = output_layout_extent(&outputs);
    let mut changed = sync_runtime_screen_size(ctx.core_mut().derived_mut(), layout_size);

    // Pre-compute per-output UI metrics and tag-display policies while we
    // hold an immutable config borrow.
    let metrics: Vec<MonitorUiMetrics> = outputs
        .iter()
        .map(|o| scaled_monitor_ui_metrics(ctx.core().config(), o.scale))
        .collect();
    let policies: Vec<TagBarPolicy> = outputs
        .iter()
        .map(|o| TagBarPolicy::resolve(ctx.core().config(), &o.name))
        .collect();

    let reconciliation = ctx.core_mut().mutate_selection(|model| {
        reconcile_monitor_model(
            model,
            &outputs,
            &metrics,
            &template,
            show_bottom_bar,
            &policies,
        )
    });
    changed |= reconciliation.changed;
    let added_monitors = reconciliation.added_monitors;

    for bar_win in reconciliation.removed_bar_windows {
        ctx.destroy_monitor_bar_window(bar_win);
    }

    notify_monitor_layout_changed(ctx, changed);

    // A freshly built monitor starts without backend bar resources, and no
    // other step on the topology path creates them: reconciling here is what
    // makes a runtime hot-plug show a bar instead of an empty strip. It runs
    // after the layout notification so bar geometry and the systray reservation
    // use the post-change selection. Existing windows are only re-synced, and
    // the Wayland branch is a no-op because its bars render from the model.
    if added_monitors {
        ctx.refresh_bar_content();
    }

    if ctx.core().model().selected_win() != previous_focus {
        refresh_focus_after_selection(ctx, previous_focus, None);
    }
    changed
}

#[derive(Debug)]
struct MonitorReconciliation {
    changed: bool,
    /// A monitor was constructed instead of matched, so it carries no backend
    /// bar resources yet.
    added_monitors: bool,
    removed_bar_windows: Vec<WindowId>,
}

/// Reconcile backend output descriptions with the authoritative monitor graph.
///
/// This operation owns stable-ID reuse, new-monitor construction, and client
/// rehoming. It reports backend resources that the orchestration layer must
/// destroy and monitors it constructed (which still need theirs created)
/// rather than performing backend I/O while mutating the model.
fn reconcile_monitor_model(
    model: &mut crate::model::WmModel,
    outputs: &[BackendOutputInfo],
    metrics: &[MonitorUiMetrics],
    tag_template: &[crate::types::Tag],
    show_bottom_bar: bool,
    policies: &[TagBarPolicy],
) -> MonitorReconciliation {
    debug_assert_eq!(outputs.len(), metrics.len());
    debug_assert_eq!(outputs.len(), policies.len());
    let mut changed = model.monitors.len() != outputs.len();
    let mut added_monitors = false;

    // Drain old monitors into a pool. They keep their stable ids + workspace
    // state; matched ones are reused, the rest are dropped after the rebuild.
    let old_monitors = model.monitors.drain();
    let mut pool: Vec<Option<Monitor>> = old_monitors.into_iter().map(Some).collect();

    let mut new_monitors = Vec::with_capacity(outputs.len());
    for (i, output) in outputs.iter().enumerate() {
        let metrics = metrics[i];
        match take_matching_monitor(&mut pool, i, output) {
            Some(mut m) => {
                if output_geom_changed(&m, output, metrics) {
                    changed = true;
                }
                // Keep the reused monitor's stable id and workspace state.
                apply_output_to_monitor(&mut m, i, output, metrics);
                new_monitors.push(m);
            }
            None => {
                changed = true;
                added_monitors = true;
                let id = model.monitors.allocate_id();
                let mut m = Monitor::new_with_values();
                m.show_bottom_bar = show_bottom_bar;
                policies[i].apply_to(&mut m);
                m.monitor_id = id;
                m.init_tags(tag_template);
                apply_output_to_monitor(&mut m, i, output, metrics);
                new_monitors.push(m);
            }
        }
    }

    // Collect the orphaned monitors' clients and bar windows as cleanup work
    // for the caller. Clients leave with their monitor, so they are taken out
    // here while the removed monitor is still in hand. The default id is the
    // "no bar" placeholder, not a real window.
    let mut removed_bar_windows = Vec::new();
    let mut orphaned_clients = Vec::new();
    for monitor in pool.into_iter().flatten() {
        for bar in [monitor.bar_win, monitor.bottom_bar_win] {
            if bar != WindowId::default() {
                removed_bar_windows.push(bar);
            }
        }
        let selected = monitor.selected;
        orphaned_clients.extend(
            monitor
                .clients
                .into_iter()
                .map(|(win, client)| (client, Some(win) == selected)),
        );
    }

    // Restore the rebuilt list. The selection is preserved if its monitor still
    // exists; otherwise the manager falls back to the first monitor.
    model.monitors.restore(new_monitors);

    // Re-home any clients whose monitor was removed onto the first survivor.
    if let Some(survivor) = model.monitors.first() {
        rehome_orphaned_clients(model, survivor, orphaned_clients);
    }

    MonitorReconciliation {
        changed,
        added_monitors,
        removed_bar_windows,
    }
}

fn scaled_monitor_ui_metrics(config: &EffectiveConfig, scale: f64) -> MonitorUiMetrics {
    let base = config.bar_metrics();
    MonitorUiMetrics {
        bar_height: crate::types::geometry::scaled_px(base.height, scale).max(1),
        horizontal_padding: crate::types::geometry::scaled_px(base.horizontal_padding, scale)
            .max(1),
        startmenu_size: crate::types::geometry::scaled_px(config.bar.startmenu_size, scale).max(1),
    }
}

/// Re-apply scaled UI metrics to every monitor after the unscaled base changed.
///
/// Config provides the unscaled base metrics while each monitor owns the scaled
/// copy for its output's UI scale. Output topology sync does this as part of
/// reconciling monitors; this
/// entry point covers the paths that change the base without touching
/// topology — `config set` and a full config reload, both of which funnel
/// through [`Wm::reinit_bar_resources`](crate::wm::Wm::reinit_bar_resources).
///
/// Layout deliberately does *not* substitute for this: `arrange` reads bar
/// geometry, and the unscaled global is never a valid value for a scaled
/// output. Writing it back from a layout pass would silently undo the scaling.
///
/// Returns whether any monitor's metrics changed.
pub fn resync_monitor_ui_metrics(core: &mut CoreState) -> bool {
    // Compute while the config is immutably borrowed, then apply separately.
    let pending: Vec<(MonitorId, f64, MonitorUiMetrics)> = core
        .model
        .monitors_iter()
        .map(|(id, monitor)| {
            let metrics = scaled_monitor_ui_metrics(&core.config, monitor.ui_scale);
            (id, monitor.ui_scale, metrics)
        })
        .collect();

    let mut changed = false;
    for (id, ui_scale, metrics) in pending {
        let Some(monitor) = core.model.monitors.get_mut(id) else {
            continue;
        };
        if monitor.ui_metrics() == metrics {
            continue;
        }
        monitor.set_ui_metrics(ui_scale, metrics);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

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
        model.add_client(
            removed,
            Client {
                win,
                tags: TagMask::single(2).unwrap(),
                ..Client::default()
            },
        );
        model
            .monitor_mut(retained)
            .unwrap()
            .set_selected_tags(TagMask::single(1).unwrap());

        let outputs = [BackendOutputInfo {
            name: "retained".to_string(),
            rect: Rect::new(0, 0, 1920, 1080),
            scale: 1.0,
            vrr_support: crate::backend::BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
            mirrors: Vec::new(),
        }];
        let result = reconcile_monitor_model(
            &mut model,
            &outputs,
            &[MonitorUiMetrics {
                bar_height: 20,
                horizontal_padding: 4,
                startmenu_size: 30,
            }],
            &[],
            false,
            &[TagBarPolicy {
                show_bar: true,
                tag_slots: crate::types::tag::DEFAULT_TAG_SLOTS,
            }],
        );

        assert!(result.changed);
        assert!(!result.added_monitors);
        assert_eq!(
            result.removed_bar_windows,
            [removed_bar, removed_bottom_bar]
        );
        assert!(model.monitor(retained).is_some());
        assert_eq!(model.monitor_of_client(win), Some(retained));
        assert_eq!(
            model.monitor(retained).unwrap().selected_tags(),
            TagMask::single(1).unwrap() | TagMask::single(2).unwrap()
        );
        assert!(
            model
                .client(win)
                .unwrap()
                .is_visible(model.monitor(retained).unwrap().selected_tags())
        );
    }

    #[test]
    fn monitor_reconciliation_preserves_identity_and_adds_new_output() {
        let mut model = crate::model::WmModel::new();
        let retained = model.monitors.push(Monitor {
            name: "eDP-1".to_string(),
            ..Monitor::default()
        });
        let outputs = [
            BackendOutputInfo {
                name: "eDP-1".to_string(),
                rect: Rect::new(0, 0, 1920, 1080),
                scale: 1.0,
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: None,
                vrr_enabled: false,
                mirrors: Vec::new(),
            },
            BackendOutputInfo {
                name: "HDMI-A-1".to_string(),
                rect: Rect::new(1920, 0, 3840, 2160),
                scale: 1.0,
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: None,
                vrr_enabled: false,
                mirrors: Vec::new(),
            },
        ];
        let result = reconcile_monitor_model(
            &mut model,
            &outputs,
            &[
                MonitorUiMetrics {
                    bar_height: 20,
                    horizontal_padding: 4,
                    startmenu_size: 30,
                },
                MonitorUiMetrics {
                    bar_height: 20,
                    horizontal_padding: 4,
                    startmenu_size: 30,
                },
            ],
            &[],
            false,
            &[
                TagBarPolicy {
                    show_bar: true,
                    tag_slots: crate::types::tag::DEFAULT_TAG_SLOTS,
                },
                TagBarPolicy {
                    show_bar: true,
                    tag_slots: 5,
                },
            ],
        );

        assert!(result.changed);
        assert!(result.added_monitors, "the new output built a monitor");
        assert!(result.removed_bar_windows.is_empty());
        assert_eq!(model.monitors.len(), 2);
        assert_eq!(model.monitor(retained).unwrap().name, "eDP-1");
        let hdmi_id = model
            .monitors_iter()
            .find_map(|(id, monitor)| (monitor.name == "HDMI-A-1").then_some(id))
            .expect("new HDMI monitor");
        assert_ne!(hdmi_id, retained);
        assert!(model.monitor(hdmi_id).unwrap().bar_default_show);
    }

    #[test]
    fn geometry_only_reconciliation_does_not_flag_added_monitors() {
        let mut model = crate::model::WmModel::new();
        let retained = model.monitors.push(Monitor {
            name: "eDP-1".to_string(),
            ..Monitor::default()
        });
        let bar_win = WindowId(90);
        model.monitor_mut(retained).unwrap().bar_win = bar_win;
        let outputs = [BackendOutputInfo {
            name: "eDP-1".to_string(),
            rect: Rect::new(0, 0, 2560, 1440),
            scale: 1.0,
            vrr_support: crate::backend::BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
            mirrors: Vec::new(),
        }];

        let result = reconcile_monitor_model(
            &mut model,
            &outputs,
            &[MonitorUiMetrics {
                bar_height: 20,
                horizontal_padding: 4,
                startmenu_size: 30,
            }],
            &[],
            false,
            &[TagBarPolicy {
                show_bar: true,
                tag_slots: crate::types::tag::DEFAULT_TAG_SLOTS,
            }],
        );

        // Only the geometry moved: the monitor keeps its identity and its bar
        // window, so the caller must not rebuild bar resources.
        assert!(result.changed);
        assert!(!result.added_monitors);
        assert!(result.removed_bar_windows.is_empty());
        assert_eq!(model.monitors.first(), Some(retained));
        assert_eq!(model.monitor(retained).unwrap().bar_win, bar_win);
        assert_eq!(
            model.monitor(retained).unwrap().monitor_rect,
            Rect::new(0, 0, 2560, 1440)
        );
    }

    #[test]
    fn layout_extent_and_screen_size_keep_negative_origin() {
        let outputs = [
            BackendOutputInfo {
                name: "top".to_string(),
                rect: Rect::new(0, -1080, 1920, 1080),
                scale: 1.0,
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: None,
                vrr_enabled: false,
                mirrors: Vec::new(),
            },
            BackendOutputInfo {
                name: "bottom".to_string(),
                rect: Rect::new(0, 0, 1920, 1080),
                scale: 1.0,
                vrr_support: crate::backend::BackendVrrSupport::Unsupported,
                vrr_mode: None,
                vrr_enabled: false,
                mirrors: Vec::new(),
            },
        ];

        assert_eq!(
            super::output_layout_extent(&outputs),
            Rect::new(0, -1080, 1920, 2160)
        );

        let mut derived = crate::core_state::DerivedState::default();
        assert!(super::sync_runtime_screen_size(
            &mut derived,
            Rect::new(0, -1080, 1920, 2160)
        ));
        assert_eq!(derived.display.x, 0);
        assert_eq!(derived.display.y, -1080);
        assert_eq!(derived.display.width, 1920);
        assert_eq!(derived.display.height, 2160);
        assert_eq!(
            derived.display.screen_rect(),
            Rect::new(0, -1080, 1920, 2160)
        );
    }
}
