//! Backend-neutral active window move/resize processing.
//!
//! Input transports decide how events are captured. Once captured, pointer and
//! touch samples use these same motion and finish operations on every backend.

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::mouse::constants::RESIZE_BORDER_ZONE;
use crate::mouse::drag::lifecycle::{ResizeDragParams, begin_resize};
use crate::types::{InteractionSource, MouseButton, Point, Rect, WindowId};

fn begin_active_resize(
    ctx: &mut WmCtx<'_>,
    params: ResizeDragParams,
) -> Result<(), crate::core_state::InteractionAlreadyActive> {
    ctx.transition_pointer_interaction(|drag| begin_resize(drag, params))
}

/// Begin a directional resize from a compositor binding on any backend.
pub fn directional_resize_begin(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
    direction: crate::types::ResizeDirection,
    geometry: Rect,
) -> bool {
    directional_resize_begin_with_policy(
        ctx,
        win,
        btn,
        source,
        direction,
        geometry,
        crate::core_state::ResizePolicy::Free,
    )
}

pub fn directional_resize_begin_with_policy(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
    direction: crate::types::ResizeDirection,
    geometry: Rect,
    policy: crate::core_state::ResizePolicy,
) -> bool {
    let Some(start) = crate::mouse::warp::warp_to_resize_corner(ctx, win, direction) else {
        return false;
    };
    if begin_active_resize(
        ctx,
        ResizeDragParams {
            win,
            button: btn,
            source,
            direction,
            start,
            geometry,
            policy,
        },
    )
    .is_err()
    {
        return false;
    }
    crate::focus::focus(ctx, Some(win));
    ctx.raise_client(win);
    true
}

/// Begin a semantic tiled-tree resize from a compositor binding.
pub fn tree_resize_begin(
    ctx: &mut WmCtx<'_>,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
    start: Point,
    geometry: Rect,
    resize: crate::layouts::manager::PointerTreeResizeStart,
) -> bool {
    if ctx
        .transition_pointer_interaction(|drag| {
            drag.begin_tree_resize(crate::core_state::TreeResizeParams {
                win,
                button: btn,
                source,
                direction: resize.direction,
                start,
                geometry,
                origin: resize.origin,
            })
        })
        .is_err()
    {
        return false;
    }
    crate::focus::focus(ctx, Some(win));
    ctx.raise_client(win);
    true
}

/// Commit the currently selected floating-border hover target.
pub fn hover_drag_begin(
    ctx: &mut WmCtx<'_>,
    position: Point,
    btn: MouseButton,
    source: InteractionSource,
) -> bool {
    let Some(target) =
        crate::mouse::hover::selected_hover_resize_target_at(ctx.core().model(), position)
    else {
        return false;
    };

    if btn == MouseButton::Middle {
        crate::client::kill::close_win(ctx, target.win);
        return true;
    }
    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }

    let drag_type = if btn == MouseButton::Right
        || target
            .geo
            .is_at_top_middle_edge(position, RESIZE_BORDER_ZONE)
    {
        crate::core_state::DragType::Move
    } else {
        crate::core_state::DragType::Resize(target.dir)
    };
    let started = match drag_type {
        crate::core_state::DragType::Move => ctx.transition_pointer_interaction(|drag| {
            drag.begin_move(target.win, btn, source, position, target.geo)
        }),
        crate::core_state::DragType::Resize(direction) => begin_active_resize(
            ctx,
            ResizeDragParams {
                win: target.win,
                button: btn,
                source,
                direction,
                start: position,
                geometry: target.geo,
                policy: crate::core_state::ResizePolicy::Free,
            },
        ),
        crate::core_state::DragType::TreeResize(_) => unreachable!(),
    };
    if started.is_err() {
        return false;
    }

    debug_assert!(!matches!(
        drag_type,
        crate::core_state::DragType::TreeResize(_)
    ));
    crate::focus::focus(ctx, Some(target.win));
    ctx.raise_client(target.win);
    true
}

/// Apply one absolute motion sample to an engaged window drag.
///
/// Handles the `Active` phase of `WindowDragState`: the press has already
/// crossed the drag threshold, or the drag started immediately from a resize
/// handle, hovered border, or client request. The pre-threshold `Armed` phase
/// is handled by `process_title_drag_motion` instead, which only records samples.
///
/// Returns `false` when no window drag is currently active (the sample was
/// not consumed); `true` when the sample was applied to the ongoing move,
/// resize, or tree-resize.
pub fn apply_active_drag_motion(ctx: &mut WmCtx<'_>, root: Point) -> bool {
    let Some(drag) = ctx.core().interaction().drag.active_interaction().cloned() else {
        return false;
    };
    ctx.transition_pointer_interaction(|drag| drag.record_interactive_motion(root));

    match drag.operation() {
        crate::core_state::DragOperation::Move => {
            apply_move_drag_motion(ctx, &drag, root);
            true
        }
        crate::core_state::DragOperation::TreeResize { direction, origin } => {
            crate::layouts::manager::update_pointer_tree_resize(
                ctx,
                drag.win(),
                origin,
                *direction,
                drag.start_point(),
                root,
            )
        }
        crate::core_state::DragOperation::Resize(dir) => {
            apply_resize_drag_motion(ctx, &drag, *dir, root);
            true
        }
    }
}

