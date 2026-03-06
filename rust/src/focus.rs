//! Focus management using explicit WM context.
//!
//! This module provides window focus functionality via `CoreCtx`, avoiding
//! global state access and making dependencies explicit.

use crate::backend::x11::X11BackendRef;
use crate::backend::BackendOps;
use crate::client::{set_focus_x11, set_urgent, unfocus_win_x11};
use crate::contexts::{CoreCtx, WaylandCtx, WmCtx};
use crate::globals::X11RuntimeConfig;
use crate::mouse::{get_cursor_client_win_x11, warp as mouse_warp};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, InputFocus, PropMode, Window};
use x11rb::wrapper::ConnectionExt as ConnectionExtWrapper;
use x11rb::CURRENT_TIME;

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
    if core.g.monitors.is_empty() {
        return None;
    }

    let sel_mon_id = core.g.selected_monitor_id();
    let mon = core.g.selected_monitor();
    let selected = mon.selected_tag_mask();
    let current_sel = mon.sel;

    // Use the requested window if it's visible, otherwise walk the stack
    // to find the first visible non-hidden client.
    let mut target = win.filter(|w| {
        core.g
            .clients
            .get(w)
            .map(|c| c.is_visible_on_tags(selected.bits()) && !c.is_hidden)
            .unwrap_or(false)
    });

    if target.is_none() {
        for &c_win in &mon.stack {
            let Some(c) = core.g.clients.get(&c_win) else {
                continue;
            };
            if c.is_visible_on_tags(selected.bits()) && !c.is_hidden {
                target = Some(c_win);
                break;
            }
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

    if let Some(mon) = core.g.monitor_mut(sel_mon_id) {
        mon.sel = target;
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
    /// Called after focus state is updated, before focusing a window.
    fn post_state_update(&self, core: &mut CoreCtx);
}

struct X11FocusBackend<'a> {
    x11: &'a X11BackendRef<'a>,
    x11_runtime: &'a mut crate::globals::X11RuntimeConfig,
    systray: Option<&'a crate::types::Systray>,
}

impl<'a> FocusBackendOps for X11FocusBackend<'a> {
    fn unfocus_current(&self, core: &mut CoreCtx, current: WindowId) {
        unfocus_win_x11(core, self.x11, &*self.x11_runtime, current, false);
    }

    fn focus_window(&self, core: &mut CoreCtx, win: WindowId) {
        let is_urgent = core
            .g
            .clients
            .get(&win)
            .map(|c| c.isurgent)
            .unwrap_or(false);
        if is_urgent {
            set_urgent(core, self.x11, win, false);
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

    fn post_state_update(&self, core: &mut CoreCtx) {
        core.bar.mark_dirty();
        crate::bar::draw_bars_x11(core, self.x11, self.x11_runtime, self.systray);
    }
}

struct WaylandFocusBackend<'a> {
    wayland: &'a WaylandCtx<'a>,
}

impl<'a> FocusBackendOps for WaylandFocusBackend<'a> {
    fn unfocus_current(&self, _core: &mut CoreCtx, _current: WindowId) {
        // Wayland doesn't need explicit unfocus - focus is managed by the backend
    }

    fn focus_window(&self, _core: &mut CoreCtx, win: WindowId) {
        self.wayland.backend.set_focus(win);
    }

    fn focus_none(&self, _core: &mut CoreCtx) {
        // Wayland: no explicit root focus needed
    }

    fn on_selection_changed(&self, _core: &mut CoreCtx) {
        // Wayland: key grabs not applicable; desktop bindings kept in core
    }

    fn post_state_update(&self, core: &mut CoreCtx) {
        core.bar.mark_dirty();
    }
}

/// Generic focus implementation shared between X11 and Wayland.
fn focus_generic(
    core: &mut CoreCtx,
    win: Option<WindowId>,
    backend: &dyn FocusBackendOps,
) -> anyhow::Result<()> {
    let result = match resolve_focus_target(core, win) {
        Some(r) => r,
        None => return Ok(()),
    };

    let current_sel = result.current_sel;
    let (target, selection_state_changed) = update_focus_state(core, result);

    // Unfocus the previous window if target changed
    if current_sel != target {
        if let Some(cur_win) = current_sel {
            backend.unfocus_current(core, cur_win);
        }
    }

    if selection_state_changed {
        backend.on_selection_changed(core);
    }

    backend.post_state_update(core);

    if let Some(w) = target {
        backend.focus_window(core, w);
    } else {
        backend.focus_none(core);
    }

    Ok(())
}

/// Set focus to a window, or to the root if None.
///
/// # Errors
/// Returns an error if X11 operations fail (e.g., connection lost).
pub fn focus_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::globals::X11RuntimeConfig,
    systray: Option<&crate::types::Systray>,
    win: Option<WindowId>,
) -> anyhow::Result<()> {
    let backend = X11FocusBackend {
        x11,
        x11_runtime,
        systray,
    };
    focus_generic(core, win, &backend)
}

/// Wayland focus implementation: pick a target window, update mon.sel,
/// tell the backend, and redraw bars.
pub fn focus_wayland(
    core: &mut CoreCtx,
    wayland: &WaylandCtx,
    win: Option<WindowId>,
) -> anyhow::Result<()> {
    let backend = WaylandFocusBackend { wayland };
    focus_generic(core, win, &backend)
}

/// Best-effort X11 focus helper for legacy call sites.
pub fn focus_soft_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::globals::X11RuntimeConfig,
    win: Option<WindowId>,
) {
    if let Err(e) = focus_x11(core, x11, x11_runtime, None, win) {
        log::warn!("focus_x11({:?}) failed: {}", win, e);
    }
}

