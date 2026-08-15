use super::offscreen::{
    extra_surface_pixels_are_offscreen, offscreen_shrink_configure_phase,
    resize_growth_is_offscreen_at_start, resize_removal_is_offscreen_at_target,
};
use super::*;

#[test]
fn resize_schedule_emits_exactly_one_configure() {
    let start = Instant::now();
    let mut animation = WaylandWindowAnimation::new(
        Rect::new(0, 0, 100, 80),
        Rect::new(0, 0, 140, 60),
        Size::from((100, 80)),
        None,
        Duration::from_millis(100),
        start,
        2,
        4,
    );

    assert_eq!(
        animation
            .tick(start + Duration::from_millis(19), Size::from((100, 80)))
            .configure_size,
        None
    );
    assert_eq!(
        animation
            .tick(start + Duration::from_millis(21), Size::from((100, 80)))
            .configure_size,
        Some(Size::from((140, 60)))
    );
    assert_eq!(
        animation
            .tick(start + Duration::from_millis(80), Size::from((140, 60)))
            .configure_size,
        None
    );
    assert!(matches!(animation.resize, ResizeConfigure::Sent(_)));
}

#[test]
fn offscreen_shrink_configures_before_movement_is_complete() {
    let start = Instant::now();
    let from = Rect::new(0, 0, 1000, 1000);
    let target = Rect::new(500, 0, 500, 1000);
    let committed = Size::from((1000, 1000));
    let output = Rect::new(0, 0, 1000, 1000);
    let mut animation = WaylandWindowAnimation::new(
        from,
        target,
        committed,
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );

    let phase = offscreen_shrink_configure_phase(
        from,
        target,
        committed,
        0,
        0,
        animation.anchors,
        &[output],
    )
    .expect("the removed half ends outside the output");
    assert!(
        phase > RESIZE_CONFIGURE_PHASE && phase < 1.0,
        "unexpected shrink configure phase: {phase}"
    );
    animation.resize_timing = ResizeTiming::OffscreenShrink;
    animation.resize_configure_phase = phase;
    assert_eq!(
        animation
            .tick(
                start + Duration::from_secs_f64(0.1 * (phase - 0.01)),
                committed,
            )
            .configure_size,
        None
    );
    assert_eq!(
        animation
            .tick(start + Duration::from_secs_f64(0.1 * phase), committed)
            .configure_size,
        Some(Size::from((500, 1000)))
    );
}

#[test]
fn overdue_offscreen_shrink_stages_before_landing() {
    let start = Instant::now();
    let from = Rect::new(0, 0, 1000, 1000);
    let target = Rect::new(500, 0, 500, 1000);
    let committed = Size::from((1000, 1000));
    let output = Rect::new(0, 0, 1000, 1000);
    let mut animation = WaylandWindowAnimation::new(
        from,
        target,
        committed,
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );
    animation.resize_timing = ResizeTiming::OffscreenShrink;
    animation.resize_configure_phase = offscreen_shrink_configure_phase(
        from,
        target,
        committed,
        0,
        0,
        animation.anchors,
        &[output],
    )
    .unwrap();

    let staged = animation.tick(start + Duration::from_millis(200), committed);
    assert_eq!(staged.configure_size, Some(Size::from((500, 1000))));
    assert!(!staged.done);
    assert_ne!(staged.frame, target);

    let landed = animation.tick(start + Duration::from_millis(200), committed);
    assert_eq!(landed.configure_size, None);
    assert!(landed.done);
    assert_eq!(landed.frame, target);
}

#[test]
fn offscreen_shrink_falls_back_when_integer_rounding_leaves_no_motion() {
    let from = Rect::new(0, 0, 2, 2);
    let target = Rect::new(1, 0, 1, 2);

    assert_eq!(
        offscreen_shrink_configure_phase(
            from,
            target,
            Size::from((2, 2)),
            0,
            0,
            SurfaceAnchors::between(from, target, 0, 0),
            &[Rect::new(0, 0, 2, 2)],
        ),
        None
    );
}