fn apply_move_drag_motion(
    ctx: &mut WmCtx<'_>,
    drag: &crate::core_state::DragInteraction,
    root: Point,
) {
    let on_bar = crate::mouse::drag::update_bar_hover_simple(ctx, root);
    let edge = crate::mouse::drag::move_drop::check_edge_snap(ctx.core().model(), root);

    if crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx.core().model(), drag.win())
    {
        crate::mouse::drag::move_drop::update_tiled_drag_preview(
            ctx,
            drag.win(),
            root,
            on_bar,
            edge,
        );
        return;
    }

    ctx.update_layout_preview(None);
    let mut new_pos = Point::new(
        drag.win_start_geo().x + (root.x - drag.start_point().x),
        drag.win_start_geo().y + (root.y - drag.start_point().y),
    );

    if on_bar {
        let mon = ctx.core().model().expect_selected_monitor();
        new_pos.y = mon.bar_y() + mon.bar_height;
    }

    crate::mouse::drag::snap_window_to_monitor_edges(
        ctx.core().state(),
        drag.win(),
        drag.win_start_geo().size(),
        &mut new_pos,
    );
    ctx.move_resize(
        drag.win(),
        Rect::new(
            new_pos.x,
            new_pos.y,
            drag.win_start_geo().w.max(1),
            drag.win_start_geo().h.max(1),
        ),
        MoveResizeOptions::hinted_immediate(true),
    );
}

fn apply_resize_drag_motion(
    ctx: &mut WmCtx<'_>,
    drag: &crate::core_state::DragInteraction,
    direction: crate::types::ResizeDirection,
    root: Point,
) {
    // Core geometry uses an outer origin and content size on both backends.
    // Account for the modelled border so an end edge remains under the input.
    let border_width = ctx
        .core()
        .model()
        .client(drag.win())
        .map_or(0, |client| client.border_width.max(0));
    let (affects_left, affects_right, affects_top, affects_bottom) = direction.affected_edges();
    let (new_x, new_w) = crate::mouse::resize::compute_axis_resize(
        root.x,
        drag.win_start_geo().x,
        drag.win_start_geo().right(),
        border_width,
        affects_left,
        affects_right,
    );
    let (new_y, new_h) = crate::mouse::resize::compute_axis_resize(
        root.y,
        drag.win_start_geo().y,
        drag.win_start_geo().bottom(),
        border_width,
        affects_top,
        affects_bottom,
    );
    let (new_w, new_h) = match drag.resize_policy() {
        crate::core_state::ResizePolicy::Free => (new_w, new_h),
        crate::core_state::ResizePolicy::PreserveAspect => {
            crate::mouse::resize::constrain_aspect_size(ctx, drag.win(), new_w, new_h)
        }
    };
    ctx.move_resize(
        drag.win(),
        Rect::new(new_x, new_y, new_w, new_h),
        MoveResizeOptions::hinted_immediate(true),
    );
}

/// Finish the active window interaction and reconcile its derived projection.
pub fn active_drag_finish(ctx: &mut WmCtx<'_>, btn: MouseButton, modifiers: u32) -> bool {
    let finished =
        ctx.transition_pointer_interaction(|drag| crate::mouse::drag::lifecycle::finish(drag, btn));
    let Some(drag) = finished else {
        return false;
    };

    match drag.drag_type() {
        crate::core_state::DragType::Move => crate::mouse::drag::drag_move_finish(
            ctx,
            drag.win(),
            drag.drop_restore_geo(),
            crate::mouse::drag::move_drop::check_edge_snap(
                ctx.core().model(),
                drag.last_root_point(),
            ),
            Some(drag.last_root_point()),
            modifiers,
        ),
        crate::core_state::DragType::Resize(_) | crate::core_state::DragType::TreeResize(_) => {
            crate::mouse::drag::drag_resize_finish(ctx, drag.win());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::apply_active_drag_motion;
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::types::{
        Client, ClientMode, InteractionSource, Monitor, MouseButton, Point, Rect, ResizeDirection,
        TagMask, WindowId,
    };
    use crate::wm::Wm;

    #[test]
    fn end_edge_resize_accounts_for_the_modelled_border() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let win = WindowId(17);
        let geometry = Rect::new(100, 100, 500, 300);
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::floating(),
            geo: geometry,
            border_width: 5,
            ..Client::default()
        });
        wm.core
            .interaction
            .drag
            .begin_resize(
                win,
                MouseButton::Right,
                InteractionSource::Pointer,
                ResizeDirection::Right,
                Point::new(610, 250),
                geometry,
            )
            .unwrap();

        assert!(apply_active_drag_motion(
            &mut wm.ctx(),
            Point::new(710, 250)
        ));
        assert_eq!(wm.core.model.client(win).unwrap().geo.w, 601);
    }

    #[test]
    fn invalid_tree_resize_motion_reports_not_applied() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let win = WindowId(99);
        wm.core
            .interaction
            .drag
            .begin_tree_resize(crate::core_state::TreeResizeParams {
                win,
                button: MouseButton::Right,
                source: InteractionSource::Pointer,
                direction: ResizeDirection::Right,
                start: Point::new(10, 10),
                geometry: Rect::new(0, 0, 100, 100),
                origin: crate::layouts::tree::LayoutTree::default(),
            })
            .unwrap();

        assert!(!apply_active_drag_motion(&mut wm.ctx(), Point::new(20, 10)));
    }
}
