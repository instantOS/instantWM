//! Client visibility: mapping/unmapping windows and WM_STATE transitions.

use crate::backend::WindowOps;
use crate::contexts::{WmCtx, WmCtxWayland};
use crate::types::{ClientMode, Rect, WindowId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct VisibilityEntry {
    pub win: WindowId,
    pub rect: Rect,
    pub border_width: i32,
    pub mode: ClientMode,
    pub visible: bool,
}

/// Snapshot visibility policy without performing backend I/O.
pub(crate) fn visibility_plan(globals: &crate::globals::Globals) -> Vec<VisibilityEntry> {
    let mut plan = Vec::new();
    for mon in globals.monitors_iter_all() {
        let selected_tags = mon.selected_tags();
        for (win, client) in mon.iter_clients(globals.clients.map()) {
            plan.push(VisibilityEntry {
                win,
                rect: client.geo,
                border_width: client.border_width,
                mode: client.mode,
                visible: client.is_visible(selected_tags),
            });
        }
    }
    plan
}

// ---------------------------------------------------------------------------
// Recursive show/hide pass
// ---------------------------------------------------------------------------

/// Walk the client list, moving each client on- or off-screen.
///
/// Visible clients (those whose tag-set overlaps the monitor's selected tags)
/// are positioned at their stored geometry.  Invisible clients are moved
/// `2 * client_width` pixels to the left of the screen (i.e. off-screen left).
///
/// This mirrors the classic dwm `showhide` function and is called by the
/// arrange path after every layout change.
pub fn apply_visibility(ctx: &mut crate::contexts::WmCtx) {
    match ctx {
        crate::contexts::WmCtx::X11(ctx_x11) => {
            crate::backend::x11::visibility::apply_visibility_x11(ctx_x11);
        }
        crate::contexts::WmCtx::Wayland(ctx_wayland) => {
            apply_visibility_wayland(ctx_wayland);
        }
    }
}

pub fn apply_visibility_wayland(ctx: &mut WmCtxWayland<'_>) {
    for entry in visibility_plan(ctx.core.globals()) {
        if entry.visible {
            ctx.wayland.map_window(entry.win);
        } else {
            ctx.wayland.unmap_window(entry.win);
        }
    }
}

pub fn show_window(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(c) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        if !c.is_hidden {
            return;
        }
        c.is_hidden = false;
        c.monitor_id
    } else {
        return;
    };

    if let WmCtx::X11(ctx_x11) = ctx {
        crate::backend::x11::visibility::show_x11(ctx_x11, win);
    }

    crate::focus::focus(ctx, Some(win));
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

pub fn hide_for_user(ctx: &mut WmCtx, win: WindowId) {
    let scratchpad_name = ctx.core().globals().clients.get(&win).and_then(|c| {
        if c.is_scratchpad() {
            Some(c.scratchpad.as_ref().unwrap().name.clone())
        } else {
            None
        }
    });

    if let Some(name) = scratchpad_name {
        crate::floating::scratchpad_hide_name(ctx, &name);
    } else {
        hide(ctx, win);
    }
}

pub fn hide(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(c) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        if c.is_hidden {
            return;
        }
        let mid = c.monitor_id;

        match ctx {
            WmCtx::X11(ctx_x11) => {
                crate::backend::x11::visibility::hide_x11(ctx_x11, win);
            }
            WmCtx::Wayland(ctx_wl) => {
                hide_wayland(ctx_wl, win);
            }
        }

        if let Some(c_mut) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            c_mut.is_hidden = true;
        }

        mid
    } else {
        return;
    };

    let snext = ctx
        .core()
        .globals()
        .monitor(monitor_id)
        .and_then(|m| m.z_order.iter_top_to_bottom().find(|&w| w != win));
    crate::focus::focus(ctx, snext);
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

fn hide_wayland(ctx: &mut WmCtxWayland<'_>, win: WindowId) {
    ctx.wayland.unmap_window(win);
    ctx.wayland.flush();
}
