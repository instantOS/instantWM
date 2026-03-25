//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `CoreCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::backend::BackendOps;
use crate::backend::x11::X11BackendRef;
use crate::client::{clear_urgency_hint_x11, set_focus_x11, unfocus_win_x11};
use crate::contexts::{CoreCtx, WaylandCtx, WmCtx};
use crate::types::*;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, InputFocus};

/// Result of resolving a focus target, containing both the target window
/// and information needed for state updates.
struct FocusTargetResult {
    target: Option<WindowId>,
    sel_mon_id: MonitorId,
    current_sel: Option<WindowId>,
}

/// Resolve the focus target based on the requested window and current state.
/// Returns `None` if there are no monitors (early exit case).
fn resolve_focus_target(core: &CoreCtx, win: Option<WindowId>) -> Option<FocusTargetResult> {
    if core.globals().monitors.is_empty() {
        return None;
    }

    let sel_mon_id = core.globals().selected_monitor_id();
    let mon = core.globals().selected_monitor();
    let selected = mon.selected_tag_mask();
    let current_sel = mon.sel;

    // Use the requested window if it's visible, otherwise walk the stack
    // to find the first visible non-hidden client.
    let mut target = win.filter(|w| {
        core.globals()
            .clients
            .get(w)
            .map(|c| c.is_visible_on_tags(selected) && !c.is_hidden)
            .unwrap_or(false)
    });

    if target.is_none() {
        // Try focus history first.
        if let Some(&hist_win) = mon.tag_focus_history.get(&selected.bits())
            && core
                .globals()
                .clients
                .get(&hist_win)
                .is_some_and(|c| c.is_visible_on_tags(selected) && !c.is_hidden)
        {
            target = Some(hist_win);
        }

        // Fallback to top of stack.
        if target.is_none() {
            target = mon.first_visible_client(core.globals().clients.map());
        }
    }

    Some(FocusTargetResult {
        target,
        sel_mon_id,
        current_sel,
    })
}

/// Update monitor state after focus target resolution.
/// Returns true if the selection state changed (focused <-> unfocused).
fn update_focus_state(core: &mut CoreCtx, result: FocusTargetResult) -> (Option<WindowId>, bool) {
    let FocusTargetResult {
        target,
        sel_mon_id,
        current_sel,
    } = result;

    let selection_state_changed = current_sel.is_none() != target.is_none();

    if let Some(mon) = core.globals_mut().monitor_mut(sel_mon_id) {
        mon.sel = target;
        if let Some(t) = target {
            mon.tag_focus_history.insert(mon.selected_tags().bits(), t);
        }
    }

    (target, selection_state_changed)
}

/// Backend-specific focus operations trait.
/// This allows the common focus logic to call backend-specific operations
/// without duplicating the surrounding logic.
trait FocusBackendOps {
    /// Unfocus the current window (if any) without focusing a new one.
    fn unfocus_current(&self, core: &mut CoreCtx, current: WindowId);
    /// Focus a specific window.
    fn focus_window(&self, core: &mut CoreCtx, win: WindowId);
    /// Handle the case where no window should be focused (focus root/nothing).
    fn focus_none(&self, core: &mut CoreCtx);
    /// Called when selection state changes (focused <-> unfocused).
    fn on_selection_changed(&self, core: &mut CoreCtx);
    /// Return `true` when the backend's seat focus is out of sync with the
    /// requested target and needs to be re-applied even though the WM-level
    /// selection (`mon.sel`) did not change.
    fn needs_focus_refresh(&self, _target: Option<WindowId>) -> bool {
        false
    }
}

struct X11FocusBackend<'a> {
    x11: &'a X11BackendRef<'a>,
    x11_runtime: &'a mut crate::backend::x11::X11RuntimeConfig,
}

impl<'a> FocusBackendOps for X11FocusBackend<'a> {
    fn unfocus_current(&self, core: &mut CoreCtx, current: WindowId) {
        unfocus_win_x11(core, self.x11, &*self.x11_runtime, current, false);
    }

