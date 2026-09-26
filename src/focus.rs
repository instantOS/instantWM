//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `CoreCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::contexts::{CoreCtx, WmCtx};
use crate::core_state::CoreState;
use crate::model::WmModel;
use crate::types::*;

fn is_focusable_on_monitor(
    model: &WmModel,
    sel_mon_id: MonitorId,
    selected: TagMask,
    win: WindowId,
) -> bool {
    model
        .client_view(win)
        .is_some_and(|view| view.monitor.id() == sel_mon_id && view.client.is_visible(selected))
}

/// Resolve the focus target on the selected monitor.
fn resolve_focus_target(model: &WmModel, win: Option<WindowId>) -> Option<WindowId> {
    let sel_mon_id = model.selected_monitor_id();
    let mon = model.expect_selected_monitor();
    let selected = mon.visible_tags();

    // Use the requested window if it is visible. Otherwise restore the newest
    // eligible focus-history entry, then fall back to persistent z-order.
    let mut target = win.filter(|&w| is_focusable_on_monitor(model, sel_mon_id, selected, w));

    if target.is_none() {
        // Try focus history first.
        if let Some(hist_win) = mon.most_recent_focus(selected, |win| {
            is_focusable_on_monitor(model, sel_mon_id, selected, win)
        }) {
            target = Some(hist_win);
        }

        // Fallback to top of stack.
        if target.is_none() {
            target = mon.first_visible_client();
        }
    }

    target
}

/// Update monitor state after focus target resolution.
fn update_focus_state(model: &mut WmModel, sel_mon_id: MonitorId, target: Option<WindowId>) {
    if let Some(mon) = model.monitor_mut(sel_mon_id) {
        mon.selected = target;
        if let Some(t) = target
            && mon.overview_state.is_none()
        {
            mon.record_focus(mon.selected_tags(), t);
        }
    }
}

/// Backend-specific focus operations trait.
/// This allows the common focus logic to call backend-specific operations
/// without duplicating the surrounding logic.
///
/// Implementations only ever need shared access to themselves, so
/// `apply_focus_transition` takes `&dyn FocusBackendOps` rather than
/// `&mut dyn`. That lets a backend implement this directly on its own handle
/// instead of wrapping it in a throwaway adapter struct.
pub(crate) trait FocusBackendOps {
    fn project_focus(&self, ctx: &mut CoreCtx<'_>, projection: FocusProjection);
    fn on_desktop_binding_state_changed(&self, state: &CoreState);
    fn needs_focus_refresh(&self, _target: Option<WindowId>) -> bool {
        false
    }
}

/// Complete backend projection of one core focus transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FocusProjection {
    pub previous: Option<WindowId>,
    pub current: Option<WindowId>,
}

/// Whether [`apply_focus_transition`] must re-apply backend focus state even
/// when the model selection did not change.
///
/// `IfNeeded` touches the backend only when the selection actually moved or the
/// backend reports its own focus as stale. `Force` re-applies seat focus and
/// desktop bindings unconditionally — used after client removal or monitor
/// switches, where `mon.selected` can no longer be trusted to mirror prior
/// backend state.
pub(crate) enum BackendRefresh {
    IfNeeded,
    Force,
}

/// Generic focus implementation shared between X11 and Wayland.
///
/// This is the backend-independent half of a focus change. The public
/// [`focus`] entry point additionally follows the selection into the overview
/// and syncs projected z-order; callers that need the previous backend focus to
/// be re-derived from scratch use [`refresh_focus`] instead.
pub(crate) fn apply_focus_transition(
    core: &mut CoreCtx,
    win: Option<WindowId>,
    previous_focus: Option<WindowId>,
    backend: &dyn FocusBackendOps,
    refresh: BackendRefresh,
) -> Option<MonitorId> {
    let force_backend_refresh = matches!(refresh, BackendRefresh::Force);
    if core.model().monitors.is_empty() {
        return None;
    }

    let sel_mon_id = core.model().selected_monitor_id();
    let target = resolve_focus_target(core.model(), win);
    let desktop_bindings_before =
        crate::keyboard::desktop_bindings_enabled(previous_focus, &core.behavior().current_mode);
    core.mutate_selection(|model| update_focus_state(model, sel_mon_id, target));
    let focus_changed = previous_focus != target;
    let desktop_bindings_after =
        crate::keyboard::desktop_bindings_enabled(target, &core.behavior().current_mode);

    // Track the previously focused window for focus-last-client.
    // This is done in the shared path so both backends behave identically.
    if focus_changed && let Some(cur_win) = previous_focus {
        core.focus.last_client = cur_win;
    }

    if desktop_bindings_before != desktop_bindings_after || force_backend_refresh {
        backend.on_desktop_binding_state_changed(core.state());
    }

    let needs_refocus = backend.needs_focus_refresh(target);

    if focus_changed || needs_refocus || force_backend_refresh {
        core.bar.mark_dirty();
        backend.project_focus(
            core,
            FocusProjection {
                previous: previous_focus,
                current: target,
            },
        );
    }

    (focus_changed || force_backend_refresh).then_some(sel_mon_id)
}

