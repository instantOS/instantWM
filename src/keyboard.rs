use std::rc::Rc;

use crate::contexts::WmCtx;
use crate::floating::{
    SnapDir, change_snap, reset_snap, save_floating_geometry, set_overlay_mode, toggle_floating,
    unhide_one,
};
use crate::focus::{direction_focus, focus_stack};

use crate::layouts::arrange;
use crate::types::*;
use crate::types::{Direction, StackDirection};

pub fn handle_keysym(ctx: &mut WmCtx, keysym: u32, mod_mask: u32) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = crate::util::clean_mask(mod_mask, numlockmask) as u16;

    let current_mode = ctx.core().globals().behavior.current_mode.clone();

    // Super + Escape always resets to default mode
    if !current_mode.is_empty()
        && current_mode != "default"
        && keysym == crate::config::keysyms::XK_ESCAPE
        && cleaned
            == crate::util::clean_mask(crate::config::keybindings::MODKEY, numlockmask) as u16
    {
        ctx.core_mut().globals_mut().behavior.current_mode = "default".to_string();
        ctx.request_bar_update(None);
        return true;
    }

    let mut transient = false;

    let action = if !current_mode.is_empty() && current_mode != "default" {
        // Look FIRST in mode-specific keybindings
        let mode_cfg = ctx.core().globals().cfg.modes.get(&current_mode);
        transient = mode_cfg.map(|m| m.transient).unwrap_or(false);

        mode_cfg
            .and_then(|mode| {
                mode.keybinds.iter().find(|key| {
                    keysym == key.keysym
                        && crate::util::clean_mask(key.mod_mask, numlockmask) as u16 == cleaned
                })
            })
            .map(|key| Rc::clone(&key.action))
            .or_else(|| {
                // Fallback to global/desktop bindings
                ctx.core()
                    .globals()
                    .cfg
                    .keys
                    .iter()
                    .find(|key| {
                        keysym == key.keysym
                            && crate::util::clean_mask(key.mod_mask, numlockmask) as u16 == cleaned
                    })
                    .or_else(|| {
                        ctx.core()
                            .globals()
                            .cfg
                            .desktop_keybinds
                            .iter()
                            .find(|key| {
                                keysym == key.keysym
                                    && crate::util::clean_mask(key.mod_mask, numlockmask) as u16
                                        == cleaned
                            })
                    })
                    .map(|key| Rc::clone(&key.action))
            })
    } else {
        // Normal mode
        ctx.core()
            .globals()
            .cfg
            .keys
            .iter()
            .find(|key| {
                keysym == key.keysym
                    && crate::util::clean_mask(key.mod_mask, numlockmask) as u16 == cleaned
            })
            .or_else(|| {
                if ctx.selected_client().is_none() {
                    ctx.core()
                        .globals()
                        .cfg
                        .desktop_keybinds
                        .iter()
                        .find(|key| {
                            keysym == key.keysym
                                && crate::util::clean_mask(key.mod_mask, numlockmask) as u16
                                    == cleaned
                        })
                } else {
                    None
                }
            })
            .map(|key| Rc::clone(&key.action))
    };

    if let Some(action) = action {
        action(ctx);
        if transient {
            ctx.core_mut().globals_mut().behavior.current_mode = "default".to_string();
            ctx.request_bar_update(None);
        }
        true
    } else {
        false
    }
}

pub fn up_press(ctx: &mut WmCtx) {
    let (selected_window, overlay_win, is_floating) = {
        let mon = ctx.core().globals().selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let is_floating = sel.is_some_and(|w| ctx.core().globals().clients.is_floating(w));
        (sel, overlay, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if selected_window == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Top);
        return;
    }

    if is_floating {
        toggle_floating(ctx);
        return;
    }

    if let Some(win) = selected_window {
        crate::client::hide(ctx, win);
    }
}

pub fn down_press(ctx: &mut WmCtx) {
    if unhide_one(ctx) {
        return;
    }

    let (selected_window, overlay_win, snap_status, is_floating) = {
        let mon = ctx.core().globals().selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let (snap_status, is_floating) = sel
            .and_then(|w| {
                ctx.core()
                    .globals()
                    .clients
                    .get(&w)
                    .map(|c| (c.snap_status, c.is_floating))
            })
            .unwrap_or((SnapPosition::None, false));
        (sel, overlay, snap_status, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if snap_status != SnapPosition::None {
        if let Some(win) = selected_window {
            reset_snap(ctx, win);
        }
        return;
    }

    if selected_window == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Bottom);
        return;
    }

    if !is_floating {
        toggle_floating(ctx);
    }
}

pub fn up_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = !ctx.core().globals().selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus(ctx, Direction::Up);
        return;
    }

    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.selected_client() {
            if let WmCtx::X11(x11_ctx) = ctx {
                crate::client::refresh_border_color_x11(
                    &x11_ctx.core,
                    &x11_ctx.x11,
                    x11_ctx.x11_runtime,
                    win,
                    false,
                );
            }
            change_snap(ctx, win, SnapDir::Up);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn down_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = !ctx.core().globals().selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus(ctx, Direction::Down);
        return;
    }

    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.selected_client() {
            change_snap(ctx, win, SnapDir::Down);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn space_toggle(ctx: &mut WmCtx) {
    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if !has_tiling {
        let Some(win) = ctx.selected_client() else {
            return;
        };

        let snap_status = {
            ctx.core()
                .globals()
                .clients
                .get(&win)
                .map(|c| c.snap_status)
                .unwrap_or(SnapPosition::None)
        };

        if snap_status != SnapPosition::None {
            reset_snap(ctx, win);
        } else {
            let border_width = ctx.core().globals().cfg.border_width_px;
            ctx.set_border(win, border_width);

            if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
                save_floating_geometry(client);
                client.snap_status = SnapPosition::Maximized;
            }

            let selmon_id = ctx.core().globals().selected_monitor_id();
            arrange(ctx, Some(selmon_id));
        }
    } else {
        toggle_floating(ctx);
    }
}
