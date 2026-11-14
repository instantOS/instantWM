//! Headless docking scenarios. Native commits are acknowledged by the fixture;
//! policy, request queues, validation, snapshots and output roles are production
//! code. These tests neither open DRM devices nor inherit a desktop socket.

use super::*;
use crate::backend::output::{OutputConnectorKind, OutputHeadCapabilities, OutputHeadSnapshot};
use crate::config::config_toml::MonitorConfig;
use std::collections::{HashMap, HashSet};

struct Docking {
    state: WaylandState,
    configs: HashMap<String, MonitorConfig>,
    closed: bool,
    paused: bool,
}

impl Docking {
    fn new() -> Self {
        let mut fixture = Self {
            state: three_outputs(),
            configs: HashMap::new(),
            closed: false,
            paused: false,
        };
        fixture.queue_policy();
        assert!(fixture.flush().is_empty());
        fixture
    }

    fn queue_policy(&mut self) {
        for (name, config) in &self.configs {
            self.state.native.set_output_config(name, config);
        }
        self.state
            .native
            .queue_output_policy_projection(&self.configs);
    }

    fn lid(&mut self, closed: bool) {
        self.closed = closed;
        self.state
            .native
            .runtime
            .lid_output_policy
            .set_closed(closed);
        self.queue_policy();
    }

    fn toggle_connector(&mut self, name: &str) {
        let output = self
            .state
            .native
            .output_management_state
            .outputs()
            .iter()
            .find(|output| output.name() == name)
            .cloned();
        if let Some(output) = output {
            self.state
                .native
                .output_management_state
                .remove_heads::<WaylandState>(std::iter::once(&output));
            self.state.native.space.unmap_output(&output);
            self.state.native.set_output_global_enabled(&output, false);
            self.state
                .native
                .runtime
                .requested_output_state
                .forget(&OutputId(name.into()));
            self.state
                .native
                .runtime
                .output_position_sources
                .remove(name);
            self.state.native.runtime.output_metadata.remove(name);
            self.state.native.drop_unavailable_mirrors();
        } else {
            let output = self.state.native.create_output_global(
                name.into(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "test".into(),
                    model: "test".into(),
                    serial_number: "test".into(),
                },
                OutputMode {
                    size: (1920, 1080).into(),
                    refresh: 60_000,
                },
                Point::default(),
                if name == "eDP-1" {
                    OutputConnectorKind::Internal
                } else {
                    OutputConnectorKind::External
                },
            );
            self.state
                .native
                .output_management_state
                .add_heads::<WaylandState>(std::iter::once(&output));
            self.state.native.space.map_output(&output, (0, 0));
        }
        self.queue_policy();
    }

    fn capabilities(&self) -> Vec<OutputHeadCapabilities> {
        self.state
            .native
            .output_management_state
            .outputs()
            .iter()
            .map(|output| OutputHeadCapabilities {
                id: OutputId(output.name()),
                modes: output.modes().into_iter().map(from_smithay_mode).collect(),
                adaptive_sync: false,
            })
            .collect()
    }

    fn change_mode(&mut self, name: &str) {
        if let Some(output) = self
            .state
            .native
            .output_management_state
            .outputs()
            .iter()
            .find(|output| output.name() == name)
        {
            let width = if output
                .modes()
                .first()
                .is_some_and(|mode| mode.size.w == 1920)
            {
                1280
            } else {
                1920
            };
            for mode in output.modes() {
                output.delete_mode(mode);
            }
            let replacement = OutputMode {
                size: (width, width * 9 / 16).into(),
                refresh: 59_940,
            };
            output.add_mode(replacement);
            output.set_preferred(replacement);
        }
        self.queue_policy();
    }

