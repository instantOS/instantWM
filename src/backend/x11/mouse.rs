//! X11 mouse backend helpers.

use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::backend::{BackendEvent, PointerOps};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::floating::toggle_floating;
use crate::geometry::MoveResizeOptions;
use crate::mouse::drag::lifecycle::{ResizeDragParams, begin_resize};
use crate::mouse::drag::{MoveState, on_motion, prepare_drag_target};
use crate::mouse::resize::compute_axis_resize;
use crate::types::{AltCursor, MouseButton, Point, Rect, ResizeDirection, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CursorUpdateTargets {
    root: bool,
    active_grab: bool,
}

fn cursor_update_targets(
    last_root_cursor: Option<AltCursor>,
    active_grab: Option<crate::backend::x11::ActivePointerGrab>,
    requested: AltCursor,
) -> CursorUpdateTargets {
    CursorUpdateTargets {
        root: last_root_cursor != Some(requested),
        active_grab: active_grab.is_some_and(|grab| grab.cursor != requested),
    }
}

/// Project the requested cursor to the root window and any active pointer grab.
///
/// Active grabs own their cursor independently of the root window, so both
/// projections must remain synchronized with the shared requested style.
pub fn set_x11_cursor(
    x11: &X11BackendRef<'_>,
    x11_runtime: &mut X11RuntimeConfig,
    cursor: AltCursor,
) {
    let targets = cursor_update_targets(
        x11_runtime.last_x11_cursor,
        x11_runtime.active_pointer_grab,
        cursor,
    );
    if !targets.root && !targets.active_grab {
        return;
    }
    let conn = x11.conn;
    let root = x11_runtime.root;
    let cursor_index = cursor.to_x11_index();
    if let Some(Some(loaded_cursor)) = x11_runtime.cursors.get(cursor_index) {
        if targets.root {
            let _ = xproto::change_window_attributes(
                conn,
                root,
                &xproto::ChangeWindowAttributesAux::new().cursor(loaded_cursor.cursor as u32),
            );
            x11_runtime.last_x11_cursor = Some(cursor);
        }
        if let Some(grab) = x11_runtime.active_pointer_grab.as_mut()
            && targets.active_grab
        {
            let _ = conn.change_active_pointer_grab(
                loaded_cursor.cursor as u32,
                x11rb::CURRENT_TIME,
                grab.event_mask,
            );
            grab.cursor = cursor;
        }
        let _ = conn.flush();
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::{CursorUpdateTargets, cursor_update_targets};
    use crate::backend::x11::ActivePointerGrab;
    use crate::types::AltCursor;
    use x11rb::protocol::xproto::EventMask;

    fn active(cursor: AltCursor) -> ActivePointerGrab {
        ActivePointerGrab {
            event_mask: EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
            cursor,
        }
    }

    #[test]
    fn active_grab_cursor_updates_even_when_root_already_matches() {
        assert_eq!(
            cursor_update_targets(
                Some(AltCursor::Move),
                Some(active(AltCursor::Default)),
                AltCursor::Move,
            ),
            CursorUpdateTargets {
                root: false,
                active_grab: true,
            }
        );
    }

    #[test]
    fn matching_root_and_grab_suppress_redundant_native_updates() {
        assert_eq!(
            cursor_update_targets(
                Some(AltCursor::Move),
                Some(active(AltCursor::Move)),
                AltCursor::Move,
            ),
            CursorUpdateTargets {
                root: false,
                active_grab: false,
            }
        );
    }
}

impl crate::backend::CursorOps for WmCtxX11<'_> {
    fn apply_cursor_style(&mut self, style: AltCursor) {
        set_x11_cursor(&self.x11, self.x11_runtime, style);
    }
}

/// X11-only synchronous window move implementation.
///
/// Grab → event loop → release handling. This is deliberately reachable only
/// from pointer-driven actions; keyboard tree placement is a separate modal
/// interaction shared by both backends.
pub fn move_mouse(ctx: &mut WmCtxX11, btn: MouseButton, float_restore_geo: Option<Rect>) {
    let Some(win) = ({
        let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
        prepare_drag_target(&mut wm_ctx)
    }) else {
        return;
    };

    let wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    let Some(start) = wm_ctx.pointer_backend().pointer_location() else {
        return;
    };

    let Some(grab_start_rect) = ctx.core.client_geo(win) else {
        return;
    };

    let mut state = MoveState {
        start_point: start,
        grab_start_rect,
        drop_restore_rect: float_restore_geo.unwrap_or(grab_start_rect),
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };

    if ctx
        .core
        .drag_state_mut()
        .begin_move(win, btn, start, grab_start_rect)
        .is_err()
    {
        return;
    }

    let release_modifiers = crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::Move,
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, .. } = event {
                let root = *root;
                ctx.core.drag_state_mut().record_interactive_motion(root);
                let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
                on_motion(&mut wm_ctx, win, root, root, &mut state);
            }
            true
        },
    )
    .unwrap_or(0);

    crate::mouse::drag::lifecycle::finish(ctx.core.drag_state_mut(), &ctx.x11, btn)
        .expect("X11 drag loop must finish the interaction using its grab button");
    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    crate::mouse::drag::finish_drag_move(
        &mut wm_ctx,
        win,
        state.drop_restore_rect,
        state.edge_snap_indicator,
        None,
        release_modifiers,
    );
}