/// Best-effort focus - backend-agnostic entry point.
///
/// Calls the appropriate backend focus function and logs any errors,
/// but does not propagate them. This is suitable for use in event
/// handlers where focus failures should not abort the operation.
pub fn focus_soft(ctx: &mut crate::contexts::WmCtx, win: Option<WindowId>) {
    use crate::contexts::WmCtx::*;
    match ctx {
        X11(x11_ctx) => {
            let systray = x11_ctx.systray.as_deref();
            if let Err(e) = focus_x11(
                &mut x11_ctx.core,
                &x11_ctx.x11,
                x11_ctx.x11_runtime,
                systray,
                win,
            ) {
                log::warn!("focus_soft X11({:?}) failed: {}", win, e);
            }
        }
        Wayland(wayland_ctx) => {
            if let Err(e) = focus_wayland(&mut wayland_ctx.core, &wayland_ctx.wayland, win) {
                log::warn!("focus_soft Wayland({:?}) failed: {}", win, e);
            }
        }
    }
}

/// Backend-agnostic unfocus - does match internally.
///
/// For X11: calls unfocus_win_x11 (resets border, releases buttons, clears focus).
/// For Wayland: currently just tracks last_client (border/focus handled differently).
pub fn unfocus_win(ctx: &mut crate::contexts::WmCtx, win: WindowId, redirect_to_root: bool) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => {
            unfocus_win_x11(core, x11, x11_runtime, win, redirect_to_root);
        }
        Wayland(WmCtxWayland { core, .. }) => {
            core.focus.last_client = win;
        }
    }
}

/// Backend-agnostic hover-focus entry point.
pub fn hover_focus_target(
    ctx: &mut crate::contexts::WmCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => {
            hover_focus_target_x11(core, x11, x11_runtime, hovered_win, entering_root);
        }
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            hover_focus_target_wayland(core, wayland, hovered_win, entering_root);
        }
    }
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
        }) => get_cursor_client_win_x11(core, x11, x11_runtime),
        Wayland(_) => None,
    }
}

