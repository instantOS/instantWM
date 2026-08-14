//! Window title bar drag operations.
//!
//! This module handles click and drag interactions on window title bars,
//! supporting both left-click (move) and right-click (resize/zoom) actions.

use crate::client::geometry::FloatingPlacementIntent;
use crate::contexts::WmCtx;
use crate::mouse::constants::DRAG_THRESHOLD;
use crate::mouse::drag::lifecycle::activate_armed_resize;
use crate::mouse::drag::move_drop::promote_to_floating;
use crate::mouse::resize::resize_from_point;
use crate::mouse::warp;
use crate::types::geometry::Point;
use crate::types::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragInput {
    Pointer(Point),
    Absolute(Point),
}

impl DragInput {
    fn position(self) -> Point {
        match self {
            Self::Pointer(point) | Self::Absolute(point) => point,
        }
    }

    fn may_warp_pointer(self) -> bool {
        matches!(self, Self::Pointer(_))
    }
}

/// Initialise a title-bar click/drag interaction.
///
/// Returns `true` if the state machine was started.  On X11 the caller
/// continues into the synchronous grab loop; on Wayland the calloop drives
/// [`process_title_drag_motion`] and [`title_drag_finish`].
pub fn title_drag_begin(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    origin: crate::core_state::ArmedDragOrigin,
    source: InteractionSource,
    click_root: Point,
    suppress_click_action: bool,
) -> bool {
    if btn == MouseButton::Right {
        let is_true_fullscreen = match ctx.core().model().client(win) {
            Some(c) => c.mode().is_true_fullscreen(),
            None => return false,
        };
        if is_true_fullscreen {
            return false;
        }
        crate::focus::focus(ctx, Some(win));
    }

    let sel = ctx.core().model().selected_win();
    let (win_start_geo, drop_restore_geo) = match ctx.core().model().client(win) {
        Some(c) => {
            let restore = c.saved_floating_rect().unwrap_or(c.geo);
            (c.geo, restore)
        }
        None => return false,
    };
    let was_hidden = ctx
        .core()
        .model()
        .client(win)
        .is_some_and(|client| client.is_hidden);
    ctx.core_mut()
        .drag_state_mut()
        .arm_title_drag(crate::core_state::ArmedDragParams {
            win,
            button: btn,
            origin,
            source,
            start: click_root,
            geometry: win_start_geo,
            restore_geometry: drop_restore_geo,
            was_focused: sel == Some(win),
            was_hidden,
            suppress_click_action,
        })
        .is_ok()
}

/// Shared move-start policy for title-bar / Super+client left-click drags.
///
/// Promotes a tiled window to floating using [`PreservePointerAnchor`] so the
/// window appears under the cursor instead of jumping to the center of the
/// work area. Already-floating windows are left untouched.
///
/// Containment clamping can still leave the cursor outside the window (e.g.
/// when the cursor is near a screen edge). In that case the cursor is warped
/// to the nearest edge of the window — the cursor jumps rather than the
/// window — and that warped position is returned as the drag anchor so the
/// drag math stays consistent.
///
/// A manual-tree placement gesture skips promotion and keeps the tiled source
/// in its original slot so cancellation is lossless.
fn begin_move_drag(
    ctx: &mut WmCtx,
    win: WindowId,
    input: DragInput,
    start_point: Point,
) -> Option<(Rect, Point)> {
    let client = ctx.core().model().client(win)?;
    if client.is_edge_scratchpad() {
        return None;
    }
    if client.snap_status != SnapPosition::None {
        crate::floating::reset_snap(ctx, win);
    }

    let position = input.position();
    if crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win) {
        let geo = ctx.client_geo(win)?;
        Some((geo, start_point))
    } else {
        let intent = FloatingPlacementIntent::PreservePointerAnchor(position);
        let (geo, _) = promote_to_floating(ctx, win, intent)?;
        if !input.may_warp_pointer() {
            return Some((geo, position));
        }
        let start = warp::clamp_into(position, geo);
        if start != position {
            ctx.pointer_backend().warp_to_point(start);
        }
        Some((geo, start))
    }
}

