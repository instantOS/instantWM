use super::cursor_size;

#[test]
fn cursor_size_stays_in_xcursor_range() {
    assert_eq!(cursor_size(0), 1);
    assert_eq!(cursor_size(24), 24);
    assert_eq!(cursor_size(512), u8::MAX);
}
use smithay::reexports::drm::control::from_u32;

use super::layout::{
    OutputHitRegion, output_layout_bounds, output_layout_size, outputs_at_pointer,
};
use crate::types::{Point, Rect, Size};

fn crtc(raw: u32) -> super::ScanoutId {
    super::ScanoutId {
        device: super::PRIMARY_DEVICE,
        crtc: from_u32(raw).expect("test CRTC handles are non-zero"),
    }
}

#[test]
fn session_reactivation_reprobes_outputs_without_fabricating_flip_completion() {
    use super::{DrmLayoutState, DrmLoopState, DrmRuntimeEvent, process_runtime_events};
    use smithay::utils::{Clock, Monotonic};
    use std::sync::mpsc;

    let panel = crtc(1);
    let mut runtime = DrmLoopState::new(&[]);
    runtime.add_output(panel);
    runtime.render_flags.insert(panel, false);
    runtime.pending_crtcs.insert(panel);
    let layout = DrmLayoutState {
        layout: Rect::new(0, 0, 1920, 1080),
        output_hit_regions: vec![],
    };
    let (tx, rx) = mpsc::channel();
    tx.send(DrmRuntimeEvent::SessionPaused).unwrap();
    process_runtime_events(
        &rx,
        &mut runtime,
        &layout,
        &mut [],
        &Clock::<Monotonic>::new(),
    );
    assert!(!runtime.session_active);
    assert!(!runtime.has_renderable_dirty_outputs());

    // Hotplug may arrive before we regain DRM ownership. Keep it queued.
    assert!(!runtime.take_topology_change(true));
    assert!(!runtime.take_topology_change(false));

    // Resume must also request a scan independently of udev notifications.
    tx.send(DrmRuntimeEvent::SessionActivated).unwrap();
    let (_, topology_changed) = process_runtime_events(
        &rx,
        &mut runtime,
        &layout,
        &mut [],
        &Clock::<Monotonic>::new(),
    );
    assert!(runtime.session_active);
    assert!(topology_changed);
    assert!(runtime.take_topology_change(topology_changed));
    assert!(!runtime.take_topology_change(false));
    assert!(runtime.pending_crtcs.contains(&panel));
    assert!(!runtime.has_renderable_dirty_outputs());

    // A delayed real completion, rather than resume, releases the redraw.
    tx.send(DrmRuntimeEvent::VBlank(panel)).unwrap();
    process_runtime_events(
        &rx,
        &mut runtime,
        &layout,
        &mut [],
        &Clock::<Monotonic>::new(),
    );
    assert!(runtime.pending_crtcs.is_empty());
    assert!(runtime.has_renderable_dirty_outputs());
}

#[test]
fn equal_crtc_handles_on_distinct_devices_complete_independently() {
    use super::{
        DrmDeviceId, DrmLayoutState, DrmLoopState, DrmRuntimeEvent, process_runtime_events,
    };
    use smithay::utils::{Clock, Monotonic};
    use std::sync::mpsc;
    let primary = crtc(1);
    let secondary = super::ScanoutId {
        device: DrmDeviceId(1),
        ..primary
    };
    let mut runtime = DrmLoopState::new(&[]);
    for scanout in [primary, secondary] {
        runtime.add_output(scanout);
        runtime.pending_crtcs.insert(scanout);
    }
    let layout = DrmLayoutState {
        layout: Rect::new(0, 0, 1, 1),
        output_hit_regions: vec![],
    };
    let (tx, rx) = mpsc::channel();
    tx.send(DrmRuntimeEvent::VBlank(secondary)).unwrap();
    process_runtime_events(
        &rx,
        &mut runtime,
        &layout,
        &mut [],
        &Clock::<Monotonic>::new(),
    );
    assert!(runtime.pending_crtcs.contains(&primary));
    assert!(!runtime.pending_crtcs.contains(&secondary));
    assert_eq!(runtime.render_flags.len(), 2);
    runtime.remove_output(secondary);
    assert!(runtime.render_flags.contains_key(&primary));
    assert!(runtime.pending_crtcs.contains(&primary));
}

#[test]
fn delayed_completion_from_removed_device_cannot_release_its_replacement() {
    use super::{
        DrmDeviceId, DrmLayoutState, DrmLoopState, DrmRuntimeEvent, process_runtime_events,
    };
    use smithay::utils::{Clock, Monotonic};
    use std::sync::mpsc;
    let old = super::ScanoutId {
        device: DrmDeviceId(1),
        ..crtc(1)
    };
    let replacement = super::ScanoutId {
        device: DrmDeviceId(2),
        ..old
    };
    let mut runtime = DrmLoopState::new(&[]);
    runtime.add_output(old);
    runtime.remove_output(old);
    runtime.add_output(replacement);
    runtime.pending_crtcs.insert(replacement);
    let layout = DrmLayoutState {
        layout: Rect::new(0, 0, 1, 1),
        output_hit_regions: vec![],
    };
    let (tx, rx) = mpsc::channel();
    tx.send(DrmRuntimeEvent::VBlank(old)).unwrap();
    process_runtime_events(
        &rx,
        &mut runtime,
        &layout,
        &mut [],
        &Clock::<Monotonic>::new(),
    );
    assert!(runtime.pending_crtcs.contains(&replacement));
    assert!(!runtime.render_flags.contains_key(&old));
}

