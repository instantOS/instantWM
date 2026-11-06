#![no_main]

use libfuzzer_sys::fuzz_target;

// Include fuzz/seeds/wayland_actions in the initial corpus to exercise the
// versioned decoder as well as historical unprefixed artifacts.
fuzz_target!(|bytes: &[u8]| {
    let scenario = instantwm::fuzzing::Scenario::from_bytes(bytes);
    instantwm::fuzzing::run(&scenario);
});