/// Handle the transition from an armed click to an active shared drag.
fn title_drag_start(ctx: &mut WmCtx, input: DragInput) -> bool {
    let (win, btn, source, start_point, suppress_click_action) = {
        let Some(drag) = ctx.core().drag_state().armed_interaction() else {
            return false;
        };
        (
            drag.win(),
            drag.button(),
            drag.source(),
            drag.start_point(),
            drag.suppress_click_action(),
        )
    };
    let is_right_click = btn == MouseButton::Right;

    if is_right_click {
        if crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win) {
            // Bar-title resizing retains its established bottom-right handle;
            // Super+right-drag uses the pointer's quadrant instead.
            let point = if suppress_click_action {
                input.position()
            } else {
                warp::warp_to_resize_corner(ctx, win, ResizeDirection::BottomRight)
                    .unwrap_or(start_point)
            };
            // Tree resize owns an initial tree snapshot, so replace the armed
            // click with the authoritative resize interaction after the drag
            // threshold has been crossed.
            let _ = ctx.core_mut().drag_state_mut().finish_armed();
            resize_from_point(ctx, win, btn, source, point);
            return true;
        }

        // Right-click: promote to floating, set up resize mode, warp cursor.
        let Some((current_geo, _)) = promote_to_floating(
            ctx,
            win,
            FloatingPlacementIntent::PreservePointerAnchor(start_point),
        ) else {
            return false;
        };

        let dir = if suppress_click_action {
            ResizeDirection::from_hit(current_geo.size(), current_geo.local_point(start_point))
        } else {
            ResizeDirection::BottomRight
        };

        let Some(warp_point) = warp::warp_to_resize_corner(ctx, win, dir) else {
            return true;
        };

        let activated = match ctx {
            WmCtx::X11(x11) => activate_armed_resize(
                x11.core.drag_state_mut(),
                &x11.x11,
                dir,
                warp_point,
                current_geo,
            ),
            WmCtx::Wayland(wayland) => activate_armed_resize(
                wayland.core.drag_state_mut(),
                wayland.wayland,
                dir,
                warp_point,
                current_geo,
            ),
        };
        if activated.is_err() {
            return false;
        }
        ctx.set_cursor_style(AltCursor::Resize(dir));
        return true;
    }

    // A tiled left-drag is a manual-tree placement gesture. Floating windows
    // continue to move directly. Keeping the tiled source in its original slot
    // also makes cancellation lossless.
    let Some((current_geo, start)) = begin_move_drag(ctx, win, input, start_point) else {
        return false;
    };

    if ctx
        .core_mut()
        .drag_state_mut()
        .activate_armed(crate::core_state::ArmedDragType::Move, start, current_geo)
        .is_err()
    {
        return false;
    }
    ctx.set_cursor_style(AltCursor::Move);
    true
}

/// Process motion during an active title drag.
///
/// Returns `true` if the drag threshold was exceeded and the drag action
/// (reorder/move/resize) was initiated — the caller should consider the
/// interaction consumed. [`DragInput::Absolute`] preserves the contact point as
/// the window anchor and never warps or consults the compositor pointer.
pub fn process_title_drag_motion(ctx: &mut WmCtx, input: DragInput) -> bool {
    let root = input.position();
    let Some(armed) = ctx.core().drag_state().armed_interaction() else {
        return false;
    };

    if root.manhattan_distance(&armed.start_point()) <= DRAG_THRESHOLD {
        ctx.core_mut()
            .drag_state_mut()
            .record_interactive_motion(root);
        return false;
    }

    // Threshold exceeded — start the drag action.
    let drag = armed.clone();
    let win = drag.win();
    let was_hidden = drag.was_hidden();

    if was_hidden {
        crate::client::show_window(ctx, win);
    }
    crate::focus::focus(ctx, Some(win));
    ctx.raise_client(win);

    // A bar-title left drag reorders the title strip while the pointer stays
    // on it; anything else takes the ordinary move/resize path.
    if drag.origin() == crate::core_state::ArmedDragOrigin::BarTitle
        && drag.button() == MouseButton::Left
        && let Some(monitor_id) = ctx
            .core()
            .model()
            .client(win)
            .map(|client| client.monitor_id)
        && title_strip_target(ctx, monitor_id, root).is_some()
        && begin_bar_reorder(ctx, win, monitor_id)
    {
        return true;
    }

    title_drag_start(ctx, input)
}