#[test]
fn shrink_stays_mid_animation_when_removed_pixels_touch_an_output() {
    let target = Rect::new(500, 500, 500, 500);
    let committed = Size::from((500, 1000));
    let anchors = SurfaceAnchors::between(Rect::new(500, 0, 500, 1000), target, 0, 0);

    assert!(!resize_removal_is_offscreen_at_target(
        target,
        committed,
        0,
        anchors,
        &[Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)],
    ));
    assert!(!resize_removal_is_offscreen_at_target(
        Rect::new(500, 400, 500, 500),
        committed,
        0,
        anchors,
        &[Rect::new(0, 0, 1000, 1000)],
    ));
}

#[test]
fn offscreen_growth_is_requested_before_movement() {
    let start = Instant::now();
    let from = Rect::new(0, 500, 500, 500);
    let target = Rect::new(0, 0, 500, 1000);
    let committed = Size::from((500, 500));
    let output = Rect::new(0, 0, 1000, 1000);
    let mut animation = WaylandWindowAnimation::new(
        from,
        target,
        committed,
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );

    assert!(resize_growth_is_offscreen_at_start(
        from,
        target,
        committed,
        0,
        animation.anchors,
        &[output],
    ));
    animation.resize_configure_phase = OFFSCREEN_GROWTH_CONFIGURE_PHASE;
    assert_eq!(
        animation.tick(start, committed).configure_size,
        Some(Size::from((500, 1000)))
    );
}

#[test]
fn growth_stays_mid_animation_when_new_pixels_touch_an_output() {
    let from = Rect::new(0, 500, 500, 500);
    let target = Rect::new(0, 0, 500, 1000);
    let committed = Size::from((500, 500));
    let anchors = SurfaceAnchors::between(from, target, 0, 0);

    assert!(!resize_growth_is_offscreen_at_start(
        from,
        target,
        committed,
        0,
        anchors,
        &[Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)],
    ));
}

#[test]
fn offscreen_resize_allows_five_percent_visible_leeway() {
    let target = Rect::new(0, 0, 500, 1000);
    let committed = Size::from((500, 500));
    let output = Rect::new(0, 0, 1000, 1000);
    let five_percent_visible = Rect::new(0, 475, 500, 500);
    let more_than_five_percent_visible = Rect::new(0, 474, 500, 500);

    assert!(resize_growth_is_offscreen_at_start(
        five_percent_visible,
        target,
        committed,
        0,
        SurfaceAnchors::between(five_percent_visible, target, 0, 0),
        // Mirrored outputs must not count the visible area twice.
        &[output, output],
    ));
    assert!(!resize_growth_is_offscreen_at_start(
        more_than_five_percent_visible,
        target,
        committed,
        0,
        SurfaceAnchors::between(more_than_five_percent_visible, target, 0, 0),
        &[output],
    ));
}

#[test]
fn offscreen_percentage_handles_maximum_surface_dimensions() {
    assert!(extra_surface_pixels_are_offscreen(
        Rect::new(0, 0, 1, 1),
        Size::from((1, 1)),
        Size::from((i32::MAX, i32::MAX)),
        0,
        SurfaceAnchors {
            x: SurfaceAnchor::Near,
            y: SurfaceAnchor::Near,
        },
        &[Rect::new(0, 0, 2, 1)],
    ));
}

#[test]
fn immediate_growth_falls_back_if_it_becomes_visible_before_the_first_tick() {
    let start = Instant::now();
    let from = Rect::new(0, 500, 500, 500);
    let target = Rect::new(0, 0, 500, 1000);
    let committed = Size::from((500, 500));
    let mut animation = WaylandWindowAnimation::new(
        from,
        target,
        committed,
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );
    animation.resize_timing = ResizeTiming::OffscreenGrowth;
    animation.resize_configure_phase = OFFSCREEN_GROWTH_CONFIGURE_PHASE;

    animation.revalidate_offscreen_resize_phase(
        committed,
        &[Rect::new(0, 0, 1000, 1000), Rect::new(0, 1000, 1000, 1000)],
    );

    assert_eq!(animation.resize_configure_phase, RESIZE_CONFIGURE_PHASE);
    assert_eq!(animation.tick(start, committed).configure_size, None);
}

