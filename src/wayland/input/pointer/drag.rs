//! Pointer drag handling (title drag, tag drag, resize drag).

use crate::contexts::WmCtxWayland;
use crate::geometry::MoveResizeOptions;
use crate::mouse::constants::RESIZE_BORDER_ZONE;
use crate::mouse::drag::lifecycle::{ResizeDragParams, begin_resize, finish};
use crate::mouse::hover::selected_hover_resize_target_at;
use crate::mouse::set_cursor_style;
use crate::types::{AltCursor, MouseButton, Point, Rect, WindowId};
use crate::wm::Wm;

/// Get the active drag window (if any).
pub fn active_drag_window(wm: &Wm) -> Option<WindowId> {
    wm.core.drag.active_interaction().map(|drag| drag.win())
}

/// Begin hover resize/move/close action based on button pressed in border zone.
pub fn hover_resize_drag_begin(
    ctx: &mut WmCtxWayland<'_>,
    position: Point,
    btn: MouseButton,
) -> bool {
    let Some(target) = selected_hover_resize_target_at(ctx.core.model(), position) else {
        return false;
    };

    if btn == MouseButton::Middle {
        let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
        crate::client::kill::close_win(&mut wm_ctx, target.win);
        return true;
    }

    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }
    let win = target.win;
    let geo = target.geo;
    let drag_type =
        if btn == MouseButton::Right || geo.is_at_top_middle_edge(position, RESIZE_BORDER_ZONE) {
            crate::core_state::DragType::Move
        } else {
            crate::core_state::DragType::Resize(target.dir)
        };
    let started = match drag_type {
        crate::core_state::DragType::Move => ctx
            .core
            .drag_state_mut()
            .begin_move(win, btn, position, geo),
        crate::core_state::DragType::Resize(dir) => begin_resize(
            ctx.core.drag_state_mut(),
            ctx.wayland,
            ResizeDragParams {
                win,
                button: btn,
                direction: dir,
                start: position,
                geometry: geo,
            },
        ),
        crate::core_state::DragType::TreeResize(_) => return false,
    };
    if started.is_err() {
        return false;
    }
    let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
    match drag_type {
        crate::core_state::DragType::Move => set_cursor_style(&mut wm_ctx, AltCursor::Move),
        crate::core_state::DragType::Resize(dir) => {
            set_cursor_style(&mut wm_ctx, AltCursor::Resize(dir));
        }
        crate::core_state::DragType::TreeResize(_) => unreachable!("handled before drag start"),
    }
    crate::focus::focus(&mut wm_ctx, Some(win));
    wm_ctx.raise_client(win);
    true
}

/// Handle interactive drag motion (move or resize) on Wayland.
///
/// This is the single motion handler for all drags in the `Active` phase,
/// regardless of how the drag was initiated (title bar, hover border,
/// keyboard shortcut, Super+button, etc.).
pub fn hover_resize_drag_motion(ctx: &mut WmCtxWayland<'_>, root: Point) -> bool {
    let Some(drag) = ctx.core.drag_state().active_interaction().cloned() else {
        return false;
    };
    ctx.core.drag_state_mut().record_interactive_motion(root);

    match drag.operation() {
        crate::core_state::DragOperationRef::Move => {
            let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
            let on_bar = crate::mouse::drag::update_bar_hover_simple(&mut wm_ctx, root);

            if crate::layouts::manager::uses_manual_tree_pointer_interaction(&wm_ctx, drag.win()) {
                // Tiled motion selects a semantic drop target; the tree is
                // mutated only on release by the shared completion path.
                let edge =
                    crate::mouse::drag::move_drop::check_edge_snap(wm_ctx.core().model(), root);
                crate::mouse::drag::move_drop::update_tiled_drag_preview(
                    &mut wm_ctx,
                    drag.win(),
                    root,
                    on_bar,
                    edge,
                );
                return true;
            }

            wm_ctx.update_layout_preview(None);

            let mut new_pos = Point::new(
                drag.win_start_geo().x + (root.x - drag.start_point().x),
                drag.win_start_geo().y + (root.y - drag.start_point().y),
            );

            // While hovering over the bar, keep the window just below it.
            if on_bar {
                let mon = wm_ctx.core().model().expect_selected_monitor();
                new_pos.y = mon.bar_y() + mon.bar_height;
            }

            crate::mouse::drag::snap_window_to_monitor_edges(
                wm_ctx.core().state(),
                drag.win(),
                crate::types::Size::new(
                    drag.win_start_geo().w.max(1),
                    drag.win_start_geo().h.max(1),
                ),
                &mut new_pos,
            );
            wm_ctx.move_resize(
                drag.win(),
                Rect {
                    x: new_pos.x,
                    y: new_pos.y,
                    w: drag.win_start_geo().w.max(1),
                    h: drag.win_start_geo().h.max(1),
                },
                MoveResizeOptions::hinted_immediate(true),
            );
            if let Some(client) = wm_ctx.core_mut().model_mut().client_mut(drag.win()) {
                client.float_geo.x = new_pos.x;
                client.float_geo.y = new_pos.y;
            }
            true
        }
        crate::core_state::DragOperationRef::TreeResize { direction, origin } => {
            let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
            crate::layouts::manager::update_pointer_tree_resize(
                &mut wm_ctx,
                drag.win(),
                origin,
                direction,
                drag.start_point(),
                root,
            )
        }
        crate::core_state::DragOperationRef::Resize(dir) => {
            let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
            let (affects_left, affects_right, affects_top, affects_bottom) = dir.affected_edges();
            let (new_x, new_w) = crate::mouse::resize::compute_axis_resize(
                root.x,
                drag.win_start_geo().x,
                drag.win_start_geo().x + drag.win_start_geo().w,
                0,
                affects_left,
                affects_right,
            );
            let (new_y, new_h) = crate::mouse::resize::compute_axis_resize(
                root.y,
                drag.win_start_geo().y,
                drag.win_start_geo().y + drag.win_start_geo().h,
                0,
                affects_top,
                affects_bottom,
            );
            wm_ctx.move_resize(
                drag.win(),
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
/// Handles finishes for all drags in the `Active` phase regardless of how the
/// drag was initiated. Returns `false` for armed click interactions so
/// `title_drag_finish` can handle the click action.
pub fn hover_resize_drag_finish(
    ctx: &mut WmCtxWayland<'_>,
    btn: MouseButton,
    modifiers: u32,
) -> bool {
    let Some(drag) = finish(ctx.core.drag_state_mut(), ctx.wayland, btn) else {
        return false;
    };
    let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
    match drag.drag_type() {
        crate::core_state::DragType::Move => {
            crate::mouse::drag::finish_drag_move(
                &mut wm_ctx,
                drag.win(),
                drag.drop_restore_geo(),
                None,
                Some(drag.last_root_point()),
                modifiers,
            );
        }
        crate::core_state::DragType::Resize(_) => {
            crate::mouse::drag::finish_drag_resize(&mut wm_ctx, drag.win());
        }
        crate::core_state::DragType::TreeResize(_) => {
            crate::mouse::drag::finish_drag_resize(&mut wm_ctx, drag.win());
        }
    }
    true
}