/// The window whose title cell is under `root` on `monitor_id`'s bar, if any.
///
/// The selected window's close-button and resize-widget zones belong to its
/// title cell, so they resolve to that window like the rest of the cell.
fn title_strip_target(ctx: &WmCtx<'_>, monitor_id: MonitorId, root: Point) -> Option<WindowId> {
    match super::bar_position_on_monitor(ctx, monitor_id, root) {
        Some(
            BarPosition::WinTitle(win)
            | BarPosition::CloseButton(win)
            | BarPosition::ResizeWidget(win),
        ) => Some(win),
        _ => None,
    }
}

/// Promote an armed bar-title press to a live title-strip reorder.
fn begin_bar_reorder(ctx: &mut WmCtx, win: WindowId, monitor_id: MonitorId) -> bool {
    if ctx
        .core_mut()
        .drag_state_mut()
        .begin_title_reorder(crate::core_state::TitleReorderDrag::new(monitor_id))
        .is_err()
    {
        return false;
    }
    crate::mouse::clear_hover_offer(ctx);
    ctx.set_cursor_style(AltCursor::HorizontalAdjust);
    ctx.core_mut()
        .bar
        .hover
        .set(monitor_id, Gesture::WinTitle(win), true);
    ctx.request_bar_update();
    true
}

/// Process motion during a live title-strip reorder.
///
/// While the pointer stays on a title cell of the press monitor's bar, the
/// dragged title swaps with the cell it enters — order changes commit
/// immediately. Leaving the title strip converts the interaction into the
/// ordinary move drag (tiled windows promote to floating and follow the
/// pointer, exactly like a drag started away from the bar). The conversion is
/// one-way: returning to the strip keeps move semantics.
///
/// Returns `true` when a reorder is in progress; the caller should consider
/// the interaction consumed.
pub fn process_title_reorder_motion(ctx: &mut WmCtx, root: Point) -> bool {
    let Some((drag, reorder)) = ctx.core().drag_state().reordering_interaction() else {
        return false;
    };
    let win = drag.win();
    let monitor_id = reorder.monitor_id();
    let source = drag.source();
    let start_point = drag.start_point();
    ctx.core_mut()
        .drag_state_mut()
        .record_interactive_motion(root);

    match title_strip_target(ctx, monitor_id, root) {
        Some(target) => {
            if target != win && crate::layouts::swap_bar_titles(ctx, monitor_id, win, target) {
                // The swap moves the dragged title to a new cell; keep it
                // highlighted there.
                ctx.core_mut()
                    .bar
                    .hover
                    .set(monitor_id, Gesture::WinTitle(win), true);
                ctx.request_bar_update();
            }
            true
        }
        None => convert_reorder_to_move(ctx, win, source, start_point, root),
    }
}

/// Convert a live title-strip reorder into the ordinary move drag.
fn convert_reorder_to_move(
    ctx: &mut WmCtx,
    win: WindowId,
    source: InteractionSource,
    start_point: Point,
    root: Point,
) -> bool {
    let input = match source {
        InteractionSource::Pointer => DragInput::Pointer(root),
        InteractionSource::Touch(_) => DragInput::Absolute(root),
    };
    let Some((current_geo, start)) = begin_move_drag(ctx, win, input, start_point) else {
        // The window cannot start a move drag (e.g. an edge scratchpad); end
        // the interaction instead of leaving the capture dangling.
        let _ = ctx.core_mut().drag_state_mut().finish_reordering();
        ctx.set_cursor_style(AltCursor::Default);
        ctx.core_mut().bar.hover.clear();
        ctx.request_bar_update();
        return false;
    };
    if ctx
        .core_mut()
        .drag_state_mut()
        .activate_reordering_as_move(start, current_geo)
        .is_err()
    {
        return false;
    }
    ctx.set_cursor_style(AltCursor::Move);
    true
}