/// Best-effort focus - the single public entry point for `WmCtx` holders.
///
/// Updates `mon.selected`, backend seat focus, and — when the selection actually
/// changed — syncs the affected monitor's projected z-order.
///
/// Focus deliberately does not mutate persistent stacking. Overlapping layout
/// policy may still project a focused tiled window on top (notably maximized
/// presentation), while floating windows retain their explicit stacking order.
pub fn focus(ctx: &mut crate::contexts::WmCtx, win: Option<WindowId>) {
    let previous = ctx.core().model().selected_win();
    focus_impl(ctx, win, previous, BackendRefresh::IfNeeded);
    crate::overview::follow_focus(ctx);
}

/// Re-resolve focus and reapply all backend focus/input state.
///
/// Client removal clears model references atomically, so lifecycle paths cannot
/// infer the previous backend focus from `Monitor::selected` afterwards. This
/// explicit operation keeps that invariant without leaving keyboard grabs or
/// seat focus stale.
pub(crate) fn refresh_focus(ctx: &mut crate::contexts::WmCtx, win: Option<WindowId>) {
    let previous = ctx.core().model().selected_win();
    refresh_focus_after_selection(ctx, previous, win);
}

/// Re-resolve selection after an earlier model transaction and project it
/// using the backend focus that existed before that transaction.
pub(crate) fn refresh_focus_after_selection(
    ctx: &mut crate::contexts::WmCtx,
    previous_focus: Option<WindowId>,
    win: Option<WindowId>,
) {
    focus_impl(ctx, win, previous_focus, BackendRefresh::Force);
    crate::overview::follow_focus(ctx);
}

fn focus_impl(
    ctx: &mut crate::contexts::WmCtx,
    win: Option<WindowId>,
    previous_focus: Option<WindowId>,
    refresh: BackendRefresh,
) {
    use crate::contexts::WmCtx::*;
    let z_order_monitor = match ctx {
        X11(x11_ctx) => {
            let backend = crate::backend::x11::focus::X11FocusBackend {
                x11: &x11_ctx.x11,
                x11_runtime: &*x11_ctx.x11_runtime,
            };
            apply_focus_transition(&mut x11_ctx.core, win, previous_focus, &backend, refresh)
        }
        Wayland(wayland_ctx) => apply_focus_transition(
            &mut wayland_ctx.core,
            win,
            previous_focus,
            wayland_ctx.wayland,
            refresh,
        ),
    };
    if let Some(monitor_id) = z_order_monitor {
        crate::layouts::sync_monitor_z_order(ctx, monitor_id);
    }
}

/// Backend-agnostic hover-focus entry point.
///
/// Checks focus-follows-mouse guards, then delegates to [`focus`] which
/// handles `mon.selected`, backend seat focus, and z-order sync in one place.
pub fn apply_hover_focus(
    ctx: &mut crate::contexts::WmCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
    pointer_pos: Option<Point>,
    trigger: crate::types::HoverFocusTrigger,
) {
    if !ctx
        .core()
        .config()
        .window
        .focus_follows_mouse
        .allows(trigger)
    {
        return;
    }
    // Overview owns a pending selection rather than immediately sending
    // keyboard input to each hovered application. Handling it here keeps both
    // backends on one focus-follows-mouse path; confirming overview commits the
    // hovered card as real focus.
    if crate::overview::hover_window(ctx, hovered_win, pointer_pos) {
        return;
    }
    // Keyboard tree placement owns a virtual selection cursor. Physical
    // pointer motion must not steal focus from the source window while that
    // session is active.
    if ctx.current_mode().tree_placement().is_some() {
        return;
    }
    if let Some(win) = hovered_win
        && let Some(mid) = ctx.core().model().monitor_of_client(win)
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
        &ctx.core().config().window,
        hovered_win,
        entering_root,
    ) {
        focus(ctx, hovered_win);
    }
}