#[test]
fn delayed_shrink_falls_back_after_an_unsafe_late_commit() {
    let start = Instant::now();
    let target = Rect::new(500, 500, 500, 500);
    let initial_committed = Size::from((500, 1000));
    let late_committed = Size::from((500, 400));
    let output = Rect::new(0, 0, 1000, 1000);
    let mut animation = WaylandWindowAnimation::new(
        Rect::new(500, 0, 500, 1000),
        target,
        initial_committed,
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );
    animation.resize_timing = ResizeTiming::OffscreenShrink;
    animation.resize_configure_phase = 1.0;

    animation.revalidate_offscreen_resize_phase(late_committed, &[output]);

    assert_eq!(animation.resize_configure_phase, RESIZE_CONFIGURE_PHASE);
    assert_eq!(
        animation
            .tick(start + Duration::from_millis(50), late_committed)
            .configure_size,
        Some(Size::from((500, 500)))
    );
}

#[test]
fn movement_only_transition_never_schedules_a_resize() {
    let start = Instant::now();
    let mut animation = WaylandWindowAnimation::new(
        Rect::new(0, 0, 100, 80),
        Rect::new(50, 20, 100, 80),
        Size::from((100, 80)),
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );

    assert_eq!(
        animation
            .tick(start + Duration::from_millis(100), Size::from((100, 80)))
            .configure_size,
        None
    );
    assert_eq!(animation.resize, ResizeConfigure::Unchanged);
}

#[test]
fn resize_schedule_compares_target_with_committed_not_visual_size() {
    let start = Instant::now();
    let needs_resize = WaylandWindowAnimation::new(
        Rect::new(0, 0, 120, 80),
        Rect::new(10, 0, 120, 80),
        Size::from((140, 80)),
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );
    let visual_size_differs_but_client_is_ready = WaylandWindowAnimation::new(
        Rect::new(0, 0, 120, 80),
        Rect::new(10, 0, 140, 80),
        Size::from((140, 80)),
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );

    assert!(matches!(needs_resize.resize, ResizeConfigure::Pending(_)));
    assert_eq!(
        visual_size_differs_but_client_is_ready.resize,
        ResizeConfigure::Unchanged
    );
}

#[test]
fn resize_schedule_supersedes_a_stale_outstanding_configure() {
    let animation = WaylandWindowAnimation::new(
        Rect::new(0, 0, 60, 80),
        Rect::new(0, 0, 100, 80),
        Size::from((100, 80)),
        Some((60, 80)),
        Duration::from_millis(100),
        Instant::now(),
        0,
        0,
    );

    assert_eq!(
        animation.resize,
        ResizeConfigure::Pending(Size::from((100, 80)))
    );
}

#[test]
fn borders_interpolate_from_the_displayed_to_the_target_width() {
    let start = Instant::now();
    let mut animation = WaylandWindowAnimation::new(
        Rect::new(0, 30, 1200, 740),
        Rect::new(150, 126, 896, 573),
        Size::from((1200, 740)),
        None,
        Duration::from_millis(100),
        start,
        0,
        2,
    );

    // Starts at the displayed (pre-transition) width: a borderless tile.
    assert_eq!(animation.displayed_border_width(), 0);

    // Early in the eased transition the width has not reached the target.
    animation.tick(start + Duration::from_millis(10), Size::from((1200, 740)));
    let early = animation.displayed_border_width();
    assert!(early > 0 && early < 2);

    // Border and buffer-size presentation are independent. The horizontal
    // Horizontal edges move equally, so that axis follows the near edge.
    // Vertically, the farther-moving near edge carries the surface while
    // the border follows the eased frame progress.
    let midpoint = animation.tick(start + Duration::from_millis(50), Size::from((1200, 740)));
    assert_eq!(midpoint.surface_location, Point::from((133, 116)));
    assert_eq!(animation.displayed_border_width(), 2);

    // The transition ends fully bordered, landed exactly on the target
    // inner rectangle: content origin offset by the final border width.
    let tick = animation.tick(start + Duration::from_millis(100), Size::from((896, 573)));
    assert_eq!(tick.surface_location, Point::from((152, 128)));
    assert_eq!(animation.displayed_border_width(), 2);
    assert_eq!(animation.target_border_width(), 2);
}