/// X11 hover-focus implementation matching the enter-notify focus path.
pub fn hover_focus_target_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::globals::X11RuntimeConfig,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    if !core.g.focusfollowsmouse {
        return;
    }

    if let Some(win) = hovered_win {
        if let Some(mid) = core.g.clients.get(&win).and_then(|c| c.monitor_id) {
            if mid != core.g.selected_monitor_id() {
                core.g.set_selected_monitor(mid);
                let _ = focus_x11(core, x11, x11_runtime, None, None);
                return;
            }
        }

        let hovered_is_floating = core
            .g
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false);
        let has_tiling = core.g.selected_monitor().is_tiling_layout();
        if !core.g.focusfollowsfloatmouse && hovered_is_floating && has_tiling && !entering_root {
            return;
        }
    } else {
        let event_win = WindowId::from(x11_runtime.root);
        if let Some(new_mon_id) = core.g.monitors.win_to_mon(
            event_win,
            x11_runtime.root,
            &*core.g.clients,
            Some(X11BackendRef::new(x11.conn, x11.screen_num)),
        ) {
            if new_mon_id != core.g.selected_monitor_id() {
                core.g.set_selected_monitor(new_mon_id);
                let _ = focus_x11(core, x11, x11_runtime, None, None);
                return;
            }
        }
    }

    let _ = focus_x11(core, x11, x11_runtime, None, hovered_win);
}

/// Shared hover-focus behavior used by both X11 and Wayland pointer paths.
pub fn hover_focus_target_wayland(
    core: &mut CoreCtx,
    wayland: &WaylandCtx,
    hovered_win: Option<WindowId>,
    entering_root: bool,
) {
    let Some(hovered_win) = hovered_win else {
        return;
    };
    if !core.g.focusfollowsmouse {
        return;
    }

    if let Some(mid) = core.g.clients.get(&hovered_win).and_then(|c| c.monitor_id) {
        if mid != core.g.selected_monitor_id() {
            core.g.set_selected_monitor(mid);
        }
    }

    let hovered_is_floating = core
        .g
        .clients
        .get(&hovered_win)
        .map(|c| c.isfloating)
        .unwrap_or(false);
    let has_tiling = core.g.selected_monitor().is_tiling_layout();
    if !core.g.focusfollowsfloatmouse && hovered_is_floating && has_tiling && !entering_root {
        return;
    }

    if core.selected_client() == Some(hovered_win) {
        return;
    }

    core.set_selected_client(Some(hovered_win));
    wayland.backend.set_focus(hovered_win);
    let _ = core;
}

