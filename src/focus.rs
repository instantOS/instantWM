//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `CoreCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::backend::WindowOps;
use crate::contexts::{CoreCtx, WmCtx};
use crate::core_state::CoreState;
use crate::model::WmModel;
use crate::types::*;
use std::collections::HashMap;

/// Result of resolving a focus target, containing both the target window
/// and information needed for state updates.
struct FocusTargetResult {
    target: Option<WindowId>,
    sel_mon_id: MonitorId,
    current_sel: Option<WindowId>,
}

fn is_focusable_on_monitor(
    model: &WmModel,
    sel_mon_id: MonitorId,
    selected: TagMask,
    win: WindowId,
) -> bool {
    model
        .client(win)
        .is_some_and(|c| c.monitor_id == sel_mon_id && c.is_visible(selected))
}

/// Resolve the focus target based on the requested window and current state.
/// Returns `None` if there are no monitors (early exit case).
fn resolve_focus_target(model: &WmModel, win: Option<WindowId>) -> Option<FocusTargetResult> {
    if model.monitors.is_empty() {
        return None;
    }

    let sel_mon_id = model.selected_monitor_id();
    let mon = model.selected_monitor();
    let selected = mon.selected_tags();
    let current_sel = mon.selected;

    // Use the requested window if it's visible, otherwise walk the stack
    // to find the first visible non-hidden client.
    let mut target = win.filter(|&w| is_focusable_on_monitor(model, sel_mon_id, selected, w));

    if target.is_none() {
        // Try focus history first.
        if let Some(&hist_win) = mon.tag_focus_history.get(&selected)
            && is_focusable_on_monitor(model, sel_mon_id, selected, hist_win)
        {
            target = Some(hist_win);
        }

        // Fallback to top of stack.
        if target.is_none() {
            target = mon.first_visible_client(model.clients.map());
        }
    }

    Some(FocusTargetResult {
        target,
        sel_mon_id,
        current_sel,
    })
}

/// Update monitor state after focus target resolution.
fn update_focus_state(model: &mut WmModel, result: FocusTargetResult) -> Option<WindowId> {
    let FocusTargetResult {
        target, sel_mon_id, ..
    } = result;

    let target_is_tiled = target
        .and_then(|win| model.client(win))
        .is_some_and(|client| !client.mode.is_floating());

    if let Some(mon) = model.monitor_mut(sel_mon_id) {
        mon.selected = target;
        if let Some(t) = target {
            mon.tag_focus_history.insert(mon.selected_tags(), t);
            if target_is_tiled {
                mon.tag_tiled_focus_history.insert(mon.selected_tags(), t);
            }
        }
    }

    if let Some(t) = target {
        model.raise_client_in_z_order(t);
    }
    target
}

/// Backend-specific focus operations trait.
/// This allows the common focus logic to call backend-specific operations
/// without duplicating the surrounding logic.
pub(crate) trait FocusBackendOps {
    fn unfocus_current(&self, state: &CoreState, current: WindowId);
    fn focus_window(&self, ctx: &mut CoreCtx<'_>, win: WindowId);
    fn focus_none(&self);
    fn on_desktop_binding_state_changed(&self, state: &CoreState);
    fn needs_focus_refresh(&self, _target: Option<WindowId>) -> bool {
        false
    }
}

struct WaylandFocusBackend<'a> {
    wayland: &'a crate::backend::wayland::WaylandBackend,
}

impl<'a> FocusBackendOps for WaylandFocusBackend<'a> {
    fn unfocus_current(&self, _state: &CoreState, _current: WindowId) {}

    fn focus_window(&self, ctx: &mut CoreCtx<'_>, win: WindowId) {
        let is_urgent = ctx
            .model()
            .client(win)
            .map(|c| c.is_urgent)
            .unwrap_or(false);
        if is_urgent && let Some(c) = ctx.model_mut().client_mut(win) {
            c.clear_urgency();
        }
        self.wayland.set_focus(win);
    }

    fn focus_none(&self) {
        self.wayland.clear_keyboard_focus();
    }

    fn on_desktop_binding_state_changed(&self, _state: &CoreState) {}

    fn needs_focus_refresh(&self, target: Option<WindowId>) -> bool {
        match target {
            Some(win) => !self.wayland.is_keyboard_focused_on(win),
            None => false,
        }
    }
}

/// Outcome of a focus operation, used to decide whether a sync_monitor_z_order is needed.
pub(crate) struct FocusOutcome {
    /// `true` when `mon.selected` actually changed.
    changed: bool,
    /// The monitor that owns the new selection.
    monitor_id: MonitorId,
}