/// Finish a live title-strip reorder (button release).
///
/// Order changes were already committed by the motion handler. No click action
/// fires: the drag threshold was crossed.
pub fn title_reorder_finish(ctx: &mut WmCtx) {
    let Some((drag, reorder)) = ctx.core().drag_state().reordering_interaction() else {
        return;
    };
    let root = drag.last_root_point();
    let monitor_id = reorder.monitor_id();
    let position = super::bar_position_on_monitor(ctx, monitor_id, root);
    ctx.core_mut().drag_state_mut().finish_reordering();
    ctx.set_cursor_style(AltCursor::Default);

    // Leave the bar in its ordinary hover state. Clearing it unconditionally
    // causes a visible one-frame flash before the next pointer-motion event.
    if let Some(position) = position {
        ctx.core_mut()
            .bar
            .hover
            .set(monitor_id, position.to_gesture(), false);
    } else {
        ctx.core_mut().bar.hover.clear();
    }
    ctx.request_bar_update();
}

/// Finish a title drag interaction (button release without exceeding the
/// drag threshold).  Performs the click action (focus / hide / zoom).
///
/// Once the drag threshold promotes the interaction to `Active`, the unified
/// the shared interaction transport handles the drop instead.
pub fn title_drag_finish(ctx: &mut WmCtx) {
    let Some(drag) = ctx.core_mut().drag_state_mut().finish_armed() else {
        return;
    };
    let win = drag.win();
    let is_right_click = drag.button() == MouseButton::Right;
    let was_focused = drag.was_focused();
    let was_hidden = drag.was_hidden();
    let suppress_click_action = drag.suppress_click_action();
    if suppress_click_action {
        return;
    }

    if is_right_click {
        if was_hidden {
            crate::client::show_window(ctx, win);
            crate::focus::focus(ctx, Some(win));
        }
        crate::client::zoom(ctx);
    } else if was_focused && !was_hidden {
        crate::client::hide_for_user(ctx, win);
    } else {
        if was_hidden {
            crate::client::show_window(ctx, win);
        }
        crate::focus::focus(ctx, Some(win));
        // A bar title is an explicit stacking handle even when ordinary
        // client-area click-to-raise is disabled.
        ctx.raise_client(win);
    }
}

/// Left-click / drag handler for a window title bar entry.
///
/// Click: hidden → show+focus; focused → hide; otherwise → focus.
/// Drag > [`DRAG_THRESHOLD`]: show, focus, and either reorder the title strip
/// while the pointer stays on it (left button) or promote to the shared move
/// state.
/// Right Click: same as above but allows zoom to master and bottom-right resize on drag.
///
/// Input adapters own capture; this function only arms shared interaction state.
pub fn handle_window_title_mouse(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
    click_root: Point,
) {
    let _ = title_drag_begin(
        ctx,
        win,
        btn,
        crate::core_state::ArmedDragOrigin::BarTitle,
        source,
        click_root,
        false,
    );
}

/// Start a client move/resize that remains a click until the pointer crosses
/// [`DRAG_THRESHOLD`]. Used by Super+client drags so X11 and Wayland have
/// identical activation semantics.
pub fn begin_thresholded_client_drag(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    source: InteractionSource,
    click_root: Point,
    suppress_click_action: bool,
) {
    let _ = title_drag_begin(
        ctx,
        win,
        btn,
        crate::core_state::ArmedDragOrigin::Client,
        source,
        click_root,
        suppress_click_action,
    );
}

#[cfg(test)]
mod tests {
    use super::{DragInput, begin_move_drag, process_title_drag_motion, title_drag_begin};
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::layouts::tree::Preset;
    use crate::mouse::constants::DRAG_THRESHOLD;
    use crate::types::{
        Client, ClientMode, InteractionSource, Monitor, MonitorId, MouseButton, Point, Rect,
        SnapPosition, TagMask, WindowId,
    };
    use crate::wm::Wm;

