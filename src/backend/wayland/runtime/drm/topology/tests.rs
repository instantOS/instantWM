//! Exercise production disconnect teardown without DRM or an active display.

use super::*;
use crate::backend::BackendVrrSupport;
use crate::backend::output::{OutputPositionSource, OutputPowerMode, OutputPowerRequest};
use crate::backend::wayland::render::drm::DrmDeviceId;
use crate::config::config_toml::VrrMode;
use crate::types::{Rect, Size};
use smithay::reexports::drm::control::from_u32;
use std::collections::HashMap;

fn entry(state: &mut WaylandState, name: &str, device: u64) -> OutputSurfaceEntry {
    let output = state
        .native
        .create_output(name, Size::new(1920, 1080), None);
    OutputSurfaceEntry {
        device: DrmDeviceId(device),
        crtc: from_u32(1).unwrap(),
        connector: from_u32(1).unwrap(),
        surface: None,
        modes: Vec::new(),
        output,
        dmabuf_feedback: None,
        rect: Rect::new(0, 0, 1920, 1080),
        vrr_support: BackendVrrSupport::Unsupported,
        configured_vrr_mode: VrrMode::Off,
        vrr_enabled: false,
        enabled: true,
        powered: true,
        pending_power_on: None,
    }
}

#[test]
fn unplugging_a_mirror_source_releases_the_survivor_and_cleans_inflight_work() {
    let (_event_loop, mut state) = crate::test_support::new_compositor();
    let mut source = entry(&mut state, "DP-1", 0);
    let mirror = entry(&mut state, "DVI-I-1", 1);
    state
        .native
        .set_mirror_roles(HashMap::from([("DVI-I-1".into(), "DP-1".into())]));
    assert_eq!(state.native.space.outputs().count(), 1);
    state
        .native
        .queue_output_policy_projection(&Default::default());
    let requested = state
        .native
        .runtime
        .output_transactions
        .latest_pending_apply()
        .unwrap()
        .clone();
    state
        .native
        .runtime
        .requested_output_state
        .commit(&requested);
    state
        .native
        .runtime
        .output_position_sources
        .insert("DP-1".into(), OutputPositionSource::ClientManaged);
    state
        .native
        .runtime
        .output_power_modes
        .insert("DP-1".into(), OutputPowerMode::On);

    let power_id = state
        .native
        .runtime
        .output_power
        .submit(OutputPowerRequest {
            output: OutputId("DP-1".into()),
            mode: OutputPowerMode::On,
        });
    state
        .native
        .runtime
        .output_power
        .take_next_pending()
        .unwrap();
    source.pending_power_on = Some(power_id);
    let mut runtime = DrmLoopState::new(&[]);
    for scanout in [source.scanout(), mirror.scanout()] {
        runtime.add_output(scanout);
        runtime.pending_crtcs.insert(scanout);
    }
    runtime.take_render_flags();
    let retired = source.scanout();
    let survivor = mirror.scanout();
    retire_disconnected_outputs(&mut state, &mut runtime, vec![source]);

    assert_eq!(
        state.native.output_management_state.outputs(),
        std::slice::from_ref(&mirror.output)
    );
    assert_eq!(
        state
            .native
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<Vec<_>>(),
        vec!["DVI-I-1"]
    );
    assert!(state.native.runtime.realized_mirrors.is_empty());
    assert!(
        !state
            .native
            .runtime
            .output_position_sources
            .contains_key("DP-1")
    );
    assert!(!state.native.runtime.output_power_modes.contains_key("DP-1"));
    assert!(!state.native.runtime.output_metadata.contains_key("DP-1"));
    assert!(!runtime.pending_crtcs.contains(&retired));
    assert!(!runtime.render_flags.contains_key(&retired));
    assert!(!runtime.taken_render_flags.contains_key(&retired));
    assert!(!runtime.presentation_seq.contains_key(&retired));
    assert!(runtime.pending_crtcs.contains(&survivor));
    let completed = state.native.runtime.output_power.take_completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].0, power_id);
    assert_eq!(
        completed[0].1,
        (
            OutputId("DP-1".into()),
            Err(OutputPowerError::Unavailable("DP-1".into()))
        )
    );
    state
        .native
        .queue_output_policy_projection(&Default::default());
    assert_eq!(
        state
            .native
            .runtime
            .output_transactions
            .latest_pending_apply()
            .unwrap()
            .heads
            .len(),
        1
    );
    retire_disconnected_outputs(&mut state, &mut runtime, Vec::new());
    assert!(runtime.pending_crtcs.contains(&survivor));
}

#[test]
fn removing_multiple_dock_devices_preserves_an_equal_primary_crtc_handle() {
    let (_event_loop, mut state) = crate::test_support::new_compositor();
    let primary = entry(&mut state, "eDP-1", 0);
    let dock_a = entry(&mut state, "DP-1", 1);
    let dock_b = entry(&mut state, "DVI-I-1", 2);
    let mut runtime = DrmLoopState::new(&[]);
    for scanout in [primary.scanout(), dock_a.scanout(), dock_b.scanout()] {
        runtime.add_output(scanout);
        runtime.pending_crtcs.insert(scanout);
    }
    retire_disconnected_outputs(&mut state, &mut runtime, vec![dock_b, dock_a]);
    assert_eq!(runtime.pending_crtcs, HashSet::from([primary.scanout()]));
    assert_eq!(runtime.render_flags.len(), 1);
    assert_eq!(
        state.native.output_management_state.outputs(),
        &[primary.output]
    );
}
