#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let scenario = instantwm::fuzzing::Scenario::from_bytes(bytes);
    instantwm::fuzzing::run(&scenario);
});
