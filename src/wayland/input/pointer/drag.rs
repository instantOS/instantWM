//! Pointer drag handling (title drag, tag drag, resize drag).

use crate::contexts::WmCtxWayland;
use crate::geometry::MoveResizeOptions;
use crate::mouse::constants::RESIZE_BORDER_ZONE;
use crate::mouse::set_cursor_style;
use crate::types::{AltCursor, MouseButton, Rect, ResizeDirection, WindowId, get_resize_direction};
use crate::wm::Wm;

fn wayland_monitor_bar_visible(ctx: &crate::contexts::WmCtxWayland<'_>) -> bool {
    let mon = ctx.core.globals().selected_monitor();
    crate::bar::monitor_bar_visible(ctx.core.globals(), mon)
}

/// Get the active drag window (if any).
pub fn wayland_active_drag_window(wm: &Wm) -> Option<WindowId> {
    if wm.g.drag.interactive.active {
        return Some(wm.g.drag.interactive.win);
    }
    None
}

/// Begin hover resize drag if applicable.
pub fn wayland_hover_resize_drag_begin(
    ctx: &mut WmCtxWayland<'_>,
    root_x: i32,
    root_y: i32,
    btn: MouseButton,
) -> bool {
    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }
    let Some((win, dir, geo)) = wayland_selected_resize_target_at(ctx, root_x, root_y) else {
        return false;
    };
    let drag_type = if btn == MouseButton::Right
        || crate::mouse::hover::is_at_top_middle_edge(&geo, root_x, root_y)
    {
        crate::globals::DragType::Move
    } else {
        crate::globals::DragType::Resize(dir)
    };
    ctx.core.globals_mut().drag.interactive = crate::globals::DragInteraction {
        active: true,
        win,
        button: btn,
        dragging: true,
        drag_type,
        start_x: root_x,
        start_y: root_y,
        win_start_geo: geo,
        drop_restore_geo: geo,
        last_root_x: root_x,
        last_root_y: root_y,
        ..Default::default()
    };
    if matches!(drag_type, crate::globals::DragType::Resize(_)) {
        let _ = ctx.wayland.backend.with_state(|state| {
            state.begin_interactive_resize(win);
        });
    }
    match drag_type {
        crate::globals::DragType::Move => {
            set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                AltCursor::Move,
            );
        }
        crate::globals::DragType::Resize(dir) => {
            set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                AltCursor::Resize(dir),
            );
        }
    }
    let _ = crate::focus::focus_wayland(&mut ctx.core, &ctx.wayland, Some(win));
    crate::contexts::WmCtx::Wayland(ctx.reborrow()).raise_client(win);
    true
}

/// Get resize target at given coordinates.
fn wayland_selected_resize_target_at(
    ctx: &crate::contexts::WmCtxWayland<'_>,
    root_x: i32,
    root_y: i32,
) -> Option<(WindowId, ResizeDirection, Rect)> {
    let win = ctx.core.selected_client()?;
    let mon = ctx.core.globals().selected_monitor();
    if wayland_monitor_bar_visible(ctx) && root_y < mon.monitor_rect.y + mon.bar_height {
        return None;
    }
    let selected_tags = mon.selected_tags();
    let c = ctx.core.client(win)?;
    if !c.is_visible(selected_tags) {
        return None;
    }
    let has_tiling = mon.is_tiling_layout();
    if !c.is_floating && has_tiling {
        return None;
    }
    if !c
        .geo
        .contains_resize_border_point(root_x, root_y, RESIZE_BORDER_ZONE)
    {
        return None;
    }
    let hit_x = root_x - c.geo.x;
    let hit_y = root_y - c.geo.y;
    let dir = get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y);
    Some((win, dir, c.geo))
}

/// Update resize direction for selected window.
pub fn update_wayland_selected_resize_offer(
    ctx: &mut WmCtxWayland<'_>,
    root_x: i32,
    root_y: i32,
) -> Option<WindowId> {
    let Some((win, dir, _)) = wayland_selected_resize_target_at(ctx, root_x, root_y) else {
        if matches!(
            ctx.core.globals().behavior.cursor_icon,
            AltCursor::Resize(_)
        ) {
            set_cursor_style(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                AltCursor::Default,
            );
        }
        return None;
    };
    set_cursor_style(
        &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
        AltCursor::Resize(dir),
    );
    Some(win)
}

/// Update bar hover gesture highlighting during a Wayland move drag.
fn update_wayland_move_bar_hover(
    ctx: &mut crate::contexts::WmCtxWayland<'_>,
    root_x: i32,
    root_y: i32,
) -> bool {
    let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
    crate::mouse::drag::update_bar_hover_simple(&mut wm_ctx, root_x, root_y)
}