    fn focus_window(&self, core: &mut CoreCtx, win: WindowId) {
        let is_urgent = core
            .globals()
            .clients
            .get(&win)
            .map(|c| c.is_urgent)
            .unwrap_or(false);
        if is_urgent {
            if let Some(c) = core.globals_mut().clients.get_mut(&win) {
                c.clear_urgency();
            }
            clear_urgency_hint_x11(self.x11, win);
        }
        set_focus_x11(core, self.x11, &*self.x11_runtime, win);
    }

    fn focus_none(&self, _core: &mut CoreCtx) {
        let _ = self.x11.conn.set_input_focus(
            InputFocus::POINTER_ROOT,
            self.x11_runtime.root,
            CURRENT_TIME,
        );
        let _ = self.x11.conn.delete_property(
            self.x11_runtime.root,
            self.x11_runtime.netatom.active_window,
        );
        let _ = self.x11.conn.flush();
    }

    fn on_selection_changed(&self, core: &mut CoreCtx) {
        crate::keyboard::grab_keys_x11(core, self.x11, &*self.x11_runtime);
    }
}

struct WaylandFocusBackend<'a> {
    wayland: &'a WaylandCtx<'a>,
}

impl<'a> FocusBackendOps for WaylandFocusBackend<'a> {
    fn unfocus_current(&self, _core: &mut CoreCtx, _current: WindowId) {
        // Wayland doesn't need explicit unfocus - focus is managed by the backend
    }

    fn focus_window(&self, core: &mut CoreCtx, win: WindowId) {
        let is_urgent = core
            .globals()
            .clients
            .get(&win)
            .map(|c| c.is_urgent)
            .unwrap_or(false);
        if is_urgent && let Some(c) = core.globals_mut().clients.get_mut(&win) {
            c.clear_urgency();
        }
        self.wayland.backend.set_focus(win);
    }

    fn focus_none(&self, _core: &mut CoreCtx) {
        self.wayland.backend.clear_keyboard_focus();
    }

    fn on_selection_changed(&self, _core: &mut CoreCtx) {
        // Wayland: key grabs not applicable; desktop bindings kept in core
    }

    fn needs_focus_refresh(&self, target: Option<WindowId>) -> bool {
        match target {
            Some(win) => !self.wayland.backend.is_keyboard_focused_on(win),
            None => false,
        }
    }
}

/// Outcome of a focus operation, used to decide whether a restack is needed.
pub(crate) struct FocusOutcome {
    /// `true` when `mon.sel` actually changed.
    changed: bool,
    /// The monitor that owns the new selection.
    monitor_id: MonitorId,
}

/// Generic focus implementation shared between X11 and Wayland.
fn focus_generic(
    core: &mut CoreCtx,
    win: Option<WindowId>,
    backend: &mut dyn FocusBackendOps,
) -> anyhow::Result<FocusOutcome> {
    let result = match resolve_focus_target(core, win) {
        Some(r) => r,
        None => {
            return Ok(FocusOutcome {
                changed: false,
                monitor_id: core.globals().selected_monitor_id(),
            });
        }
    };

    let current_sel = result.current_sel;
    let sel_mon_id = result.sel_mon_id;
    let (target, selection_state_changed) = update_focus_state(core, result);

    // Track the previously focused window for focus-last-client.
    // This is done in the shared path so both backends behave identically.
    if current_sel != target
        && let Some(cur_win) = current_sel
    {
        core.focus.last_client = cur_win;
        backend.unfocus_current(core, cur_win);
    }

    if selection_state_changed {
        backend.on_selection_changed(core);
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
        backend.focus_none(core);
    }

    Ok(FocusOutcome {
        changed: focus_changed,
        monitor_id: sel_mon_id,
    })
}

/// Set focus to a window, or to the root if None.
///
/// # Errors
/// Returns an error if X11 operations fail (e.g., connection lost).
pub fn focus_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::backend::x11::X11RuntimeConfig,
    win: Option<WindowId>,
) -> anyhow::Result<FocusOutcome> {
    let mut backend = X11FocusBackend { x11, x11_runtime };
    focus_generic(core, win, &mut backend)
}