#[test]
fn removed_output_recovers_the_pointer_even_with_a_gap_in_the_layout() {
    use crate::test_support::MonitorBuilder;
    let (_event_loop, mut state) = crate::test_support::new_compositor();
    let output = state
        .native
        .create_output("DP-1", Size::new(1920, 1080), None);
    state.native.space.map_output(&output, (1920, 0));
    state.native.runtime.pointer_location = (50.0, 50.0).into();
    state.wm = crate::wm::WaylandWm::new(crate::backend::WaylandBackendData::default());
    state.wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .named("DP-1")
            .monitor_rect(Rect::new(1920, 0, 1920, 1080))
            .build(),
    );

    super::layout::recover_pointer_after_output_change(&mut state);
    assert_eq!(state.native.pending_warp, Some((2880.0, 540.0).into()));
    let pointer = state.native.pointer.clone();
    let keyboard = state.native.keyboard.clone();
    assert!(crate::backend::wayland::input::apply_pending_warp(
        &mut state, &pointer, &keyboard
    ));
    assert_eq!(
        state.native.runtime.pointer_location,
        (2880.0, 540.0).into()
    );
    assert!(
        state
            .native
            .space
            .output_under(state.native.runtime.pointer_location)
            .next()
            .is_some()
    );
}

#[test]
fn output_changes_do_not_warp_a_pointer_already_on_a_remaining_display() {
    let (_event_loop, mut state) = crate::test_support::new_compositor();
    let output = state
        .native
        .create_output("DP-1", Size::new(1920, 1080), None);
    state.native.space.map_output(&output, (0, 0));
    state.native.runtime.pointer_location = (50.0, 50.0).into();
    state.wm = crate::wm::WaylandWm::new(crate::backend::WaylandBackendData::default());
    super::layout::recover_pointer_after_output_change(&mut state);
    assert!(state.native.pending_warp.is_none());
}

#[test]
fn stacked_outputs_with_overlapping_x_ranges_route_by_both_axes() {
    let top_left = crtc(1);
    let top_right = crtc(2);
    let bottom = crtc(3);
    let regions = [
        OutputHitRegion {
            scanout: top_left,
            rect: Rect::new(0, 0, 1920, 1080),
        },
        OutputHitRegion {
            scanout: top_right,
            rect: Rect::new(1920, 0, 1920, 1080),
        },
        OutputHitRegion {
            scanout: bottom,
            rect: Rect::new(960, 1080, 1920, 1080),
        },
    ];

    assert_eq!(
        outputs_at_pointer(&regions, Point::new(1200, 500)).collect::<Vec<_>>(),
        vec![top_left]
    );
    assert_eq!(
        outputs_at_pointer(&regions, Point::new(2200, 500)).collect::<Vec<_>>(),
        vec![top_right]
    );
    assert_eq!(
        outputs_at_pointer(&regions, Point::new(1200, 1500)).collect::<Vec<_>>(),
        vec![bottom]
    );
    assert_eq!(
        outputs_at_pointer(&regions, Point::new(4000, 1500)).count(),
        0
    );
}

#[test]
fn overlapping_heads_both_match_the_pointer() {
    let first = crtc(1);
    let second = crtc(2);
    let neighbor = crtc(3);
    let shared_rect = Rect::new(0, 0, 1920, 1080);
    let regions = [
        OutputHitRegion {
            scanout: first,
            rect: shared_rect,
        },
        OutputHitRegion {
            scanout: second,
            rect: shared_rect,
        },
        OutputHitRegion {
            scanout: neighbor,
            rect: Rect::new(1920, 0, 1920, 1080),
        },
    ];

    // Both overlapping CRTCs are returned so both are dirtied.
    assert_eq!(
        outputs_at_pointer(&regions, Point::new(100, 100)).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(
        outputs_at_pointer(&regions, Point::new(2000, 100)).collect::<Vec<_>>(),
        vec![neighbor]
    );
    assert_eq!(
        outputs_at_pointer(&regions, Point::new(4000, 1500)).count(),
        0
    );
}

#[test]
fn layout_size_includes_vertical_offsets() {
    let outputs = [
        Rect::new(0, 0, 3840, 1080),
        Rect::new(960, 1080, 1920, 1080),
    ];

    assert_eq!(
        output_layout_size(outputs, Size::new(1, 1)),
        Size::new(3840, 2160)
    );
}

#[test]
fn empty_layout_uses_its_callers_fallback() {
    assert_eq!(
        output_layout_size([], Size::new(1280, 800)),
        Size::new(1280, 800)
    );
}

#[test]
fn layout_bounds_keep_negative_origin_for_absolute_mapping() {
    let outputs = [Rect::new(0, -1080, 1920, 1080), Rect::new(0, 0, 1920, 1080)];

    assert_eq!(
        output_layout_bounds(outputs, Size::new(1, 1)),
        Rect::new(0, -1080, 1920, 2160)
    );
    assert_eq!(
        output_layout_size(outputs, Size::new(1, 1)),
        Size::new(1920, 2160)
    );
}