#[test]
fn centered_float_to_single_tile_anchors_near_edges_so_the_surface_moves() {
    let start = Instant::now();
    let from = Rect::new(198, 313, 600, 400);
    let to = Rect::new(0, 30, 1000, 970);
    let committed = Size::from((600, 400));
    let mut animation = WaylandWindowAnimation::new(
        from,
        to,
        committed,
        None,
        Duration::from_millis(100),
        start,
        2,
        0,
    );

    assert_eq!(animation.anchors.x, SurfaceAnchor::Near);
    assert_eq!(animation.anchors.y, SurfaceAnchor::Near);

    let midpoint = animation.tick(start + Duration::from_millis(50), committed);
    assert!(midpoint.surface_location.x < from.x + 2);
    assert!(midpoint.surface_location.y < from.y + 2);

    let end = animation.tick(start + Duration::from_millis(100), Size::from((1000, 970)));
    assert_eq!(end.surface_location, Point::from((to.x, to.y)));
}

#[test]
fn right_half_to_bottom_right_quarter_follows_the_moving_top_edge() {
    let animation = WaylandWindowAnimation::new(
        Rect::new(500, 0, 500, 1000),
        Rect::new(500, 500, 500, 500),
        Size::from((500, 1000)),
        None,
        Duration::from_millis(100),
        Instant::now(),
        0,
        0,
    );

    assert_eq!(animation.anchors.x, SurfaceAnchor::Near);
    assert_eq!(animation.anchors.y, SurfaceAnchor::Near);
}

#[test]
fn bottom_left_quarter_to_left_half_follows_the_moving_top_edge() {
    let start = Instant::now();
    let committed = Size::from((500, 500));
    let mut animation = WaylandWindowAnimation::new(
        Rect::new(0, 500, 500, 500),
        Rect::new(0, 0, 500, 1000),
        committed,
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );

    assert_eq!(animation.anchors.y, SurfaceAnchor::Near);
    let midpoint = animation.tick(start + Duration::from_millis(50), committed);
    assert!(midpoint.surface_location.y < 500);
}

#[test]
fn retarget_keeps_the_anchor_matching_the_current_surface_location() {
    let frame = Rect::new(0, 0, 120, 80);
    let committed = Size::from((100, 80));
    let current_location = Point::from((20, 0));
    let preferred = SurfaceAnchors {
        x: SurfaceAnchor::Near,
        y: SurfaceAnchor::Near,
    };

    assert_eq!(
        preferred.continuous_with(frame, committed, 0, current_location),
        SurfaceAnchors {
            x: SurfaceAnchor::Far,
            y: SurfaceAnchor::Near,
        }
    );
}

#[test]
fn one_sided_growth_follows_the_moving_far_edge() {
    let anchors = SurfaceAnchors::between(Rect::new(0, 0, 100, 80), Rect::new(0, 0, 140, 80), 0, 0);
    let halfway_frame = Rect::new(0, 0, 120, 80);

    let before_resize = anchored_surface_location(halfway_frame, Size::from((100, 80)), 0, anchors);
    let after_resize = anchored_surface_location(halfway_frame, Size::from((140, 80)), 0, anchors);

    assert_eq!(anchors.x, SurfaceAnchor::Far);
    assert_eq!(before_resize, Point::from((20, 0)));
    assert_eq!(after_resize, Point::from((-20, 0)));
    assert_eq!(
        anchored_surface_location(Rect::new(0, 0, 140, 80), Size::from((140, 80)), 0, anchors,),
        Point::from((0, 0))
    );
}