/// Wayland focus implementation: pick a target window, update mon.sel,
/// tell the backend, and redraw bars.
pub fn focus_wayland(
    core: &mut CoreCtx,
    wayland: &WaylandCtx,
    win: Option<WindowId>,
) -> anyhow::Result<FocusOutcome> {
    let mut backend = WaylandFocusBackend { wayland };
    focus_generic(core, win, &mut backend)
}

/// Best-effort X11 focus helper for legacy call sites.
pub(crate) fn focus_soft_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::backend::x11::X11RuntimeConfig,
    win: Option<WindowId>,
) {
    if let Err(e) = focus_x11(core, x11, x11_runtime, win) {
        log::warn!("focus_x11({:?}) failed: {}", win, e);
    }
}

/// Best-effort focus - backend-agnostic entry point.
///
/// Updates `mon.sel`, backend seat focus, and — when the selection actually
/// changed — restacks the affected monitor so that Z-order stays in sync.
/// This is critical for overlapping layouts (monocle, floating) where the
/// focused window must be visually on top.
pub fn focus_soft(ctx: &mut crate::contexts::WmCtx, win: Option<WindowId>) {
    use crate::contexts::WmCtx::*;
    let outcome = match ctx {
        X11(x11_ctx) => {
            match focus_x11(&mut x11_ctx.core, &x11_ctx.x11, x11_ctx.x11_runtime, win) {
                Ok(o) => o,
                Err(e) => {
                    log::warn!("focus_soft X11({:?}) failed: {}", win, e);
                    return;
                }
            }
        }
        Wayland(wayland_ctx) => {
            match focus_wayland(&mut wayland_ctx.core, &wayland_ctx.wayland, win) {
                Ok(o) => o,
                Err(e) => {
                    log::warn!("focus_soft Wayland({:?}) failed: {}", win, e);
                    return;
                }
            }
        }
    };
    if outcome.changed {
        crate::layouts::restack(ctx, outcome.monitor_id);
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
            unfocus_win_x11(core, x11, x11_runtime, win, redirect_to_root);
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
/// handles `mon.sel`, backend seat focus, and restacking in one place.
pub fn hover_focus_target(
    ctx: &mut crate::contexts::WmCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    use crate::contexts::{WmCtx::*, WmCtxX11};

    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => {
            // X11 has extra pointer-query logic for monitor switching when
            // hovered_win is None, so it keeps its own path.
            hover_focus_target_x11(core, x11, x11_runtime, hovered_win, entering_root);
        }
        Wayland(_) => {
            if !should_hover_focus(ctx.core(), hovered_win, entering_root) {
                return;
            }
            // Switch monitor if the hovered window lives on a different one.
            if let Some(win) = hovered_win
                && let Some(mid) = ctx.core().globals().clients.monitor_id(win)
                && mid != ctx.core().globals().selected_monitor_id()
            {
                ctx.core_mut().globals_mut().set_selected_monitor(mid);
            }
            focus_soft(ctx, hovered_win);
        }
    }
}

/// Common hover-focus guard checks shared by both backends.
///
/// Returns `true` when hover focus should proceed for `hovered_win`.
fn should_hover_focus(core: &CoreCtx, hovered_win: Option<WindowId>, entering_root: bool) -> bool {
    let Some(win) = hovered_win else {
        return false;
    };
    if !core.globals().behavior.focus_follows_mouse {
        return false;
    }
    // Already focused — nothing to do.
    if core.selected_client() == Some(win) {
        return false;
    }
    // Respect the "don't focus floating windows on hover" setting.
    let hovered_is_floating = core
        .globals()
        .clients
        .get(&win)
        .map(|c| c.is_floating)
        .unwrap_or(false);
    let has_tiling = core.globals().selected_monitor().is_tiling_layout();
    if !core.globals().behavior.focus_follows_float_mouse
        && hovered_is_floating
        && has_tiling
        && !entering_root
    {
        return false;
    }
    true
}

/// Backend-agnostic cursor query for hover logic.
pub fn cursor_client(ctx: &crate::contexts::WmCtx) -> Option<WindowId> {
    use crate::contexts::{WmCtx::*, WmCtxX11};
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => crate::backend::x11::mouse::get_cursor_client_win_with_conn(
            core,
            x11.conn,
            x11_runtime.root,
        ),
        Wayland(_) => None,
    }
}

