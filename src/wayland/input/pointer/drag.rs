//! Pointer drag handling (title drag, tag drag, resize drag).

use crate::contexts::WmCtxWayland;
use crate::geometry::MoveResizeOptions;
use crate::mouse::constants::RESIZE_BORDER_ZONE;
use crate::mouse::drag::lifecycle::{ResizeDragParams, begin_resize, finish};
use crate::mouse::hover::selected_hover_resize_target_at;
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
        crate::core_state::DragType::Move => wm_ctx.set_cursor_style(AltCursor::Move),
        crate::core_state::DragType::Resize(dir) => {
            wm_ctx.set_cursor_style(AltCursor::Resize(dir));
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
                drag.win_start_geo().right(),
                0,
                affects_left,
                affects_right,
            );
            let (new_y, new_h) = crate::mouse::resize::compute_axis_resize(
                root.y,
                drag.win_start_geo().y,
                drag.win_start_geo().bottom(),
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
        crate::core_state::DragType::Resize(_) | crate::core_state::DragType::TreeResize(_) => {
            crate::mouse::drag::finish_drag_resize(&mut wm_ctx, drag.win());
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::hover_resize_drag_motion;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::contexts::WmCtx;
    use crate::layouts::tree::Preset;
    use crate::types::{Client, ClientMode, Monitor, MouseButton, Point, Rect, TagMask, WindowId};
    use crate::wm::Wm;

    const SAMPLE_COUNT: usize = 8_192;
    const HIGH_RATE_BATCH_SIZE: usize = 48;

    fn twenty_window_drag_fixture() -> (Wm, WindowId) {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            available_rect: Rect::new(0, 0, 1920, 1080),
            show_bar: false,
            ..Monitor::default()
        });
        wm.core.model.monitors.set_selected(monitor_id);
        let windows = (1..=20).map(WindowId).collect::<Vec<_>>();
        for &win in &windows {
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
            monitor.clients = windows.clone();
            monitor.selected = Some(windows[0]);
            monitor
                .per_tag_state()
                .layout_tree
                .apply_preset(Preset::Grid, &windows, 1);
            monitor
                .per_tag()
                .unwrap()
                .layout_tree
                .bounds(monitor.available_rect)
        };
        for (&win, &geo) in &bounds {
            wm.core.model.client_mut(win).unwrap().geo = geo;
        }

        let source = windows[0];
        let source_geo = bounds[&source];
        wm.core
            .drag
            .begin_move(
                source,
                MouseButton::Left,
                Point::new(
                    source_geo.x + source_geo.w / 2,
                    source_geo.y + source_geo.h / 2,
                ),
                source_geo,
            )
            .unwrap();
        (wm, source)
    }

    fn sample_point(index: usize) -> Point {
        // Sweep repeatedly across the centre and edge trigger bands of the
        // lower grid rows, never crossing the dragged source's own cell.
        Point::new(
            400 + i32::try_from(index.wrapping_mul(17) % 1_400).unwrap(),
            330 + i32::try_from(index.wrapping_mul(11) % 650).unwrap(),
        )
    }

    fn process_drag_sample(wm: &mut Wm, point: Point) {
        let WmCtx::Wayland(mut ctx) = wm.ctx() else {
            unreachable!("fixture always uses the Wayland backend");
        };
        assert!(hover_resize_drag_motion(&mut ctx, point));
    }

    fn run_samples(wm: &mut Wm, clear_cache_each_sample: bool, batch_size: usize) -> usize {
        let mut updates = 0;
        for batch_start in (0..SAMPLE_COUNT).step_by(batch_size) {
            let last = (batch_start + batch_size).min(SAMPLE_COUNT) - 1;
            if clear_cache_each_sample {
                for index in batch_start..=last {
                    wm.core.pointer_placement_cache = None;
                    process_drag_sample(wm, sample_point(index));
                    updates += 1;
                }
            } else {
                process_drag_sample(wm, sample_point(last));
                updates += 1;
            }
        }
        updates
    }

    #[test]
    fn high_rate_motion_batch_keeps_the_last_drag_preview() {
        let (mut every_sample, _) = twenty_window_drag_fixture();
        let (mut coalesced, _) = twenty_window_drag_fixture();

        let every_update_count = run_samples(&mut every_sample, false, 1);
        let coalesced_update_count = run_samples(&mut coalesced, false, HIGH_RATE_BATCH_SIZE);

        assert_eq!(
            every_sample.core.layout_preview,
            coalesced.core.layout_preview
        );
        assert_eq!(every_update_count, SAMPLE_COUNT);
        assert_eq!(
            coalesced_update_count,
            SAMPLE_COUNT.div_ceil(HIGH_RATE_BATCH_SIZE)
        );
    }

    #[test]
    #[ignore = "deterministic profiling benchmark; run explicitly with --nocapture"]
    fn benchmark_twenty_window_drag_pipeline() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        fn measure(
            clear_cache_each_sample: bool,
            batch_size: usize,
        ) -> (Duration, usize, Option<Rect>) {
            let (mut wm, _) = twenty_window_drag_fixture();
            if !clear_cache_each_sample {
                // Measure steady-state pointer motion, not one-time cache fill.
                for index in 0..SAMPLE_COUNT {
                    process_drag_sample(&mut wm, sample_point(index));
                }
            }
            let started = Instant::now();
            let updates = black_box(run_samples(&mut wm, clear_cache_each_sample, batch_size));
            (started.elapsed(), updates, wm.core.layout_preview)
        }

        fn median(mut samples: Vec<Duration>) -> Duration {
            samples.sort_unstable();
            samples[samples.len() / 2]
        }

        let mut uncached = Vec::new();
        let mut memoized = Vec::new();
        let mut coalesced = Vec::new();
        for _ in 0..5 {
            let (elapsed, updates, expected_preview) = measure(true, 1);
            assert_eq!(updates, SAMPLE_COUNT);
            uncached.push(elapsed);

            let (elapsed, updates, preview) = measure(false, 1);
            assert_eq!(updates, SAMPLE_COUNT);
            assert_eq!(preview, expected_preview);
            memoized.push(elapsed);

            let (elapsed, updates, preview) = measure(false, HIGH_RATE_BATCH_SIZE);
            assert_eq!(updates, SAMPLE_COUNT.div_ceil(HIGH_RATE_BATCH_SIZE));
            assert_eq!(preview, expected_preview);
            coalesced.push(elapsed);
        }

        let uncached = median(uncached);
        let memoized = median(memoized);
        let coalesced = median(coalesced);
        let nanos_per_sample =
            |duration: Duration| duration.as_nanos() as f64 / SAMPLE_COUNT as f64;
        println!("20-window active drag, {SAMPLE_COUNT} input samples (median of 5):");
        println!(
            "  uncached per event: {:>10.1} ns/input ({uncached:?})",
            nanos_per_sample(uncached)
        );
        println!(
            "  memoized per event: {:>10.1} ns/input ({memoized:?})",
            nanos_per_sample(memoized)
        );
        println!(
            "  memoized + {HIGH_RATE_BATCH_SIZE}:1 coalescing: {:>10.1} ns/input ({coalesced:?})",
            nanos_per_sample(coalesced)
        );
    }
}