/// Apply the optional click-to-raise policy after normal client-area focus.
///
/// Bar-title and move/resize interactions are explicit stacking operations and
/// bypass this option. Only a semantic left click is eligible.
pub fn raise_floating_on_client_click(
    ctx: &mut crate::contexts::WmCtx,
    win: WindowId,
    button: MouseButton,
) {
    if button != MouseButton::Left || !ctx.core().config().window.raise_floating_on_click {
        return;
    }
    let should_raise = ctx.core().model().client_view(win).is_some_and(|view| {
        view.client.mode().is_free_positioned() || !view.monitor.is_tiling_layout()
    });
    if should_raise {
        ctx.raise_client(win);
    }
}

/// Common hover-focus guard checks shared by both backends.
///
/// Returns `true` when hover focus should proceed for `hovered_win`.
fn should_hover_focus(
    model: &crate::model::WmModel,
    window: &crate::core_state::WindowConfig,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) -> bool {
    let Some(win) = hovered_win else {
        return false;
    };
    // Already focused — nothing to do.
    if model.selected_win() == Some(win) {
        return false;
    }
    // Respect the "don't focus floating windows on hover" setting.
    let hovered_is_floating = model
        .client(win)
        .map(|c| c.placement() == ClientPlacement::Floating)
        .unwrap_or(false);
    let has_tiling = model.expect_selected_monitor().is_tiling_layout();
    if !window.focus_follows_float_mouse && hovered_is_floating && has_tiling && !entering_root {
        return false;
    }
    true
}

/// Switch the selected monitor to `monitor_id` and re-focus the target.
///
/// Returns `true` if the selection actually changed (i.e. the monitor was not
/// already selected), `false` otherwise.
pub fn select_monitor(ctx: &mut crate::contexts::WmCtx, monitor_id: MonitorId) -> bool {
    if !ctx.core().model().can_change_selected_monitor(monitor_id) {
        return false;
    }

    if ctx.core().model().is_overview_active() {
        crate::overview::exit_overview(ctx, crate::overview::ExitMode::RestorePrevious);
    }

    let previous_focus = ctx.core().model().selected_win();
    let selected = ctx.core_mut().select_monitor(monitor_id);
    debug_assert!(selected);
    ctx.update_ewmh_desktop_props();
    // Project from the focus that existed before the monitor transaction,
    // rather than trying to reconstruct it from the destination monitor.
    focus_impl(ctx, None, previous_focus, BackendRefresh::Force);
    true
}