    fn tiled_pair_fixture() -> (Wm, WindowId, Rect) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            show_bar: false,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let windows = [WindowId(21), WindowId(22)];
        for win in windows {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags,
                mode: ClientMode::tiled(),
                ..Client::default()
            });
        }
        let bounds = {
            let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
            monitor.set_selected_tags(tags);
            monitor.clients = windows.to_vec();
            monitor.selected = Some(windows[0]);
            monitor
                .per_tag_state()
                .layout_tree
                .apply_preset(Preset::MasterStack, &windows, 1);
            monitor
                .per_tag()
                .unwrap()
                .layout_tree
                .bounds(monitor.available_rect)
        };
        for (&win, &geo) in &bounds {
            wm.core.model.client_mut(win).unwrap().geo = geo;
        }
        (wm, windows[0], bounds[&windows[0]])
    }

    #[test]
    fn tiled_right_drag_preserves_source_when_arming_tree_resize() {
        let (mut wm, win, geo) = tiled_pair_fixture();
        let press = Point::new(geo.x + geo.w / 2, geo.y + geo.h / 2);
        assert!(title_drag_begin(
            &mut wm.ctx(),
            win,
            MouseButton::Right,
            crate::core_state::ArmedDragOrigin::Client,
            InteractionSource::Pointer,
            press,
            true,
        ));

        assert!(process_title_drag_motion(
            &mut wm.ctx(),
            DragInput::Absolute(Point::new(press.x + 20, press.y))
        ));
        let active = wm.core.drag.active_interaction().unwrap();
        assert_eq!(active.source(), InteractionSource::Pointer);
        assert!(matches!(
            active.drag_type(),
            crate::core_state::DragType::TreeResize(_)
        ));
    }

    /// Two tiled clients on a monitor with a visible bar, in bar-presentation
    /// order `[first, second]`.
    fn bar_title_fixture(
        presentation: crate::layouts::PresentationMode,
    ) -> (Wm, MonitorId, WindowId, WindowId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.tags.num_tags = 9;
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 0, 1200, 800),
            bar_height: 30,
            show_bar: true,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let windows = [WindowId(31), WindowId(32)];
        for win in windows {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                tags,
                mode: ClientMode::tiled(),
                geo: Rect::new(0, 30, 600, 770),
                ..Client::default()
            });
        }
        let monitor = wm.core.model.monitor_mut(monitor_id).unwrap();
        monitor.set_selected_tags(tags);
        monitor.clients = windows.to_vec();
        monitor.selected = Some(windows[0]);
        monitor.per_tag_state().presentation = presentation;
        if presentation == crate::layouts::PresentationMode::Maximized {
            monitor
                .per_tag_state()
                .layout_tree
                .apply_preset(Preset::MasterStack, &windows, 1);
        }
        (wm, monitor_id, windows[0], windows[1])
    }

    /// Root-space center x of `win`'s title cell, scanned through the shared
    /// hit-test so the test cannot drift from the renderer's layout.
    fn title_cell_center(wm: &mut Wm, monitor_id: MonitorId, win: WindowId) -> i32 {
        let mut span: Option<(i32, i32)> = None;
        let ctx = wm.ctx();
        for x in 0..1200 {
            if super::title_strip_target(&ctx, monitor_id, Point::new(x, 10)) == Some(win) {
                match span {
                    Some((start, _)) => span = Some((start, x)),
                    None => span = Some((x, x)),
                }
            }
        }
        let (start, end) = span.expect("window must own a title cell");
        (start + end + 1) / 2
    }

    #[test]
    fn bar_title_drag_within_strip_reorders_and_leaving_converts_to_move() {
        let (mut wm, monitor_id, first, second) =
            bar_title_fixture(crate::layouts::PresentationMode::Tiled);
        let first_x = title_cell_center(&mut wm, monitor_id, first);
        let second_x = title_cell_center(&mut wm, monitor_id, second);
        assert!(second_x - first_x > DRAG_THRESHOLD * 2);

        let press = Point::new(first_x, 10);
        assert!(title_drag_begin(
            &mut wm.ctx(),
            first,
            MouseButton::Left,
            crate::core_state::ArmedDragOrigin::BarTitle,
            InteractionSource::Pointer,
            press,
            false,
        ));

        // Crossing the threshold while still on the title strip starts a
        // reorder, not a move.
        assert!(process_title_drag_motion(
            &mut wm.ctx(),
            DragInput::Absolute(Point::new(first_x + DRAG_THRESHOLD + 1, 10))
        ));
        assert!(
            wm.core.drag.reordering_interaction().is_some(),
            "threshold crossing inside the strip must engage a reorder"
        );

        // Dragging onto the neighbour's cell swaps the bar order.
        assert!(super::process_title_reorder_motion(
            &mut wm.ctx(),
            Point::new(second_x, 10)
        ));
        let monitor = wm.core.model.monitor(monitor_id).unwrap();
        assert_eq!(
            monitor.bar_client_order(&wm.core.model.clients),
            vec![second, first]
        );

        // Leaving the strip converts the reorder into an ordinary move drag.
        assert!(super::process_title_reorder_motion(
            &mut wm.ctx(),
            Point::new(second_x, 200)
        ));
        assert!(wm.core.drag.reordering_interaction().is_none());
        let active = wm.core.drag.active_interaction().unwrap();
        assert_eq!(active.win(), first);
        assert_eq!(active.drag_type(), crate::core_state::DragType::Move);
    }

    #[test]
    fn maximized_bar_title_drag_swaps_tree_order() {
        let (mut wm, monitor_id, first, second) =
            bar_title_fixture(crate::layouts::PresentationMode::Maximized);
        let first_x = title_cell_center(&mut wm, monitor_id, first);
        let second_x = title_cell_center(&mut wm, monitor_id, second);

        assert!(title_drag_begin(
            &mut wm.ctx(),
            first,
            MouseButton::Left,
            crate::core_state::ArmedDragOrigin::BarTitle,
            InteractionSource::Pointer,
            Point::new(first_x, 10),
            false,
        ));
        assert!(process_title_drag_motion(
            &mut wm.ctx(),
            DragInput::Absolute(Point::new(first_x + DRAG_THRESHOLD + 1, 10))
        ));
        assert!(super::process_title_reorder_motion(
            &mut wm.ctx(),
            Point::new(second_x, 10)
        ));

        let monitor = wm.core.model.monitor(monitor_id).unwrap();
        assert_eq!(
            monitor.tiled_tree_order(&wm.core.model.clients),
            vec![second, first],
            "maximized titles are stack tabs and must swap tree leaves"
        );
    }

    #[test]
    fn client_origin_drag_never_engages_a_reorder() {
        let (mut wm, monitor_id, first, _second) =
            bar_title_fixture(crate::layouts::PresentationMode::Tiled);
        let first_x = title_cell_center(&mut wm, monitor_id, first);

        assert!(title_drag_begin(
            &mut wm.ctx(),
            first,
            MouseButton::Left,
            crate::core_state::ArmedDragOrigin::Client,
            InteractionSource::Pointer,
            Point::new(first_x, 10),
            true,
        ));
        assert!(process_title_drag_motion(
            &mut wm.ctx(),
            DragInput::Absolute(Point::new(first_x + DRAG_THRESHOLD + 1, 10))
        ));
        assert!(
            wm.core.drag.reordering_interaction().is_none(),
            "a Super+client drag must keep taking the move path"
        );
        assert!(wm.core.drag.active_interaction().is_some());
    }

    #[test]
    fn snapped_move_restores_free_geometry_before_starting() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let work = Rect::new(0, 30, 1200, 770);
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: work,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let win = WindowId(23);
        let saved = Rect::new(250, 180, 600, 420);
        let mut client = Client {
            win,
            monitor_id,
            tags,
            mode: ClientMode::floating(),
            geo: Rect::new(0, 30, 600, 770),
            snap_status: SnapPosition::Left,
            ..Client::default()
        };
        client.save_floating_placement(saved, work);
        wm.core.model.insert_client(client);

        let result = begin_move_drag(
            &mut wm.ctx(),
            win,
            DragInput::Absolute(Point::new(300, 220)),
            Point::new(300, 220),
        );

        assert_eq!(result, Some((saved, Point::new(300, 220))));
        assert_eq!(
            wm.core.model.client(win).unwrap().snap_status,
            SnapPosition::None
        );
    }

    #[test]
    fn edge_scratchpad_cannot_start_a_move_drag() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1200, 800),
            available_rect: Rect::new(0, 30, 1200, 770),
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let win = WindowId(24);
        let mut client = Client {
            win,
            monitor_id,
            mode: ClientMode::floating(),
            geo: Rect::new(0, 30, 400, 770),
            ..Client::default()
        };
        client
            .promote_to_scratchpad("edge", Some(crate::types::EdgeDirection::Left), 1200, 800)
            .unwrap();
        wm.core.model.insert_client(client);

        assert_eq!(
            begin_move_drag(
                &mut wm.ctx(),
                win,
                DragInput::Absolute(Point::new(100, 200)),
                Point::new(100, 200),
            ),
            None
        );
    }
}