fn with_wm_ctx_x11<T>(ctx_x11: &mut WmCtxX11<'_>, f: impl FnOnce(&mut WmCtx<'_>) -> T) -> T {
    let mut ctx = WmCtx::X11(ctx_x11.reborrow());
    f(&mut ctx)
}

pub(crate) fn resize_tree_mouse_x11(
    ctx: &mut WmCtxX11<'_>,
    win: WindowId,
    btn: MouseButton,
    start: Point,
    geo: Rect,
    resize: crate::layouts::manager::PointerTreeResizeStart,
) {
    if ctx
        .core
        .drag_state_mut()
        .begin_tree_resize(
            win,
            btn,
            resize.direction,
            start,
            geo,
            resize.origin.clone(),
        )
        .is_err()
    {
        return;
    }
    crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::Resize(resize.direction),
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, .. } = event {
                ctx.core.drag_state_mut().record_interactive_motion(*root);
                let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                let _ = crate::layouts::manager::update_pointer_tree_resize(
                    &mut wm_ctx,
                    win,
                    &resize.origin,
                    resize.direction,
                    start,
                    *root,
                );
            }
            true
        },
    );
    crate::mouse::drag::lifecycle::finish(ctx.core.drag_state_mut(), &ctx.x11, btn)
        .expect("X11 tree resize must finish using its grab button");
    crate::mouse::drag::finish_drag_resize(&mut WmCtx::X11(ctx.reborrow()), win);
}

/// Directional resize: supports all 8 directions (corners and edges).
///
/// When `direction` is `None`, behaves like a bottom-right corner resize.
/// Otherwise, resizes from the specified edge or corner.
pub fn resize_mouse_directional(
    ctx: &mut WmCtxX11,
    direction: Option<ResizeDirection>,
    btn: MouseButton,
) {
    let Some(win) = ctx.core.model().selected_win() else {
        return;
    };
    let (is_blocked, orig_left, orig_top, orig_right, orig_bottom, border_width) =
        match ctx.core.model().client(win) {
            Some(c) => (
                c.mode().is_true_fullscreen(),
                c.geo.x,
                c.geo.y,
                c.geo.right(),
                c.geo.bottom(),
                c.border_width,
            ),
            None => return,
        };
    if is_blocked {
        return;
    }

    let dir = direction.unwrap_or(ResizeDirection::BottomRight);
    let (affects_left, affects_right, affects_top, affects_bottom) = dir.affected_edges();

    with_wm_ctx_x11(ctx, |ctx| {
        ctx.raise_client(win);
        let selmon_id = ctx.core().model().selected_monitor_id();
        crate::layouts::sync_monitor_z_order(ctx, selmon_id);
    });

    let Some(start) = ctx.x11.pointer_location() else {
        return;
    };
    let Some(geo) = ctx.core.client_geo(win) else {
        return;
    };
    if begin_resize(
        ctx.core.drag_state_mut(),
        &ctx.x11,
        ResizeDragParams {
            win,
            button: btn,
            direction: dir,
            start,
            geometry: geo,
        },
    )
    .is_err()
    {
        return;
    }

    crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::Resize(dir),
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, .. } = event {
                let pointer_x = root.x;
                let pointer_y = root.y;

                let (new_x, new_w) = compute_axis_resize(
                    pointer_x,
                    orig_left,
                    orig_right,
                    border_width,
                    affects_left,
                    affects_right,
                );

                let (new_y, new_h) = compute_axis_resize(
                    pointer_y,
                    orig_top,
                    orig_bottom,
                    border_width,
                    affects_top,
                    affects_bottom,
                );

                let snap = ctx.core.config().window.snap_threshold;

                let should_toggle = if let Some(client) = ctx.core.model().client(win) {
                    let has_tiling = ctx
                        .core
                        .model()
                        .expect_selected_monitor()
                        .is_tiling_layout();

                    !client.mode().is_normal_floating()
                        && has_tiling
                        && ((new_w - client.geo.w).abs() > snap
                            || (new_h - client.geo.h).abs() > snap)
                } else {
                    false
                };

                if should_toggle {
                    with_wm_ctx_x11(ctx, toggle_floating);
                } else {
                    let is_floating = match ctx.core.model().client(win) {
                        Some(c) => c.mode().is_normal_floating(),
                        None => return false,
                    };
                    let has_tiling = ctx
                        .core
                        .model()
                        .expect_selected_monitor()
                        .is_tiling_layout();

                    if !has_tiling || is_floating {
                        with_wm_ctx_x11(ctx, |ctx| {
                            ctx.move_resize(
                                win,
                                Rect {
                                    x: new_x,
                                    y: new_y,
                                    w: new_w,
                                    h: new_h,
                                },
                                MoveResizeOptions::hinted_immediate(true),
                            );
                        });
                    }
                }
            }
            true
        },
    );

    crate::mouse::drag::lifecycle::finish(ctx.core.drag_state_mut(), &ctx.x11, btn)
        .expect("X11 drag loop must finish the interaction using its grab button");
    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    crate::mouse::drag::finish_drag_resize(&mut wm_ctx, win);
}