/// Generic focus implementation shared between X11 and Wayland.
pub(crate) fn focus_generic(
    core: &mut CoreCtx,
    win: Option<WindowId>,
    backend: &mut dyn FocusBackendOps,
) -> anyhow::Result<FocusOutcome> {
    let result = match resolve_focus_target(core.model(), win) {
        Some(r) => r,
        None => {
            return Ok(FocusOutcome {
                changed: false,
                monitor_id: core.model().selected_monitor_id(),
            });
        }
    };

    let current_sel = result.current_sel;
    let sel_mon_id = result.sel_mon_id;
    let desktop_bindings_before =
        crate::keyboard::desktop_bindings_enabled(current_sel, &core.behavior().current_mode);
    let target = update_focus_state(core.model_mut(), result);
    let desktop_bindings_after =
        crate::keyboard::desktop_bindings_enabled(target, &core.behavior().current_mode);

    // Track the previously focused window for focus-last-client.
    // This is done in the shared path so both backends behave identically.
    if current_sel != target
        && let Some(cur_win) = current_sel
    {
        core.focus.last_client = cur_win;
        backend.unfocus_current(core.state(), cur_win);
    }

    if desktop_bindings_before != desktop_bindings_after {
        backend.on_desktop_binding_state_changed(core.state());
    }

    let focus_changed = current_sel != target;
    let needs_refocus = backend.needs_focus_refresh(target);

    if let Some(w) = target {
        if focus_changed || needs_refocus {
            core.bar.mark_dirty();
            backend.focus_window(core, w);
        }
    } else if focus_changed {
        core.bar.mark_dirty();
        backend.focus_none();
    }

    Ok(FocusOutcome {
        changed: focus_changed,
        monitor_id: sel_mon_id,
    })
}

/// Best-effort focus - the single public entry point for `WmCtx` holders.
///
/// Updates `mon.selected`, backend seat focus, and — when the selection actually
/// changed — syncs the affected monitor z-order so visuals stay in sync.
/// This is critical for overlapping layouts (monocle, floating) where the
/// focused window must be visually on top.
pub fn focus(ctx: &mut crate::contexts::WmCtx, win: Option<WindowId>) {
    use crate::contexts::WmCtx::*;
    let outcome = match ctx {
        X11(x11_ctx) => {
            let mut backend = crate::backend::x11::focus::X11FocusBackend {
                x11: &x11_ctx.x11,
                x11_runtime: x11_ctx.x11_runtime,
            };
            match focus_generic(&mut x11_ctx.core, win, &mut backend) {
                Ok(o) => o,
                Err(e) => {
                    log::warn!("focus X11({:?}) failed: {}", win, e);
                    return;
                }
            }
        }
        Wayland(wayland_ctx) => {
            let mut backend = WaylandFocusBackend {
                wayland: wayland_ctx.wayland,
            };
            match focus_generic(&mut wayland_ctx.core, win, &mut backend) {
                Ok(o) => o,
                Err(e) => {
                    log::warn!("focus Wayland({:?}) failed: {}", win, e);
                    return;
                }
            }
        }
    };
    if outcome.changed {
        crate::layouts::sync_monitor_z_order(ctx, outcome.monitor_id);
    }
}

/// Backend-agnostic unfocus.
///
/// Records the window in `last_client` (for focus-last), then delegates
/// to backend-specific cleanup (border/buttons on X11, nothing extra on
/// Wayland since the Smithay seat is updated by the focus path).
pub fn unfocus_win(ctx: &mut crate::contexts::WmCtx, win: WindowId, redirect_to_root: bool) {
    use crate::contexts::{WmCtx::*, WmCtxX11};
    if win == WindowId::default() {
        return;
    }
    ctx.core_mut().focus.last_client = win;
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => {
            crate::backend::x11::focus::unfocus_win(
                core.state(),
                x11,
                x11_runtime,
                win,
                redirect_to_root,
            );
        }
        Wayland(_) => {
            // Seat focus is managed by the focus path (focus_generic →
            // set_focus / clear_seat_focus). No extra backend work needed.
        }
    }
}