pub fn set_focus_win_x11(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &crate::globals::X11RuntimeConfig,
    win: WindowId,
) {
    let x11_win: Window = win.into();
    if let Some(c) = core.g.clients.get(&win) {
        if !c.neverfocus {
            let _ = x11
                .conn
                .set_input_focus(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
            let _ = x11.conn.change_property32(
                PropMode::REPLACE,
                x11_runtime.root,
                x11_runtime.netatom.active_window,
                AtomEnum::WINDOW,
                &[x11_win],
            );
        }
        let _ = x11.conn.flush();
    }
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
    let mon = core.g.selected_monitor();

    let selected = mon.selected_tag_mask();

    let Some(source_win) = mon.sel else {
        focus_fn(None);
        return;
    };

    let Some(source_client) = core.g.clients.get(&source_win) else {
        focus_fn(None);
        return;
    };

    let (source_center_x, source_center_y) = source_client.geo.center();

    let candidates = get_directional_candidates(
        &mon.clients,
        &*core.g.clients,
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    );

    focus_fn(candidates);
}

fn get_directional_candidates(
    clients: &Vec<WindowId>,
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
        if !c.is_visible_on_tags(selected_tags.bits()) {
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
    if core.g.monitors.is_empty() {
        return None;
    }
    let mon = core.g.selected_monitor();
    let source_win = mon.sel?;
    let source_client = core.g.clients.get(&source_win)?;
    let (source_center_x, source_center_y) = source_client.geo.center();

    let selected = mon.selected_tag_mask();

    get_directional_candidates(
        &mon.clients,
        &*core.g.clients,
        selected,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    )
}

pub fn direction_focus_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::globals::X11RuntimeConfig,
    direction: Direction,
) {
    if let Some(target) = get_direction_focus_candidate(core, direction) {
        let _ = focus_x11(core, x11, x11_runtime, None, Some(target));
    }
}

pub fn focus_last_client(ctx: &mut WmCtx) {
    let last_client_win = ctx.core().focus.last_client;
    if last_client_win == WindowId::default() {
        return;
    }
    let last_win = last_client_win;

    let last_client = match ctx.g().clients.get(&last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        crate::scratchpad::scratchpad_show_name(ctx, &last_client.scratchpad_name);
        return;
    }

    let tags = last_client.tags;
    let last_mon_id = last_client.monitor_id;

    if let Some(last_mid) = last_mon_id {
        let sel_mon_id = ctx.g().selected_monitor_id();
        if !ctx.g().monitors.is_empty() && sel_mon_id != last_mid {
            if let Some(sel) = ctx.g().monitor(sel_mon_id).and_then(|m| m.sel) {
                unfocus_win(ctx, sel, false);
                ctx.g_mut().set_selected_monitor(last_mid);
            }
        }
    }

    if let Some(cur) = ctx.selected_client() {
        ctx.core_mut().focus.last_client = cur;
    }

    crate::tags::view::view(ctx, TagMask::from_bits(tags));
    focus_soft(ctx, Some(last_win));

    let monitor_id = ctx.g().selected_monitor_id();
    crate::layouts::arrange(ctx, Some(monitor_id));
}

pub fn warp_cursor_to_client_x11(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    c_win: WindowId,
) {
    mouse_warp::warp_impl_x11(core, x11, x11_runtime, c_win);
}

pub fn warp_to_focus_x11(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &mut X11RuntimeConfig) {
    if let Some(win) = core.selected_client() {
        warp_cursor_to_client_x11(core, x11, x11_runtime, win);
    }
}

/// Focus the next or previous client in the stack.
pub fn focus_stack_direction<F>(core: &CoreCtx, forward: bool, focus_fn: F)
where
    F: FnOnce(Option<WindowId>),
{
    let mon = core.g.selected_monitor();

    let selected_window = mon.sel;
    let stack = get_visible_stack(mon, &*core.g.clients);

    if stack.is_empty() {
        focus_fn(None);
        return;
    }

    let current_idx = match selected_window {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if forward {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    focus_fn(Some(stack[next_idx]));
}

fn get_visible_stack(
    mon: &Monitor,
    clients: &std::collections::HashMap<WindowId, Client>,
) -> Vec<WindowId> {
    let mut stack = Vec::new();
    let selected = mon.selected_tag_mask();

    for (c_win, c) in mon.iter_stack(clients) {
        if c.is_visible_on_tags(selected.bits()) {
            stack.push(c_win);
        }
    }

    stack
}

/// Shared logic to compute the next stack index for focus.
fn get_stack_focus_target(core: &CoreCtx, direction: StackDirection) -> Option<WindowId> {
    if core.g.monitors.is_empty() {
        return None;
    }
    let mon = core.g.selected_monitor();
    let stack = get_visible_stack(mon, &*core.g.clients);

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

//TODO: this seems redundant, there is a backend agnostic focus method, and
//get_stack_focus_target is already agnostic
pub fn focus_stack_x11(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut crate::globals::X11RuntimeConfig,
    direction: StackDirection,
) {
    if let Some(target) = get_stack_focus_target(core, direction) {
        let _ = focus_x11(core, x11, x11_runtime, None, Some(target));
    }
}

pub fn direction_focus_wayland(core: &mut CoreCtx, wayland: &WaylandCtx, direction: Direction) {
    if let Some(target) = get_direction_focus_candidate(core, direction) {
        if let Err(e) = focus_wayland(core, wayland, Some(target)) {
            log::warn!("focus_wayland({:?}) failed: {}", target, e);
        }
    }
}

pub fn focus_stack_wayland(core: &mut CoreCtx, wayland: &WaylandCtx, direction: StackDirection) {
    if let Some(target) = get_stack_focus_target(core, direction) {
        if let Err(e) = focus_wayland(core, wayland, Some(target)) {
            log::warn!("focus_wayland({:?}) failed: {}", target, e);
        }
    }
}

pub fn direction_focus(ctx: &mut WmCtx, direction: Direction) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => direction_focus_x11(core, x11, x11_runtime, direction),
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            direction_focus_wayland(core, wayland, direction)
        }
    }
}

pub fn focus_stack(ctx: &mut WmCtx, direction: StackDirection) {
    use crate::contexts::{WmCtx::*, WmCtxWayland, WmCtxX11};
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => focus_stack_x11(core, x11, x11_runtime, direction),
        Wayland(WmCtxWayland { core, wayland, .. }) => {
            focus_stack_wayland(core, wayland, direction)
        }
    }
}