/// Interactive resize that respects the window's declared aspect-ratio hints.
///
/// Reads `client.min_aspect`, `client.max_aspect`, and `client.size_hints` to clamp the
/// new dimensions so the window's aspect ratio stays within the range it
/// advertised via `WM_NORMAL_HINTS`.
pub fn resize_aspect_mouse_x11(ctx: &mut WmCtxX11, win: WindowId, btn: MouseButton) {
    let (is_fullscreen, orig_geo) = match ctx.core.model().client(win) {
        Some(c) => (c.mode().is_fullscreen(), c.geo),
        None => return,
    };
    if is_fullscreen {
        return;
    }

    {
        let mut tmp = WmCtx::X11(ctx.reborrow());
        let selmon_id = tmp.core().model().selected_monitor_id();
        crate::layouts::sync_monitor_z_order(&mut tmp, selmon_id);
    }

    let Some(start) = ctx.x11.pointer_location() else {
        return;
    };
    let Some(geo) = ctx.core.client_geo(win) else {
        return;
    };
    if begin_resize(
        ctx.core.drag_state_mut(),
        &ctx.x11,
        ResizeDragParams {
            win,
            button: btn,
            direction: ResizeDirection::BottomRight,
            start,
            geometry: geo,
        },
    )
    .is_err()
    {
        return;
    }

    crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        btn,
        AltCursor::Resize(ResizeDirection::BottomRight),
        false,
        |ctx, event| {
            if let BackendEvent::Motion { root, .. } = event {
                let (_, raw_nw) =
                    compute_axis_resize(root.x, orig_geo.x, orig_geo.right(), 0, false, true);
                let (_, raw_nh) =
                    compute_axis_resize(root.y, orig_geo.y, orig_geo.bottom(), 0, false, true);

                if let Some((client_geo, sh, min_aspect, max_aspect)) = ctx
                    .core
                    .state()
                    .model
                    .client(win)
                    .map(|c| (c.geo, c.size_hints, c.min_aspect, c.max_aspect))
                {
                    let mut nw = raw_nw;
                    let mut nh = raw_nh;

                    if sh.min_width > 0 {
                        nw = nw.max(sh.min_width);
                    }
                    if sh.min_height > 0 {
                        nh = nh.max(sh.min_height);
                    }
                    if sh.max_width > 0 {
                        nw = nw.min(sh.max_width);
                    }
                    if sh.max_height > 0 {
                        nh = nh.min(sh.max_height);
                    }

                    if min_aspect > 0.0 && max_aspect > 0.0 {
                        if max_aspect < nw as f32 / nh as f32 {
                            nw = (nh as f32 * max_aspect) as i32;
                        } else if min_aspect < nh as f32 / nw as f32 {
                            nh = (nw as f32 * min_aspect) as i32;
                        }
                    }

                    with_wm_ctx_x11(ctx, |ctx| {
                        ctx.move_resize(
                            win,
                            Rect {
                                x: client_geo.x,
                                y: client_geo.y,
                                w: nw,
                                h: nh,
                            },
                            MoveResizeOptions::hinted_immediate(true),
                        );
                    });
                }
            }
            true
        },
    );

    crate::mouse::drag::lifecycle::finish(ctx.core.drag_state_mut(), &ctx.x11, btn)
        .expect("X11 drag loop must finish the interaction using its grab button");
    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    crate::mouse::drag::finish_drag_resize(&mut wm_ctx, win);
}

pub fn cursor_client_win(
    globals: &crate::core_state::CoreState,
    conn: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
) -> Option<WindowId> {
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;

    if reply.child == x11rb::NONE {
        return None;
    }

    let win = WindowId::from(reply.child);
    if globals.model.client(win).is_some() {
        Some(win)
    } else {
        None
    }
}