/// X11 hover-focus implementation matching the enter-notify focus path.
pub fn hover_focus_target_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::backend::x11::X11RuntimeConfig,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    if !core.globals().behavior.focus_follows_mouse {
        return;
    }

    if let Some(win) = hovered_win {
        if let Some(mid) = core.globals().clients.monitor_id(win)
            && mid != core.globals().selected_monitor_id()
        {
            core.globals_mut().set_selected_monitor(mid);
            let _ = focus_x11(core, x11, x11_runtime, None);
            return;
        }

        let hovered_is_floating = core
            .globals()
            .clients
            .get(&win)
            .map(|c| c.is_floating)
            .unwrap_or(false);
        let has_tiling = core.globals().selected_monitor().is_tiling_layout();
        if !core.globals().behavior.focus_follows_float_mouse
            && hovered_is_floating
            && has_tiling
            && !entering_root
        {
            return;
        }
    } else if let Ok(cookie) = x11rb::protocol::xproto::query_pointer(x11.conn, x11_runtime.root)
        && let Ok(reply) = cookie.reply()
    {
        let ptr = (reply.root_x as i32, reply.root_y as i32);
        if let Some(new_mon_id) = core.globals().monitors.find_monitor_at_pointer(ptr)
            && new_mon_id != core.globals().selected_monitor_id()
        {
            core.globals_mut().set_selected_monitor(new_mon_id);
            let _ = focus_x11(core, x11, x11_runtime, None);
            return;
        }
    }

    let _ = focus_x11(core, x11, x11_runtime, hovered_win);
}

/// Focus a client in the given direction.
///
/// This function uses dependency injection by accepting explicit parameters
/// instead of accessing global state directly.
///
/// # Arguments
/// * `monitors` - Slice of all monitors
/// * `sel_mon_id` - Currently selected monitor ID
/// * `clients` - Reference to all clients
/// * `direction` - Direction to search for a client
/// * `focus_fn` - Function to call with the target window
pub fn focus_direction<F>(core: &CoreCtx, direction: Direction, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let mon = core.globals().selected_monitor();

    let selected = mon.selected_tag_mask();

    let Some(source_win) = mon.sel else {
        focus_fn(None);
        return;
    };

    let Some(source_client) = core.globals().clients.get(&source_win) else {
        focus_fn(None);
        return;
    };

    let (source_center_x, source_center_y) = source_client.geo.center();

    let candidates = get_directional_candidates(
        &mon.clients,
        core.globals().clients.map(),
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    );

    focus_fn(candidates);
}