#[test]
fn one_sided_shrink_follows_the_moving_near_edge() {
    let anchors = SurfaceAnchors::between(Rect::new(0, 0, 100, 80), Rect::new(40, 0, 60, 80), 0, 0);
    let halfway_frame = Rect::new(20, 0, 80, 80);

    let before_resize = anchored_surface_location(halfway_frame, Size::from((100, 80)), 0, anchors);
    let after_resize = anchored_surface_location(halfway_frame, Size::from((60, 80)), 0, anchors);

    assert_eq!(anchors.x, SurfaceAnchor::Near);
    assert_eq!(before_resize, Point::from((20, 0)));
    assert_eq!(after_resize, Point::from((20, 0)));
    assert_eq!(
        anchored_surface_location(Rect::new(40, 0, 60, 80), Size::from((60, 80)), 0, anchors,),
        Point::from((40, 0))
    );
}

#[test]
fn far_anchored_completion_waits_for_the_target_committed_size() {
    let start = Instant::now();
    let mut animation = WaylandWindowAnimation::new(
        Rect::new(0, 0, 100, 80),
        Rect::new(0, 0, 140, 80),
        Size::from((100, 80)),
        None,
        Duration::from_millis(100),
        start,
        0,
        0,
    );

    assert_eq!(animation.anchors.x, SurfaceAnchor::Far);
    assert!(animation.needs_landing(Size::from((100, 80))));
    assert!(!animation.needs_landing(Size::from((140, 80))));

    let tick = animation.tick(start + Duration::from_millis(100), Size::from((100, 80)));
    assert!(tick.done);
    assert!(tick.waiting_for_resize);
    assert!(animation.is_waiting_for_resize());
}

#[test]
fn every_edge_combination_anchors_the_farther_traveled_edge() {
    let border = 4;
    let from = Rect::new(0, 0, 100, 80);

    for left_delta in [-20, 0, 20] {
        for right_delta in [-20, 0, 20] {
            for top_delta in [-20, 0, 20] {
                for bottom_delta in [-20, 0, 20] {
                    let left = left_delta;
                    let right = 100 + right_delta;
                    let top = top_delta;
                    let bottom = 80 + bottom_delta;
                    let frame = Rect::new(left, top, right - left, bottom - top);
                    assert!(frame.is_valid());
                    let anchors = SurfaceAnchors::between(from, frame, border, border);
                    assert_eq!(
                        anchors.x,
                        if left_delta.abs() >= right_delta.abs() {
                            SurfaceAnchor::Near
                        } else {
                            SurfaceAnchor::Far
                        }
                    );
                    assert_eq!(
                        anchors.y,
                        if top_delta.abs() >= bottom_delta.abs() {
                            SurfaceAnchor::Near
                        } else {
                            SurfaceAnchor::Far
                        }
                    );

                    let committed_sizes = [
                        Size::from((100, 80)),
                        Size::from((frame.w, frame.h)),
                        Size::from((120, 60)),
                    ];

                    for committed in committed_sizes {
                        let location = anchored_surface_location(frame, committed, border, anchors);
                        match anchors.x {
                            SurfaceAnchor::Near => assert_eq!(location.x, frame.x + border),
                            SurfaceAnchor::Far => {
                                assert_eq!(location.x + committed.w, frame.x + border + frame.w)
                            }
                        }
                        match anchors.y {
                            SurfaceAnchor::Near => assert_eq!(location.y, frame.y + border),
                            SurfaceAnchor::Far => {
                                assert_eq!(location.y + committed.h, frame.y + border + frame.h)
                            }
                        }
                    }
                }
            }
        }
    }
}