    /// Model only the hardware acknowledgement. A failure or test must never
    /// commit intent or project a new desktop, just as in the DRM apply path.
    fn flush(&mut self) -> Vec<OutputTransactionError> {
        if self.paused {
            return Vec::new();
        }
        let mut errors = Vec::new();
        while let Some((id, mut pending)) = self
            .state
            .native
            .runtime
            .output_transactions
            .take_next_pending()
        {
            let capabilities = self.capabilities();
            // Stand in for the connector's native modes/preferred flag, using
            // the same reconciliation helper as the DRM policy apply path.
            if pending.origin == OutputTransactionOrigin::CompositorPolicy {
                for head in &mut pending.transaction.heads {
                    if let Some(capability) = capabilities.iter().find(|cap| cap.id == head.id) {
                        let preferred = self
                            .state
                            .native
                            .output_management_state
                            .outputs()
                            .iter()
                            .find(|output| output.name() == head.id.0)
                            .and_then(Output::preferred_mode)
                            .map(from_smithay_mode);
                        head.reconcile_mode(&capability.modes, preferred);
                    }
                }
            }
            let effective = self
                .state
                .native
                .effective_output_transaction(&pending.transaction, pending.origin);
            let validation = self
                .state
                .native
                .lid_output_projection(&pending.transaction)
                .validate_transaction(&pending.transaction, &effective, &capabilities);
            let result = validation.map(|()| {
                if pending.kind == OutputTransactionKind::Apply {
                    self.state
                        .native
                        .runtime
                        .requested_output_state
                        .commit(&pending.transaction);
                    if pending.origin == OutputTransactionOrigin::Client {
                        for head in &pending.transaction.heads {
                            self.state
                                .native
                                .runtime
                                .output_position_sources
                                .insert(head.id.0.clone(), OutputPositionSource::ClientManaged);
                        }
                    }
                }
                OutputSnapshot {
                    heads: effective
                        .heads
                        .into_iter()
                        .map(|configuration| OutputHeadSnapshot {
                            modes: capabilities
                                .iter()
                                .find(|cap| cap.id == configuration.id)
                                .unwrap()
                                .modes
                                .clone(),
                            configuration,
                            adaptive_sync_policy: AdaptiveSyncPolicy::Disabled,
                            adaptive_sync_enabled: false,
                        })
                        .collect(),
                }
            });
            if let Err(error) = &result {
                errors.push(error.clone());
            }
            self.state
                .native
                .runtime
                .output_transactions
                .complete(id, (pending.kind, result));
            self.state.native.project_completed_output_transactions();
        }
        errors
    }

    fn check(&self, trace: &str) {
        let observed = self.state.native.observed_output_transaction();
        let actual: HashSet<_> = self
            .state
            .native
            .space
            .outputs()
            .map(Output::name)
            .collect();
        let mirrors = &self.state.native.runtime.realized_mirrors;
        let expected: HashSet<_> = observed
            .heads
            .iter()
            .filter(|head| head.enabled && !mirrors.contains_key(&head.id.0))
            .map(|head| head.id.0.clone())
            .collect();
        assert_eq!(actual, expected, "{trace}");
        for head in &observed.heads {
            assert!(
                !(self.closed && head.id.0 == "eDP-1" && head.enabled),
                "closed panel lit: {trace}"
            );
            if head.enabled {
                assert!(
                    self.capabilities()
                        .iter()
                        .find(|cap| cap.id == head.id)
                        .unwrap()
                        .modes
                        .contains(&head.mode.unwrap()),
                    "{trace}"
                );
            }
        }
        for (mirror, source) in mirrors {
            let head = observed
                .heads
                .iter()
                .find(|head| &head.id.0 == mirror)
                .unwrap();
            let source = observed
                .heads
                .iter()
                .find(|head| &head.id.0 == source)
                .unwrap();
            assert!(head.enabled && source.enabled, "{trace}");
            assert_eq!(
                (head.position, head.scale),
                (source.position, source.scale),
                "{trace}"
            );
        }
        let requested = self.state.native.requested_output_transaction();
        let connected: HashSet<_> = observed.heads.iter().map(|head| head.id.clone()).collect();
        assert_eq!(
            requested
                .heads
                .iter()
                .map(|head| head.id.clone())
                .collect::<HashSet<_>>(),
            connected,
            "{trace}"
        );
        assert!(
            !self.state.native.runtime.output_transactions.has_pending(),
            "{trace}"
        );
    }