fn get_directional_candidates(
    clients: &[WindowId],
    globals_map: &std::collections::HashMap<WindowId, Client>,
    selected_tags: TagMask,
    source_win: WindowId,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> Option<WindowId> {
    let mut out_client: Option<WindowId> = None;
    let mut min_score: i32 = 0;

    for (c_win, c) in crate::types::ClientListIter::new(clients, globals_map) {
        if !c.is_visible_on_tags(selected_tags) {
            continue;
        }

        let center_x = c.geo.x + c.geo.w / 2;
        let center_y = c.geo.y + c.geo.h / 2;

        if is_client_in_direction(
            c_win,
            source_win,
            center_x,
            center_y,
            source_center_x,
            source_center_y,
            direction,
        ) {
            let score = calculate_direction_score(
                center_x,
                center_y,
                source_center_x,
                source_center_y,
                direction,
            );
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
    center_x: i32,
    center_y: i32,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> bool {
    if c_win == source_win {
        return false;
    }

    match direction {
        Direction::Up => center_y < source_center_y,
        Direction::Down => center_y > source_center_y,
        Direction::Left => center_x < source_center_x,
        Direction::Right => center_x > source_center_x,
    }
}

fn calculate_direction_score(
    center_x: i32,
    center_y: i32,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> i32 {
    let dist_x = (source_center_x - center_x).abs();
    let dist_y = (source_center_y - center_y).abs();

    match direction {
        Direction::Up | Direction::Down => {
            if dist_x > dist_y {
                return i32::MAX;
            }
            dist_x + dist_y / 4
        }
        Direction::Left | Direction::Right => {
            if dist_y > dist_x {
                return i32::MAX;
            }
            dist_y + dist_x / 4
        }
    }
}

/// Shared logic for directional focus - finds the candidate window.
fn get_direction_focus_candidate(core: &CoreCtx, direction: Direction) -> Option<WindowId> {
    if core.globals().monitors.is_empty() {
        return None;
    }
    let mon = core.globals().selected_monitor();
    let source_win = mon.sel?;
    let source_client = core.globals().clients.get(&source_win)?;
    let (source_center_x, source_center_y) = source_client.geo.center();

    let selected = mon.selected_tag_mask();

    get_directional_candidates(
        &mon.clients,
        core.globals().clients.map(),
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    )
}

pub fn focus_last_client(ctx: &mut WmCtx) {
    let last_client_win = ctx.core().focus.last_client;
    if last_client_win == WindowId::default() {
        return;
    }
    let last_win = last_client_win;

    let last_client = match ctx.client(last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        let _ = crate::floating::scratchpad_show_name(ctx, &last_client.scratchpad_name);
        return;
    }

    let tags = crate::types::TagMask::from_bits(last_client.tags);
    let last_mon_id = last_client.monitor_id;

    let sel_mon_id = ctx.core().globals().selected_monitor_id();
    if !ctx.core().globals().monitors.is_empty()
        && sel_mon_id != last_mon_id
        && let Some(sel) = ctx.core().globals().monitor(sel_mon_id).and_then(|m| m.sel)
    {
        unfocus_win(ctx, sel, false);
        ctx.core_mut()
            .globals_mut()
            .set_selected_monitor(last_mon_id);
    }

    if let Some(cur) = ctx.selected_client() {
        ctx.core_mut().focus.last_client = cur;
    }

    crate::tags::view::view(ctx, tags);
    focus_soft(ctx, Some(last_win));

    let monitor_id = ctx.core().globals().selected_monitor_id();
    crate::layouts::arrange(ctx, Some(monitor_id));
}

/// Focus the next or previous client in the stack.
pub fn focus_stack_direction<F>(core: &CoreCtx, forward: bool, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let target = get_stack_focus_target(
        core,
        if forward {
            StackDirection::Next
        } else {
            StackDirection::Previous
        },
    );
    focus_fn(target);
}

fn get_visible_stack(
    mon: &Monitor,
    clients: &std::collections::HashMap<WindowId, Client>,
) -> Vec<WindowId> {
    let mut stack = Vec::new();
    let selected = mon.selected_tag_mask();

    for (c_win, c) in mon.iter_stack(clients) {
        if c.is_visible_on_tags(selected) {
            stack.push(c_win);
        }
    }

    stack
}

/// Shared logic to compute the next stack index for focus.
fn get_stack_focus_target(core: &CoreCtx, direction: StackDirection) -> Option<WindowId> {
    if core.globals().monitors.is_empty() {
        return None;
    }
    let mon = core.globals().selected_monitor();
    let stack = get_visible_stack(mon, core.globals().clients.map());

    if stack.is_empty() {
        return None;
    }

    let selected_window = core.selected_client();
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
    if let Some(target) = get_direction_focus_candidate(ctx.core(), direction) {
        focus_soft(ctx, Some(target));
    }
}

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    if let Some(target) = get_stack_focus_target(ctx.core(), direction) {
        focus_soft(ctx, Some(target));
    }
}