/// Make the output that owns `win` the current monitor.
///
/// This is deliberately not the same as selecting the client. [`select_monitor`]
/// forwards a `None` focus target, so the destination output's own previously
/// selected window takes focus; the caller asked to *go to* the output a window
/// lives on (workspace-switch semantics, e.g. an IPC request naming a window),
/// not to raise that window. Use [`focus`] when the window itself should be the
/// one focused.
pub fn select_monitor_for_client(ctx: &mut crate::contexts::WmCtx, win: WindowId) -> bool {
    let Some(monitor_id) = ctx.core().model().monitor_of_client(win) else {
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
        .client_view(win)
        .map(|view| (view.monitor.id(), view.client.tags))
    else {
        return false;
    };

    select_monitor(ctx, monitor_id);

    let target_tags = client_tags.without_scratchpad();
    let visible_tags = ctx.core().model().expect_selected_monitor().visible_tags();
    if !target_tags.is_empty() && !target_tags.intersects(visible_tags) {
        crate::tags::view::view_tags(ctx, target_tags);
    }

    // Activation is an explicit request to surface the window even when the
    // model already names it as selected. Unlike ordinary focus, it therefore
    // raises persistently (within the window's policy layer).
    refresh_focus(ctx, Some(win));
    ctx.raise_client(win);
    true
}

pub fn select_monitor_at_pointer(ctx: &mut crate::contexts::WmCtx, pointer_pos: Point) -> bool {
    let Some(monitor) = ctx.core().model().monitors.monitor_at_pointer(pointer_pos) else {
        return false;
    };
    select_monitor(ctx, monitor.id())
}

fn get_directional_candidate(
    monitor: &Monitor,
    selected_tags: TagMask,
    source_win: WindowId,
    source_center: crate::types::Point,
    direction: Direction,
) -> Option<WindowId> {
    crate::types::OrderedClients::new(monitor.focus_order(), monitor.clients())
        .filter(|(win, client)| {
            if *win == source_win || !client.is_visible(selected_tags) {
                return false;
            }
            let center = client.geo.center();
            match direction {
                Direction::Up => center.y < source_center.y,
                Direction::Down => center.y > source_center.y,
                Direction::Left => center.x < source_center.x,
                Direction::Right => center.x > source_center.x,
            }
        })
        .min_by_key(|(_, client)| {
            let center = client.geo.center();
            let dx = center.abs_diff_x(&source_center);
            let dy = center.abs_diff_y(&source_center);
            match direction {
                Direction::Up | Direction::Down if dx > dy => i32::MAX,
                Direction::Left | Direction::Right if dy > dx => i32::MAX,
                Direction::Up | Direction::Down => dx + dy / 4,
                Direction::Left | Direction::Right => dy + dx / 4,
            }
        })
        .map(|(win, _)| win)
}

/// Shared logic for directional focus - finds the candidate window.
fn get_direction_focus_candidate(
    model: &crate::model::WmModel,
    direction: Direction,
) -> Option<WindowId> {
    if model.monitors.is_empty() {
        return None;
    }
    let mon = model.expect_selected_monitor();
    let source_win = mon.selected?;
    let source_client = model.client(source_win)?;
    let source_center = source_client.geo.center();

    let selected = mon.visible_tags();

    get_directional_candidate(mon, selected, source_win, source_center, direction)
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
        let name = last_client
            .scratchpad()
            .expect("is_scratchpad() implies scratchpad data is present")
            .name()
            .to_string();
        let _ = crate::floating::scratchpad_show_name(ctx, &name);
        return;
    }

    let tags = last_client.tags;
    let Some(last_mon_id) = ctx.core().model().monitor_of_client(last_win) else {
        return;
    };

    let sel_mon_id = ctx.core().model().selected_monitor_id();
    if !ctx.core().model().monitors.is_empty() && sel_mon_id != last_mon_id {
        select_monitor(ctx, last_mon_id);
    }

    if let Some(cur) = ctx.core().model().selected_win() {
        ctx.core_mut().focus.last_client = cur;
    }

    crate::tags::view::view_tags(ctx, tags);
    focus(ctx, Some(last_win));

    let monitor_id = ctx.core().model().selected_monitor_id();
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

/// Shared logic to compute the next stack index for focus.
fn stack_focus_target(
    stack: &[WindowId],
    selected_window: Option<WindowId>,
    direction: StackDirection,
    wrap: bool,
) -> Option<WindowId> {
    if stack.is_empty() {
        return None;
    }

    let Some(current_idx) = selected_window.and_then(|win| stack.iter().position(|&w| w == win))
    else {
        return if direction.is_forward() {
            stack.first().copied()
        } else {
            stack.last().copied()
        };
    };

    if direction.is_forward() {
        stack
            .get(current_idx + 1)
            .copied()
            .or_else(|| wrap.then(|| stack[0]))
    } else {
        current_idx
            .checked_sub(1)
            .and_then(|idx| stack.get(idx).copied())
            .or_else(|| wrap.then(|| stack[stack.len() - 1]))
    }
}

fn get_stack_focus_target(
    model: &crate::model::WmModel,
    direction: StackDirection,
    wrap: bool,
) -> Option<WindowId> {
    if model.monitors.is_empty() {
        return None;
    }
    let mon = model.expect_selected_monitor();
    let stack = mon.focus_cycle_order();

    let selected_window = model
        .selected_win()
        .filter(|win| stack.contains(win))
        .or_else(|| {
            mon.is_maximized_layout()
                .then(|| mon.most_recent_focus(mon.selected_tags(), |win| stack.contains(&win)))
                .flatten()
        });
    stack_focus_target(&stack, selected_window, direction, wrap)
}

/// Focus the best visible window in `direction`.
///
/// Returns whether focus moved.  Keeping the result explicit lets higher-level
/// navigation commands provide a boundary action (such as changing tags)
/// without having to repeat the candidate-selection logic.
pub fn direction_focus(ctx: &mut WmCtx, direction: Direction) -> bool {
    if let Some(target) = get_direction_focus_candidate(ctx.core().model(), direction) {
        focus(ctx, Some(target));
        true
    } else {
        false
    }
}

/// Focus the window at the far edge of the current tag, resolving a
/// directional focus step that ran off the edge instead of giving up.
///
/// This is the geometric counterpart of [`direction_focus`]: that one looks
/// for a neighbour in `direction` and reports failure at the boundary, this
/// one answers the boundary question. Running off the right edge lands on the
/// leftmost window, running off the top edge on the bottom-most, and so on.
///
/// Shared by both axes, but reached under different circumstances: horizontally
/// it is [`crate::config::config_toml::HorizontalEdge::Wrap`], vertically it is
/// the tiled half of [`crate::config::config_toml::VerticalEdge::Wrap`].
///
/// Returns whether focus moved.
pub fn wrap_direction_focus(ctx: &mut WmCtx, direction: Direction) -> bool {
    if let Some(target) = get_wrapping_candidate(ctx.core().model(), direction) {
        focus(ctx, Some(target));
        true
    } else {
        false
    }
}

fn get_wrapping_candidate(model: &crate::model::WmModel, direction: Direction) -> Option<WindowId> {
    if model.monitors.is_empty() {
        return None;
    }
    let mon = model.expect_selected_monitor();
    let source_win = mon.selected?;
    let source_client = model.client(source_win)?;
    let source_center = source_client.geo.center();

    let selected = mon.visible_tags();

    get_wrapping_window(mon, selected, source_win, source_center, direction)
}

/// Pick the window that a wrapped step should land on.
///
/// Candidates are ranked by the coordinate along `direction`, inverted so the
/// *far* edge sorts first: running off the right edge wants the smallest `x`,
/// running off the top edge the largest `y`. The secondary key mirrors
/// [`get_directional_candidate`], preferring candidates that are also close on
/// the cross axis so motion stays continuous when a whole row or column of
/// windows shares one coordinate.
fn get_wrapping_window(
    monitor: &Monitor,
    selected_tags: TagMask,
    source_win: WindowId,
    source_center: crate::types::Point,
    direction: Direction,
) -> Option<WindowId> {
    crate::types::OrderedClients::new(monitor.focus_order(), monitor.clients())
        .filter(|(win, client)| {
            if *win == source_win || !client.is_visible(selected_tags) {
                return false;
            }
            // A wrapped step still has to move *along* its axis. When every
            // visible window shares the source's coordinate there is no edge
            // to wrap around, and the honest answer is "nowhere" rather than a
            // cross-axis move dressed up as an along-axis one. Tiling tree
            // navigation usually intercepts a pure row/column before this is
            // reached, so treat it as a guard on the degenerate input rather
            // than a behaviour users are expected to hit.
            let center = client.geo.center();
            match direction {
                Direction::Left | Direction::Right => center.x != source_center.x,
                Direction::Up | Direction::Down => center.y != source_center.y,
            }
        })
        .min_by_key(|(_, client)| {
            let center = client.geo.center();
            let dx = center.abs_diff_x(&source_center);
            let dy = center.abs_diff_y(&source_center);
            // Running off the right edge wraps onto the leftmost window, off
            // the top edge onto the bottom-most. `min_by_key` needs a minimum
            // in every case, so negate the ones that sort farthest first —
            // widening first so no coordinate can overflow the inversion.
            let edge_rank = match direction {
                Direction::Right => i64::from(center.x),
                Direction::Left => -i64::from(center.x),
                Direction::Down => i64::from(center.y),
                Direction::Up => -i64::from(center.y),
            };
            let cross_axis_distance = match direction {
                Direction::Up | Direction::Down => dx + dy / 4,
                Direction::Left | Direction::Right => dy + dx / 4,
            };
            (edge_rank, cross_axis_distance)
        })
        .map(|(win, _)| win)
}

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    let _ = focus_stack_neighbor(ctx, direction, true);
}

/// Focus the adjacent window in stable stack/bar order.
///
/// `wrap` selects the boundary model: `false` stops at the outer edge so the
/// caller can run its own boundary policy, `true` cycles back to the other
/// end. Returns whether focus moved, so a caller can hand a `false` straight
/// to its boundary action without re-deriving why focus did not move.
pub fn focus_stack_neighbor(ctx: &mut WmCtx, direction: StackDirection, wrap: bool) -> bool {
    if let Some(target) = get_stack_focus_target(ctx.core().model(), direction, wrap) {
        focus(ctx, Some(target));
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
