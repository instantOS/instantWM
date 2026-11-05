//! Replayable hostile-client tests. No display, D-Bus session or GPU is opened.
//!
//! The action format is shared by the process watchdog and coverage-guided
//! fuzzers. Keep resource limits here rather than trusting a fuzzer's input.

use serde::{Deserialize, Serialize};

pub const MAX_ACTIONS: usize = 512;
pub const CLIENTS: usize = 3;
pub const WINDOWS: usize = 6;
/// Connection tags handed to `Peer::connect`, stable across reconnects.
pub const PEER_TAGS: [&str; CLIENTS] = ["peer0", "peer1", "peer2"];

/// Logical handles deliberately survive deletion/reconnection: an operation
/// on an empty slot is a no-op, so minimized traces retain useful meaning.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub client: u8,
    pub window: u8,
    pub op: Op,
    /// False batches requests with subsequent actions before server dispatch.
    pub dispatch: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Op {
    Connect,
    Disconnect,
    Create,
    Destroy,
    Commit,
    Unmap,
    Fullscreen(bool),
    Maximize(bool),
    Minimize,
    Parent(Option<u8>),
    Title(u16),
    Geometry(u16, u16),
    Constraints(u16, u16),
    FrameBurst(u8),
    HoldAcks(bool),
    StopReading(bool),
    Focus,
    CloseBurst(u8),
    InvalidAck,
    InvalidGeometry,
    DuplicateRole,
    /// A fresh raw connection sends a malformed core protocol request.
    MalformedWire(u8),
    Pump,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub actions: Vec<Action>,
}

impl Scenario {
    /// Stable, bounded byte-to-action mapping shared by libFuzzer and replay.
    pub fn from_bytes(data: &[u8]) -> Self {
        let actions = data
            .chunks_exact(6)
            .take(MAX_ACTIONS)
            .map(|b| {
                let n = u16::from_le_bytes([b[3], b[4]]);
                let op = match b[0] % 23 {
                    0 => Op::Connect,
                    1 => Op::Disconnect,
                    2 => Op::Create,
                    3 => Op::Destroy,
                    4 => Op::Commit,
                    5 => Op::Unmap,
                    6 => Op::Fullscreen(b[3] & 1 != 0),
                    7 => Op::Maximize(b[3] & 1 != 0),
                    8 => Op::Minimize,
                    9 => Op::Parent((b[4] & 1 != 0).then_some(b[3])),
                    10 => Op::Title(n),
                    11 => Op::Geometry(n, u16::from(b[4]) + 1),
                    12 => Op::Constraints(n, u16::from(b[4]) + 1),
                    13 => Op::FrameBurst(b[3]),
                    14 => Op::HoldAcks(b[3] & 1 != 0),
                    15 => Op::StopReading(b[3] & 1 != 0),
                    16 => Op::Focus,
                    17 => Op::CloseBurst(b[3]),
                    18 => Op::InvalidAck,
                    19 => Op::InvalidGeometry,
                    20 => Op::DuplicateRole,
                    21 => Op::MalformedWire(b[3]),
                    _ => Op::Pump,
                };
                Action {
                    client: b[1],
                    window: b[2],
                    op,
                    dispatch: b[5] & 3 != 0,
                }
            })
            .collect();
        Self { actions }
    }

    /// A portable PRNG makes seeds stable across Rust versions/platforms.
    pub fn seeded(seed: u64, steps: usize) -> Self {
        let mut state = seed;
        let mut bytes = Vec::with_capacity(steps.min(MAX_ACTIONS) * 6);
        for _ in 0..steps.min(MAX_ACTIONS) * 6 {
            state = state.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            bytes.push((z ^ (z >> 31)) as u8);
        }
        Self::from_bytes(&bytes)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub actions: usize,
    pub mapped_windows: usize,
    pub expected_disconnects: usize,
    pub witness_roundtrips: usize,
    pub max_roundtrip_micros: u128,
}

/// Executes actual client/server protocols and production policy dispatch.
/// Panics on a violated invariant. Run in a watchdog child for hang detection.
pub fn run(scenario: &Scenario) -> Report {
    assert!(
        scenario.actions.len() <= MAX_ACTIONS,
        "scenario exceeds action limit"
    );
    crate::backend::wayland::compositor::fuzzing::run(scenario)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_scenarios_are_bounded_and_replayable() {
        let scenario = Scenario::seeded(42, MAX_ACTIONS + 100);
        assert_eq!(scenario.actions.len(), MAX_ACTIONS);
        assert_eq!(scenario, Scenario::seeded(42, MAX_ACTIONS));
        let text = serde_json::to_string(&scenario).unwrap();
        assert_eq!(serde_json::from_str::<Scenario>(&text).unwrap(), scenario);
    }

    #[test]
    fn protocol_lifecycle_smoke_corpus() {
        for seed in [0, 1, 42, 0x5a00_2026] {
            let report = run(&Scenario::seeded(seed, 48));
            assert_eq!(report.actions, 48);
            assert!(report.mapped_windows >= 1);
            assert!(report.witness_roundtrips >= 2);
        }
    }
}