    fn flush_and_check(&mut self, trace: &str) {
        let disconnected = self
            .state
            .native
            .output_management_state
            .outputs()
            .is_empty();
        let errors = self.flush();
        assert!(
            errors
                .iter()
                .all(|error| disconnected && *error == OutputTransactionError::NoEnabledOutputs),
            "unexpected rejected policy: {errors:?}; {trace}"
        );
        self.check(trace);
    }
}

#[test]
fn rejected_client_disable_all_cannot_be_saved_by_a_queued_lid_recovery() {
    let mut fixture = Docking::new();
    fixture.lid(true);
    fixture.flush();
    let mut invalid = fixture.state.native.observed_output_transaction();
    for head in &mut invalid.heads {
        head.enabled = false;
    }
    fixture
        .state
        .native
        .runtime
        .output_transactions
        .submit_client(OutputTransactionKind::Apply, invalid);
    fixture.lid(false);
    assert_eq!(
        fixture.flush(),
        vec![OutputTransactionError::NoEnabledOutputs]
    );
    assert!(
        fixture
            .state
            .native
            .observed_output_transaction()
            .heads
            .iter()
            .all(|head| head.enabled),
        "a rejected client request must not silently disable either dock display through a policy transaction"
    );
}

#[test]
fn test_only_client_settings_do_not_leak_into_undocking_or_lid_restoration() {
    let mut fixture = Docking::new();
    fixture.lid(true);
    fixture.flush();
    let mut tested = fixture.state.native.requested_output_transaction();
    for head in &mut tested.heads {
        head.scale = 2.0;
    }
    fixture
        .state
        .native
        .runtime
        .output_transactions
        .submit_client(OutputTransactionKind::Test, tested.clone());
    assert!(fixture.flush().is_empty());
    assert!(
        fixture
            .state
            .native
            .observed_output_transaction()
            .heads
            .iter()
            .all(|head| head.scale == 1.0)
    );
    assert!(
        fixture
            .state
            .native
            .requested_output_transaction()
            .heads
            .iter()
            .all(|head| head.scale == 1.0)
    );
    fixture
        .state
        .native
        .runtime
        .output_transactions
        .submit_client(OutputTransactionKind::Test, tested);
    fixture.toggle_connector("DP-1");
    fixture.toggle_connector("HDMI-1");
    fixture.lid(false);
    fixture.flush();
    let restored = fixture.state.native.observed_output_transaction();
    assert_eq!(restored.heads.len(), 1);
    assert!(restored.heads[0].enabled);
    assert_eq!(restored.heads[0].scale, 1.0);
    fixture.check("test settings followed by undock");
}

#[test]
fn client_scale_rotation_and_negative_positions_survive_independent_dock_removal() {
    let mut fixture = Docking::new();
    let mut client = fixture.state.native.requested_output_transaction();
    let panel = client
        .heads
        .iter_mut()
        .find(|head| head.id.0 == "eDP-1")
        .unwrap();
    panel.scale = 1.5;
    panel.transform = OutputTransform::Rotate90;
    panel.position = Point::new(-720, -1280);
    let panel = panel.clone();
    fixture
        .state
        .native
        .runtime
        .output_transactions
        .submit_client(OutputTransactionKind::Apply, client);
    assert!(fixture.flush().is_empty());
    for name in ["DP-1", "HDMI-1"] {
        fixture.lid(true);
        fixture.toggle_connector(name);
        assert!(fixture.flush().is_empty());
        fixture.check("scaled panel closed during partial undock");
        fixture.lid(false);
        assert!(fixture.flush().is_empty());
        assert_eq!(
            fixture
                .state
                .native
                .observed_output_transaction()
                .heads
                .iter()
                .find(|head| head.id == panel.id),
            Some(&panel)
        );
    }
}