/// Backend-agnostic hover-focus entry point.
///
/// Checks focus-follows-mouse guards, then delegates to `focus_soft` which
/// handles `mon.selected`, backend seat focus, and z-order sync in one place.
pub fn hover_focus_target(
    ctx: &mut crate::contexts::WmCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
    pointer_pos: Option<Point>,
) {
    if !ctx.core().behavior().focus_follows_mouse {
        return;
    }

    if let Some(win) = hovered_win
        && let Some(mid) = ctx
            .core()
            .model()
            .client(win)
            .map(|client| client.monitor_id)
        && select_monitor(ctx, mid)
    {
        // After switching monitors, continue with the hovered window so both
        // backends share the same "focus what's under the pointer" behavior.
    } else if hovered_win.is_none()
        && let Some(pointer_pos) = pointer_pos
        && select_monitor_at_pointer(ctx, pointer_pos)
    {
        return;
    }

    if should_hover_focus(
        ctx.core().model(),
        ctx.core().behavior(),
        hovered_win,
        entering_root,
    ) {
        focus(ctx, hovered_win);
    }
}

/// Common hover-focus guard checks shared by both backends.
///
/// Returns `true` when hover focus should proceed for `hovered_win`.
fn should_hover_focus(
    model: &crate::model::WmModel,
    behavior: &crate::core_state::WmBehavior,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) -> bool {
    let Some(win) = hovered_win else {
        return false;
    };
    if !behavior.focus_follows_mouse {
        return false;
    }
    // Already focused — nothing to do.
    if model.selected_win() == Some(win) {
        return false;
    }
    // Respect the "don't focus floating windows on hover" setting.
    let hovered_is_floating = model
        .client(win)
        .map(|c| c.mode.is_floating())
        .unwrap_or(false);
    let has_tiling = model.selected_monitor().is_tiling_layout();
    if !behavior.focus_follows_float_mouse && hovered_is_floating && has_tiling && !entering_root {
        return false;
    }
    true
}

/// Switch the selected monitor to `monitor_id` and re-focus the target.
///
/// Returns `true` if the selection actually changed (i.e. the monitor was not
/// already selected), `false` otherwise.
pub fn select_monitor(ctx: &mut crate::contexts::WmCtx, monitor_id: MonitorId) -> bool {
    if ctx.core().model().monitors.is_empty() {
        return false;
    }
    if monitor_id == ctx.core().model().selected_monitor_id() {
        return false;
    }

    ctx.core_mut().model_mut().set_selected_monitor(monitor_id);
    ctx.update_ewmh_desktop_props();
    focus(ctx, None);
    true
}

pub fn select_monitor_for_client(ctx: &mut crate::contexts::WmCtx, win: WindowId) -> bool {
    let Some(monitor_id) = ctx
        .core()
        .model()
        .client(win)
        .map(|client| client.monitor_id)
    else {
        return false;
    };
    select_monitor(ctx, monitor_id)
}

/// Route an external activation request (e.g. xdg-activation from a notification)
/// through the normal WM focus path.
///
/// This makes the target monitor current, reveals the client's non-scratchpad
/// tags when needed, and then applies the backend focus/sync_monitor_z_order logic.
pub fn activate_client(ctx: &mut crate::contexts::WmCtx, win: WindowId) -> bool {
    let Some((monitor_id, client_tags)) = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|client| (client.monitor_id, client.tags))
    else {
        return false;
    };

    if monitor_id != ctx.core().model().selected_monitor_id() {
        ctx.core_mut().model_mut().set_selected_monitor(monitor_id);
    }

    let target_tags = client_tags.without_scratchpad();
    let visible_tags = ctx.core().model().selected_monitor().selected_tags();
    if !target_tags.is_empty() && !target_tags.intersects(visible_tags) {
        crate::tags::view::view_tags(ctx, target_tags);
    }

    focus(ctx, Some(win));
    true
}

pub fn select_monitor_at_pointer(ctx: &mut crate::contexts::WmCtx, pointer_pos: Point) -> bool {
    let Some(new_mon_id) = ctx
        .core()
        .state()
        .model
        .monitors
        .find_monitor_at_pointer(pointer_pos)
    else {
        return false;
    };
    select_monitor(ctx, new_mon_id)
}

fn get_directional_candidates(
    clients: &[WindowId],
    globals_map: &HashMap<WindowId, Client>,
    selected_tags: TagMask,
    source_win: WindowId,
    source_center: crate::types::Point,
    direction: Direction,
) -> Option<WindowId> {
    let mut out_client: Option<WindowId> = None;
    let mut min_score: i32 = 0;

    for (c_win, c) in crate::types::ClientListIter::new(clients, globals_map) {
        if !c.is_visible(selected_tags) {
            continue;
        }

        let center = c.geo.center();

        if is_client_in_direction(c_win, source_win, center, source_center, direction) {
            let score = calculate_direction_score(center, source_center, direction);
            if score < min_score || min_score == 0 {
                out_client = Some(c_win);
                min_score = score;
            }
        }
    }

    out_client
}

