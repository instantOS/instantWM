//! Pointer drag handling (title drag, tag drag, resize drag).

use crate::types::WindowId;
use crate::wm::Wm;

/// Get the active drag window (if any).
pub fn active_drag_window(wm: &Wm) -> Option<WindowId> {
    wm.core.drag.active_interaction().map(|drag| drag.win())
}

#[cfg(test)]
mod tests {
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
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
                crate::types::InteractionSource::Pointer,
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
        assert!(crate::mouse::drag::active_drag_motion(&mut wm.ctx(), point));
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
