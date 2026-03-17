//! Pointer drag handling (title drag, tag drag, resize drag).

use crate::types::AltCursor;
use crate::types::{MouseButton, ResizeDirection};
use crate::wm::Wm;

/// Get the active drag window (if any).
pub fn wayland_active_drag_window(wm: &Wm) -> Option<crate::types::WindowId> {
    if wm.g.drag.hover_resize.active {
        return Some(wm.g.drag.hover_resize.win);
    }
    if wm.g.drag.title.active {
        return Some(wm.g.drag.title.win);
    }
    None
}

/// Begin hover resize drag if applicable.
pub fn wayland_hover_resize_drag_begin(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
    button: MouseButton,
) -> bool {
    let target = match wayland_selected_resize_target_at(wm, root_x, root_y) {
        Some((win, _)) => win,
        None => return false,
    };

    wm.g.drag.hover_resize.active = true;
    wm.g.drag.hover_resize.win = target;
    wm.g.drag.hover_resize.button = button;

    // Bring window to front
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
        let _ = crate::focus::focus_wayland(&mut ctx.core, &ctx.wayland, Some(target));
    }

    true
}

/// Get resize target at given coordinates.
pub fn wayland_selected_resize_target_at(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
) -> Option<(crate::types::WindowId, ResizeDirection)> {
    let selected = wm.g.selected_win()?;

    // Only floating windows can be resized via edge hover
    let client = wm.g.clients.get(&selected)?;
    if !client.is_floating {
        return None;
    }

    // Only the selected window can be resized
    let ctx = wm.ctx();
    let (_, dir) = crate::mouse::hover::selected_hover_resize_target_at(&ctx, root_x, root_y)?;

    Some((selected, dir))
}

/// Update resize direction for selected window.
pub fn update_wayland_selected_resize_offer(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
) -> Option<crate::types::WindowId> {
    // Check if we're over an edge of the selected floating window
    let target = wayland_selected_resize_target_at(wm, root_x, root_y);

    match target {
        Some((win, dir)) => {
            wm.g.drag.resize_direction = Some(dir);
            Some(win)
        }
        None => {
            wm.g.drag.resize_direction = None;
            None
        }
    }
}

/// Clear hover resize state.
pub fn clear_wayland_hover_resize_offer(ctx: &mut crate::contexts::WmCtxWayland<'_>) {
    ctx.core.g.drag.resize_direction = None;
    ctx.core.g.behavior.cursor_icon = AltCursor::None;
}

/// Move bar hover state.
pub fn update_wayland_move_bar_hover(wm: &mut Wm, root_x: i32, root_y: i32, _button: u32) {
    // Don't process if this is a drag button
    if wm.g.drag.tag.active || wm.g.drag.title.active {
        return;
    }

    // For now, simplified - just trigger bar hover update
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
        crate::mouse::drag::update_bar_hover_simple(
            &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
            root_x,
            root_y,
        );
    }
}

/// Handle hover resize drag motion.
pub fn wayland_hover_resize_drag_motion(wm: &mut Wm, root_x: i32, root_y: i32) -> bool {
    if !wm.g.drag.hover_resize.active {
        return false;
    }

    let button = wm.g.drag.hover_resize.button;
    let win = wm.g.drag.hover_resize.win;

    // Check if mouse is still over the same edge
    let target = wayland_selected_resize_target_at(wm, root_x, root_y);

    let Some((target_win, _)) = target else {
        // No longer over valid edge - cancel drag
        wm.g.drag.hover_resize.active = false;
        wm.g.drag.hover_resize.win = crate::types::WindowId(0);
        return false;
    };

    if target_win != win {
        // Moved to different window's edge - cancel
        wm.g.drag.hover_resize.active = false;
        wm.g.drag.hover_resize.win = crate::types::WindowId(0);
        return false;
    }

    // Update position
    let drag = wm.g.drag.hover_resize.clone();
    wm.g.drag.hover_resize.last_root_x = root_x;
    wm.g.drag.hover_resize.last_root_y = root_y;

    if drag.move_mode {
        // Handle move mode
        let mut new_x = drag.win_start_geo.x + (root_x - drag.start_x);
        let mut new_y = drag.win_start_geo.y + (root_y - drag.start_y);

        let ctx = wm.ctx();
        if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
            crate::mouse::drag::snap_window_to_monitor_edges(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                drag.win,
                drag.win_start_geo.w.max(1),
                drag.win_start_geo.h.max(1),
                &mut new_x,
                &mut new_y,
            );

            crate::client::resize(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                drag.win,
                &crate::types::Rect {
                    x: new_x,
                    y: new_y,
                    w: drag.win_start_geo.w,
                    h: drag.win_start_geo.h,
                },
            );
        }
    } else {
        // Resize mode
        let dx = root_x - drag.last_root_x;
        let dy = root_y - drag.last_root_y;

        let mut new_w = drag.win_start_geo.w;
        let mut new_h = drag.win_start_geo.h;
        let (dx_mul, dy_mul) = drag.direction.warp_offset(new_w, new_h, 0);

        new_w += dx * dx_mul;
        new_h += dy * dy_mul;

        new_w = new_w.max(1);
        new_h = new_h.max(1);

        let ctx = wm.ctx();
        if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
            crate::client::resize(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                drag.win,
                &crate::types::Rect {
                    x: drag.win_start_geo.x,
                    y: drag.win_start_geo.y,
                    w: new_w,
                    h: new_h,
                },
            );
        }
    }

    true
}

/// Finish hover resize drag.
pub fn wayland_hover_resize_drag_finish(wm: &mut Wm, button: MouseButton) -> bool {
    if !wm.g.drag.hover_resize.active {
        return false;
    }

    if button != wm.g.drag.hover_resize.button {
        return false;
    }

    let win = wm.g.drag.hover_resize.win;

    // Clear drag state
    wm.g.drag.hover_resize.active = false;
    wm.g.drag.hover_resize.win = crate::types::WindowId(0);
    wm.g.drag.resize_direction = None;

    // Focus the window
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
        let _ = crate::focus::focus_wayland(&mut ctx.core, &ctx.wayland, Some(win));
    }

    true
}