fn is_client_in_direction(
    c_win: WindowId,
    source_win: WindowId,
    center: crate::types::Point,
    source_center: crate::types::Point,
    direction: Direction,
) -> bool {
    if c_win == source_win {
        return false;
    }

    match direction {
        Direction::Up => center.y < source_center.y,
        Direction::Down => center.y > source_center.y,
        Direction::Left => center.x < source_center.x,
        Direction::Right => center.x > source_center.x,
    }
}

fn calculate_direction_score(
    center: crate::types::Point,
    source_center: crate::types::Point,
    direction: Direction,
) -> i32 {
    let dx = center.abs_diff_x(&source_center);
    let dy = center.abs_diff_y(&source_center);

    match direction {
        Direction::Up | Direction::Down => {
            if dx > dy {
                return i32::MAX;
            }
            // Use weighted scoring to favor windows that are more vertically aligned.
            dx + dy / 4
        }
        Direction::Left | Direction::Right => {
            if dy > dx {
                return i32::MAX;
            }
            // Use weighted scoring to favor windows that are more horizontally aligned.
            dy + dx / 4
        }
    }
}

/// Shared logic for directional focus - finds the candidate window.
fn get_direction_focus_candidate(
    model: &crate::model::WmModel,
    direction: Direction,
) -> Option<WindowId> {
    if model.monitors.is_empty() {
        return None;
    }
    let mon = model.selected_monitor();
    let source_win = mon.selected?;
    let source_client = model.client(source_win)?;
    let source_center = source_client.geo.center();

    let selected = mon.selected_tags();

    get_directional_candidates(
        &mon.clients,
        model.clients.map(),
        selected,
        source_win,
        source_center,
        direction,
    )
}

pub fn focus_last_client(ctx: &mut WmCtx) {
    let last_client_win = ctx.core().focus.last_client;
    if last_client_win == WindowId::default() {
        return;
    }
    let last_win = last_client_win;

    let last_client = match ctx.core().model().client(last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        let name = last_client.scratchpad.as_ref().unwrap().name.clone();
        let _ = crate::floating::scratchpad_show_name(ctx, &name);
        return;
    }

    let tags = last_client.tags;
    let last_mon_id = last_client.monitor_id;

    let sel_mon_id = ctx.core().model().selected_monitor_id();
    if !ctx.core().model().monitors.is_empty()
        && sel_mon_id != last_mon_id
        && let Some(sel) = ctx
            .core()
            .model()
            .monitor(sel_mon_id)
            .and_then(|m| m.selected)
    {
        unfocus_win(ctx, sel, false);
        ctx.core_mut().model_mut().set_selected_monitor(last_mon_id);
    }

    if let Some(cur) = ctx.core().model().selected_win() {
        ctx.core_mut().focus.last_client = cur;
    }

    crate::tags::view::view_tags(ctx, tags);
    focus(ctx, Some(last_win));

    let monitor_id = ctx.core().model().selected_monitor_id();
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

fn get_visible_stack(mon: &Monitor, clients: &HashMap<WindowId, Client>) -> Vec<WindowId> {
    let mut stack = Vec::new();
    let selected = mon.selected_tags();

    for (c_win, c) in mon.iter_clients(clients) {
        if c.is_visible(selected) {
            stack.push(c_win);
        }
    }

    stack
}

/// Shared logic to compute the next stack index for focus.
fn get_stack_focus_target(
    model: &crate::model::WmModel,
    direction: StackDirection,
) -> Option<WindowId> {
    if model.monitors.is_empty() {
        return None;
    }
    let mon = model.selected_monitor();
    let stack = get_visible_stack(mon, model.clients.map());

    if stack.is_empty() {
        return None;
    }

    let selected_window = model.selected_win();
    let current_idx = match selected_window {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if direction.is_forward() {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    Some(stack[next_idx])
}

pub fn direction_focus(ctx: &mut WmCtx, direction: Direction) {
    if let Some(target) = get_direction_focus_candidate(ctx.core().model(), direction) {
        focus(ctx, Some(target));
    }
}

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    if let Some(target) = get_stack_focus_target(ctx.core().model(), direction) {
        focus(ctx, Some(target));
    }
}