/// Handle interactive drag motion (move or resize) on Wayland.
///
/// This is the single motion handler for **all** active drags once
/// `dragging == true`, regardless of how the drag was initiated (title
/// bar, hover border, keyboard shortcut, Super+button, etc.).
pub fn wayland_hover_resize_drag_motion(
    ctx: &mut WmCtxWayland<'_>,
    root_x: i32,
    root_y: i32,
) -> bool {
    if !ctx.core.globals().drag.interactive.active || !ctx.core.globals().drag.interactive.dragging
    {
        return false;
    }
    let drag = ctx.core.globals().drag.interactive.clone();
    ctx.core.globals_mut().drag.interactive.last_root_x = root_x;
    ctx.core.globals_mut().drag.interactive.last_root_y = root_y;

    match drag.drag_type {
        crate::globals::DragType::Move => {
            let on_bar = update_wayland_move_bar_hover(ctx, root_x, root_y);

            let mut new_x = drag.win_start_geo.x + (root_x - drag.start_x);
            let mut new_y = drag.win_start_geo.y + (root_y - drag.start_y);

            // While hovering over the bar, keep the window just below it.
            if on_bar {
                let wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
                let mon = wm_ctx.core().globals().selected_monitor();
                new_y = mon.bar_y + mon.bar_height;
            }

            {
                let wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
                crate::mouse::drag::snap_window_to_monitor_edges(
                    &wm_ctx,
                    drag.win,
                    drag.win_start_geo.w.max(1),
                    drag.win_start_geo.h.max(1),
                    &mut new_x,
                    &mut new_y,
                );
            }
            crate::contexts::WmCtx::Wayland(ctx.reborrow()).move_resize(
                drag.win,
                Rect {
                    x: new_x,
                    y: new_y,
                    w: drag.win_start_geo.w.max(1),
                    h: drag.win_start_geo.h.max(1),
                },
                MoveResizeOptions::hinted_immediate(true),
            );
            if let Some(client) = ctx.core.globals_mut().clients.get_mut(&drag.win) {
                client.float_geo.x = new_x;
                client.float_geo.y = new_y;
            }
            true
        }
        crate::globals::DragType::Resize(dir) => {
            let orig_left = drag.win_start_geo.x;
            let orig_top = drag.win_start_geo.y;
            let orig_right = drag.win_start_geo.x + drag.win_start_geo.w;
            let orig_bottom = drag.win_start_geo.y + drag.win_start_geo.h;
            let (affects_left, affects_right, affects_top, affects_bottom) = dir.affected_edges();
            let (new_x, new_w) = if affects_left {
                (root_x, (orig_right - root_x).max(1))
            } else if affects_right {
                (orig_left, (root_x - orig_left + 1).max(1))
            } else {
                (orig_left, drag.win_start_geo.w.max(1))
            };
            let (new_y, new_h) = if affects_top {
                (root_y, (orig_bottom - root_y).max(1))
            } else if !affects_top && affects_bottom {
                (orig_top, (root_y - orig_top + 1).max(1))
            } else {
                (orig_top, drag.win_start_geo.h.max(1))
            };
            crate::contexts::WmCtx::Wayland(ctx.reborrow()).move_resize(
                drag.win,
                Rect {
                    x: new_x,
                    y: new_y,
                    w: new_w,
                    h: new_h,
                },
                MoveResizeOptions::hinted_immediate(true),
            );
            true
        }
    }
}

/// Finish an active drag interaction (move or resize) on Wayland.
///
/// Handles **all** `dragging == true` finishes regardless of how the drag
/// was initiated.  Returns `false` for click-without-drag interactions so
/// `title_drag_finish` can handle the click action.
pub fn wayland_hover_resize_drag_finish(ctx: &mut WmCtxWayland<'_>, btn: MouseButton) -> bool {
    if !ctx.core.globals().drag.interactive.active
        || !ctx.core.globals().drag.interactive.dragging
        || ctx.core.globals().drag.interactive.button != btn
    {
        return false;
    }
    let drag = ctx.core.globals().drag.interactive.clone();
    ctx.core.globals_mut().drag.interactive = crate::globals::DragInteraction::default();
    if matches!(drag.drag_type, crate::globals::DragType::Resize(_)) {
        let _ = ctx.wayland.backend.with_state(|state| {
            state.end_interactive_resize(drag.win);
        });
    }
    set_cursor_style(
        &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
        AltCursor::Default,
    );
    match drag.drag_type {
        crate::globals::DragType::Move => {
            crate::mouse::drag::complete_move_drop(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                drag.win,
                drag.drop_restore_geo,
                None,
                Some((drag.last_root_x, drag.last_root_y)),
            );
            crate::mouse::drag::clear_bar_hover(&mut crate::contexts::WmCtx::Wayland(
                ctx.reborrow(),
            ));
        }
        crate::globals::DragType::Resize(_) => {
            crate::mouse::monitor::handle_client_monitor_switch(
                &mut crate::contexts::WmCtx::Wayland(ctx.reborrow()),
                drag.win,
            );
        }
    }
    crate::contexts::WmCtx::Wayland(ctx.reborrow()).raise_client(drag.win);
    true
}