#[test]
fn mirror_source_removal_and_redocking_follow_the_current_lid_state() {
    let mut fixture = Docking::new();
    fixture.state.native.runtime.mirror_of = mirror_map(&[("DP-1", "eDP-1")]);
    fixture.queue_policy();
    fixture.flush();
    assert_eq!(fixture.state.native.runtime.realized_mirrors.len(), 1);
    fixture.lid(true);
    fixture.flush();
    assert!(fixture.state.native.runtime.realized_mirrors.is_empty());
    assert!(
        fixture
            .state
            .native
            .space
            .outputs()
            .any(|output| output.name() == "DP-1")
    );
    fixture.toggle_connector("eDP-1");
    fixture.flush();
    fixture.toggle_connector("eDP-1");
    fixture.flush();
    fixture.check("internal connector disappeared and returned behind a closed lid");
    fixture.lid(false);
    fixture.flush();
    assert_eq!(
        fixture
            .state
            .native
            .runtime
            .realized_mirrors
            .get("DP-1")
            .map(String::as_str),
        Some("eDP-1")
    );
    fixture.check("mirror restored after reopening");
}

#[test]
fn mirror_follows_its_automatic_source_when_lid_closure_moves_the_source() {
    let mut fixture = Docking::new();
    fixture.state.native.runtime.mirror_of = mirror_map(&[("HDMI-1", "DP-1")]);
    fixture.queue_policy();
    assert!(fixture.flush().is_empty());
    fixture.lid(true);
    assert!(fixture.flush().is_empty());
    let observed = fixture.state.native.observed_output_transaction();
    let source = observed
        .heads
        .iter()
        .find(|head| head.id.0 == "DP-1")
        .unwrap();
    let mirror = observed
        .heads
        .iter()
        .find(|head| head.id.0 == "HDMI-1")
        .unwrap();
    assert_eq!(source.position, Point::default());
    assert_eq!(mirror.position, source.position);
    fixture.check("lid closure compacts an external mirror source");
}

#[test]
fn stale_client_layout_after_partial_undock_cannot_change_the_surviving_outputs() {
    let mut fixture = Docking::new();
    let mut client = fixture.state.native.requested_output_transaction();
    let panel = client
        .heads
        .iter_mut()
        .find(|head| head.id.0 == "eDP-1")
        .unwrap();
    panel.enabled = false;
    panel.scale = 2.0;
    fixture
        .state
        .native
        .runtime
        .output_transactions
        .submit_client(OutputTransactionKind::Apply, client);
    fixture.toggle_connector("DP-1");
    assert_eq!(
        fixture.flush(),
        vec![OutputTransactionError::UnknownOutput("DP-1".into())]
    );
    let observed = fixture.state.native.observed_output_transaction();
    assert_eq!(observed.heads.len(), 2);
    assert!(
        observed
            .heads
            .iter()
            .all(|head| head.enabled && head.scale == 1.0)
    );
    fixture.check("stale client apply after native DP removal");
}

#[test]
fn invalid_client_request_does_not_hide_an_earlier_queued_monitor_setting() {
    let mut fixture = Docking::new();
    fixture.state.native.set_output_config(
        "eDP-1",
        &MonitorConfig {
            scale: Some(1.25),
            ..Default::default()
        },
    );
    let mut invalid = fixture.state.native.requested_output_transaction();
    for head in &mut invalid.heads {
        head.enabled = false;
    }
    fixture
        .state
        .native
        .runtime
        .output_transactions
        .submit_client(OutputTransactionKind::Apply, invalid);
    fixture.lid(true);
    assert_eq!(
        fixture.flush(),
        vec![OutputTransactionError::NoEnabledOutputs]
    );
    fixture.lid(false);
    assert!(fixture.flush().is_empty());
    let observed = fixture.state.native.observed_output_transaction();
    assert!(observed.heads.iter().all(|head| head.enabled));
    assert_eq!(
        observed
            .heads
            .iter()
            .find(|head| head.id.0 == "eDP-1")
            .unwrap()
            .scale,
        1.25
    );
}

#[test]
fn mode_and_refresh_changes_behind_a_closed_lid_are_applied_without_waking_the_panel() {
    let mut fixture = Docking::new();
    fixture.lid(true);
    assert!(fixture.flush().is_empty());
    fixture.paused = true;
    fixture.change_mode("eDP-1");
    fixture.change_mode("DP-1");
    fixture.toggle_connector("HDMI-1");
    fixture.paused = false;
    fixture.flush_and_check("resume with changed panel/DP modes and DisplayLink gone");
    let requested = fixture.state.native.requested_output_transaction();
    let panel = requested
        .heads
        .iter()
        .find(|head| head.id.0 == "eDP-1")
        .unwrap();
    assert!(panel.enabled);
    assert_eq!(panel.mode.unwrap().width, 1280);
    assert_eq!(panel.mode.unwrap().refresh_millihertz, 59_940);
    fixture.lid(false);
    fixture.flush_and_check("opening restores the new supported mode");
    assert!(
        fixture
            .state
            .native
            .observed_output_transaction()
            .heads
            .iter()
            .all(|head| head.enabled)
    );
}

#[test]
fn generated_docking_sequences_preserve_lid_and_output_invariants() {
    // Replayable walks, including duplicate events and long stretches of
    // queued changes while paused. No timing, entropy or external processes.
    // 128 walks * 96 events = 12,288 events in replayable sequences.
    let mut covered = HashSet::new();
    const EVENTS: [&str; 14] = [
        "close",
        "open",
        "DP",
        "DisplayLink",
        "panel",
        "mirror-panel",
        "mirror-DP",
        "unmirror",
        "pause",
        "resume",
        "queue",
        "flush",
        "DP-mode",
        "panel-mode",
    ];
    for seed in 1..=128u64 {
        let mut fixture = Docking::new();
        let mut random = seed;
        let mut trace = format!("seed {seed}:");
        for _ in 0..96 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let event = (random % 14) as u8;
            covered.insert(event);
            trace.push_str(&format!(" {}", EVENTS[event as usize]));
            match event {
                0 => fixture.lid(true),
                1 => fixture.lid(false),
                2 => fixture.toggle_connector("DP-1"),
                3 => fixture.toggle_connector("HDMI-1"),
                4 => fixture.toggle_connector("eDP-1"),
                5 | 6 => {
                    fixture.state.native.runtime.mirror_of = if event == 5 {
                        mirror_map(&[("DP-1", "eDP-1"), ("HDMI-1", "eDP-1")])
                    } else {
                        mirror_map(&[("HDMI-1", "DP-1")])
                    };
                    fixture.queue_policy();
                }
                7 => {
                    fixture.state.native.runtime.mirror_of = Default::default();
                    fixture.queue_policy();
                }
                8 => fixture.paused = true,
                9 => {
                    fixture.paused = false;
                    fixture.queue_policy();
                }
                10 => fixture.queue_policy(),
                12 => fixture.change_mode("DP-1"),
                13 => fixture.change_mode("eDP-1"),
                11 if !fixture.paused => {
                    fixture.flush_and_check(&trace);
                }
                _ => {}
            }
        }
        fixture.paused = false;
        fixture.queue_policy();
        fixture.flush_and_check(&trace);
        // Every walk must remain recoverable after all its chaotic events.
        if !fixture
            .state
            .native
            .output_management_state
            .outputs()
            .iter()
            .any(|output| output.name() == "eDP-1")
        {
            fixture.toggle_connector("eDP-1");
        }
        fixture.lid(false);
        assert!(fixture.flush().is_empty(), "{trace}");
        assert!(
            fixture
                .state
                .native
                .space
                .outputs()
                .any(|output| output.name() == "eDP-1"),
            "{trace}"
        );
    }
    assert_eq!(covered.len(), EVENTS.len());
}
