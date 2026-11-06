use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use smithay::reexports::wayland_server::backend::protocol::{
    AllowNull, ArgumentType, MessageDesc, ObjectInfo,
};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};

/// Keep malformed transport isolated from the typed clients. These complete
/// messages must be rejected; partial frames alone must not block other peers.
pub(super) struct RawPeer {
    peer: UnixStream,
    client: smithay::reexports::wayland_server::Client,
    variant: u8,
}

pub(super) fn malformed(state: &mut WaylandState, variant: u8) -> RawPeer {
    let (mut peer, server) = UnixStream::pair().unwrap();
    let client = state
        .native
        .display_handle
        .insert_client(server, Arc::new(WaylandClientState::default()))
        .unwrap();
    let words = match variant % 8 {
        0 => vec![1_u32, (8 << 16) | 0xffff], // unknown wl_display opcode
        1 => vec![0x7fff_ffff, 8 << 16],      // nonexistent receiver
        2 => vec![1, 12 << 16, 1],            // new_id collides with wl_display
        3 => vec![1, 12 << 16, 0],            // null new_id
        4 => vec![1, 12 << 16],               // partial valid sync frame
        // A sync header claims a 64 KiB body, but the fixed 4096-byte input
        // buffer rejects the header immediately; the body never arrives.
        6 => vec![1_u32, 65532 << 16, 3],
        // A real get_registry followed by a complete frame with an opcode
        // wl_registry does not have: signature validation must reject it.
        7 => vec![1, (12 << 16) | 1, 2, 2, (12 << 16) | 2, 0],
        // Bulk deterministic garbage (variant % 8 == 5). Bounded to the
        // server's 4096-byte input buffer so the blocking write cannot
        // deadlock the single-threaded harness before the server reads.
        _ => {
            let mut rng = u64::from(variant) | 1;
            let mut next = move || {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                rng
            };
            let len = 1 + next() % 1024;
            (0..len).map(|_| next() as u32).collect()
        }
    };
    let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_ne_bytes).collect();
    peer.write_all(&bytes).unwrap();
    peer.set_nonblocking(true).unwrap();
    RawPeer {
        peer,
        client,
        variant,
    }
}

/// Begin registry discovery. Every stage ends with a sync on this connection;
/// a heartbeat on another client alone does not prove these requests ran.
pub(super) fn raw_start(state: &mut WaylandState) -> RawPeer {
    let (peer, server) = UnixStream::pair().unwrap();
    let client = state
        .native
        .display_handle
        .insert_client(server, Arc::new(WaylandClientState::default()))
        .unwrap();
    peer.set_nonblocking(true).unwrap();
    let mut raw = RawPeer {
        peer,
        client,
        variant: 0,
    };
    raw.write_request(1, 1, &words(&[2]));
    raw.write_request(1, 0, &words(&[3]));
    raw
}

/// Bind an advertised global. Discovery and binding are setup, so failures
/// here are harness failures rather than acceptable fuzzed protocol errors.
pub(super) fn raw_bind(peer: &mut RawPeer, selector: u8) -> String {
    let bytes = peer.read_available();
    assert!(
        has_sync(&bytes, 3),
        "raw registry discovery did not complete"
    );
    let globals = parse_registry(&bytes);
    assert!(!globals.is_empty(), "raw registry contained no globals");
    let global = &globals[selector as usize % globals.len()];
    let mut body = words(&[global.name]);
    put_string(&mut body, &global.interface);
    body.extend(words(&[global.version, 4]));
    peer.write_request(2, 0, &body);
    peer.write_request(1, 0, &words(&[5]));
    global.interface.clone()
}

pub(super) fn raw_bound(peer: &mut RawPeer, state: &WaylandState, interface: &str) {
    assert!(
        has_sync(&peer.read_available(), 5),
        "raw global bind did not complete: {interface}"
    );
    let objects = peer.objects(state);
    assert!(
        objects
            .iter()
            .any(|obj| obj.id == 4 && obj.interface.name == interface),
        "raw global bind did not create {interface}"
    );
}

pub(super) struct Request {
    pub marker: u32,
    pub structured: bool,
    pub child: bool,
}

/// Shape most requests from the actual bound object's protocol metadata.
/// Prefer constructors on even seeds and follow newly created objects on later
/// steps. FD-bearing requests remain in the arbitrary-input branch: inventing
/// a valid backing store/device would require protocol-specific setup.
pub(super) fn raw_request(
    peer: &mut RawPeer,
    state: &WaylandState,
    seed: u16,
    step: u32,
    next_id: &mut u32,
) -> Option<Request> {
    if !peer.connected(state) {
        return None;
    }
    let objects = peer.objects(state);
    let target = objects
        .iter()
        .filter(|obj| obj.id >= 4 && obj.id < 0xff00_0000)
        .max_by_key(|obj| obj.id)?;
    let structured = seed & 3 != 3;
    let mut body = Vec::new();
    let opcode = if structured {
        let mut requests: Vec<_> = target
            .interface
            .requests
            .iter()
            .enumerate()
            .filter(|(_, request)| request.since <= target.version && supported(request, &objects))
            .collect();
        if seed & 1 == 0
            && requests
                .iter()
                .any(|(_, req)| req.child_interface.is_some())
        {
            requests.retain(|(_, req)| req.child_interface.is_some());
        }
        if requests.is_empty() {
            return None;
        }
        let (opcode, request) = requests[(usize::from(seed >> 2) + step as usize) % requests.len()];
        let mut object_arg = 0;
        for arg in request.signature {
            let value = match arg {
                ArgumentType::NewId => {
                    let id = *next_id;
                    *next_id += 1;
                    id
                }
                ArgumentType::Object(nullable) => {
                    let interface = request.arg_interfaces[object_arg];
                    object_arg += 1;
                    if *nullable == AllowNull::Yes && seed & 0x10 != 0 {
                        0
                    } else {
                        objects
                            .iter()
                            .find(|obj| obj.interface.name == interface.name)
                            .map_or(0, |obj| obj.id)
                    }
                }
                ArgumentType::Str(_) => {
                    put_string(&mut body, "raw-fuzz");
                    continue;
                }
                ArgumentType::Array => {
                    body.extend(words(&[0]));
                    continue;
                }
                ArgumentType::Int | ArgumentType::Uint | ArgumentType::Fixed => {
                    [0, 1, 256, u32::MAX, 0x8000_0000, u32::from(seed)]
                        [(usize::from(seed >> 4) + step as usize) % 6]
                }
                ArgumentType::Fd => unreachable!(),
            };
            body.extend(words(&[value]));
        }
        opcode as u16
    } else {
        let mut rng = u64::from(seed) | 1;
        for _ in 0..usize::from(seed & 0x3f) {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            body.extend(words(&[rng as u32]));
        }
        seed >> 8
    };
    // The server cannot process/disconnect this client until the caller pumps;
    // both bounded writes must succeed, including for a malformed request.
    peer.write_request(target.id, opcode, &body);
    let marker = *next_id;
    *next_id += 1;
    peer.write_request(1, 0, &words(&[marker]));
    Some(Request {
        marker,
        structured,
        child: target.id != 4,
    })
}

fn supported(request: &MessageDesc, objects: &[ObjectInfo]) -> bool {
    let mut object_arg = 0;
    request.signature.iter().all(|arg| match arg {
        ArgumentType::Fd => false,
        ArgumentType::NewId => request.child_interface.is_some(),
        ArgumentType::Object(nullable) => {
            let interface = request.arg_interfaces[object_arg];
            object_arg += 1;
            *nullable == AllowNull::Yes
                || objects
                    .iter()
                    .any(|obj| obj.interface.name == interface.name)
        }
        _ => true,
    })
}

/// A rejected request may disconnect. Otherwise the trailing sync must arrive,
/// proving the server consumed the request rather than buffering it forever.
pub(super) fn raw_finish(peer: &mut RawPeer, state: &WaylandState, marker: u32) {
    let bytes = peer.read_available();
    assert!(
        !peer.connected(state) || has_sync(&bytes, marker),
        "raw request was neither processed nor rejected"
    );
}

fn has_sync(bytes: &[u8], marker: u32) -> bool {
    messages(bytes).any(|(object, opcode, body)| object == marker && opcode == 0 && body.len() == 4)
}

fn messages(bytes: &[u8]) -> impl Iterator<Item = (u32, u16, &[u8])> {
    let mut remaining = bytes;
    std::iter::from_fn(move || {
        if remaining.is_empty() {
            return None;
        }
        assert!(remaining.len() >= 8, "truncated server message header");
        let object = read_word(remaining, 0);
        let header = read_word(remaining, 4);
        let size = (header >> 16) as usize;
        assert!(
            size >= 8 && size % 4 == 0 && size <= remaining.len(),
            "invalid server message length"
        );
        let body = &remaining[8..size];
        remaining = &remaining[size..];
        Some((object, header as u16, body))
    })
}

struct Global {
    name: u32,
    interface: String,
    version: u32,
}

fn parse_registry(bytes: &[u8]) -> Vec<Global> {
    messages(bytes)
        .filter_map(|(object, opcode, body)| {
            if object != 2 || opcode != 0 {
                return None;
            }
            assert!(body.len() >= 4, "missing global name");
            let (interface, next) =
                read_string(body, 4, body.len()).expect("invalid registry interface string");
            assert_eq!(next + 4, body.len(), "invalid registry global arguments");
            Some(Global {
                name: read_word(body, 0),
                interface,
                version: read_word(body, next),
            })
        })
        .collect()
}

fn read_word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_string(bytes: &[u8], offset: usize, end: usize) -> Option<(String, usize)> {
    if offset + 4 > end {
        return None;
    }
    let length = read_word(bytes, offset) as usize;
    let start = offset + 4;
    let padded = length.checked_add(3)? & !3;
    let next = start.checked_add(padded)?;
    if length == 0 || next > end {
        return None;
    }
    let raw = &bytes[start..start + length];
    let data = raw.strip_suffix(&[0])?;
    Some((std::str::from_utf8(data).ok()?.to_owned(), next))
}

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

fn put_string(body: &mut Vec<u8>, value: &str) {
    let length = value.len() + 1;
    let padded = (length + 3) & !3;
    body.extend(words(&[length as u32]));
    body.extend_from_slice(value.as_bytes());
    body.push(0);
    body.resize(body.len() + (padded - length), 0);
}

impl RawPeer {
    fn connected(&self, state: &WaylandState) -> bool {
        self.client
            .get_credentials(&state.native.display_handle)
            .is_ok()
    }

    fn objects(&self, state: &WaylandState) -> Vec<ObjectInfo> {
        let handle = state.native.display_handle.backend_handle();
        let mut ids = Vec::new();
        // Do not call back into the backend while with_all_objects_for holds its lock.
        handle
            .with_all_objects_for(self.client.id(), |id| ids.push(id))
            .unwrap();
        let mut objects: Vec<_> = ids
            .into_iter()
            .filter_map(|id| handle.object_info(id).ok())
            .collect();
        objects.sort_by_key(|obj| obj.id);
        objects
    }

    fn write_request(&mut self, object: u32, opcode: u16, body: &[u8]) {
        let mut message = Vec::with_capacity(8 + body.len());
        message.extend(words(&[object]));
        message.extend(words(
            &[((8 + body.len()) as u32) << 16 | u32::from(opcode)],
        ));
        message.extend_from_slice(body);
        self.peer
            .write_all(&message)
            .expect("raw request write failed");
    }

    fn read_available(&mut self) -> Vec<u8> {
        let mut output = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            match self.peer.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => output.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
                Err(error) => panic!("raw response read failed: {error}"),
            }
        }
        output
    }
}

pub(super) fn check(peer: &mut RawPeer, state: &WaylandState) {
    let mut bytes = [0; 4096];
    while matches!(peer.peer.read(&mut bytes), Ok(n) if n > 0) {}
    let connected = peer
        .client
        .get_credentials(&state.native.display_handle)
        .is_ok();
    match peer.variant % 8 {
        // A small incomplete frame is buffered, never parsed, never punished.
        4 => assert!(connected, "partial frame prematurely disconnected"),
        // A header declaring more than the 4096-byte input buffer is rejected
        // immediately, even though its body never arrives.
        6 => assert!(!connected, "oversized frame header was accepted"),
        // Pure garbage ends in whichever state its random first-frame size
        // dictates: usually buffered-unparsed forever, occasionally parsed
        // and rejected. Both are correct server answers.
        5 => {}
        _ => assert!(!connected, "crafted malformed request was accepted"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::Harness;
    use super::*;
    use crate::fuzzing::{Action, Op};

    fn event(object: u32, opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut bytes = words(&[object, ((body.len() as u32 + 8) << 16) | u32::from(opcode)]);
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn registry_handles_padding_and_interleaved_events() {
        let mut bytes = event(3, 0, &words(&[123]));
        for (name, interface, version) in [(10, "wl_shm", 2), (20, "wl_compositor", 6)] {
            let mut body = words(&[name]);
            put_string(&mut body, interface);
            body.extend(words(&[version]));
            bytes.extend(event(2, 0, &body));
        }
        bytes.extend(event(1, 1, &words(&[3])));
        let globals = parse_registry(&bytes);
        assert_eq!(globals.len(), 2);
        assert_eq!(
            (
                globals[1].name,
                globals[1].interface.as_str(),
                globals[1].version
            ),
            (20, "wl_compositor", 6)
        );
        assert!(has_sync(&bytes, 3));
        assert!(!has_sync(&bytes, 5));
        assert!(read_string(&words(&[u32::MAX]), 0, 4).is_none());
        assert!(read_string(&words(&[1, 65]), 0, 8).is_none());
    }

    #[test]
    #[should_panic(expected = "invalid server message length")]
    fn truncated_server_frame_is_not_silently_ignored() {
        parse_registry(&words(&[2, 32 << 16, 1]));
    }

    #[test]
    #[should_panic(expected = "raw registry discovery did not complete")]
    fn missing_discovery_is_a_harness_failure() {
        let mut harness = Harness::new();
        let mut peer = raw_start(&mut harness.state);
        raw_bind(&mut peer, 0);
    }

    #[test]
    fn constructors_reach_child_handlers() {
        let mut harness = Harness::new();
        for interface in ["wl_compositor", "xdg_wm_base"] {
            let selector = harness
                .witness
                .events
                .globals
                .iter()
                .position(|(_, name)| name == interface)
                .unwrap() as u8;
            let before = harness.report.raw_child_requests;
            harness.step(&Action {
                client: 0,
                window: 0,
                op: Op::RawGlobal(selector, 0),
                dispatch: true,
            });
            assert!(
                harness.report.raw_child_requests > before,
                "no child request for {interface}"
            );
        }
        assert_eq!(harness.report.raw_global_binds, 2);
        assert!(harness.report.raw_requests >= 4);
        assert_eq!(
            harness.report.raw_structured_requests,
            harness.report.raw_requests
        );
        harness.finish();
    }

    #[test]
    fn valid_constructor_and_child_destructor_complete_on_raw_connection() {
        let mut harness = Harness::new();
        let selector = harness
            .witness
            .events
            .globals
            .iter()
            .position(|(_, name)| name == "wl_compositor")
            .unwrap() as u8;
        let mut peer = raw_start(&mut harness.state);
        harness.heartbeat();
        let interface = raw_bind(&mut peer, selector);
        harness.heartbeat();
        raw_bound(&mut peer, &harness.state, &interface);
        let mut next_id = 6;
        let request = raw_request(&mut peer, &harness.state, 0, 0, &mut next_id).unwrap();
        harness.heartbeat();
        raw_finish(&mut peer, &harness.state, request.marker);
        assert!(
            peer.objects(&harness.state)
                .iter()
                .any(|obj| obj.id == 6 && obj.interface.name == "wl_surface")
        );
        let request = raw_request(&mut peer, &harness.state, 1, 0, &mut next_id).unwrap();
        assert!(request.child);
        harness.heartbeat();
        raw_finish(&mut peer, &harness.state, request.marker);
        assert!(peer.connected(&harness.state));
        assert!(!peer.objects(&harness.state).iter().any(|obj| obj.id == 6));
        drop(peer);
        harness.finish();
    }

    #[test]
    fn every_advertised_global_binds_and_stays_contained() {
        let mut harness = Harness::new();
        let count = harness.witness.events.globals.len();
        assert!(count <= 256);
        for selector in 0..count {
            for seed in [0, 1, 0x1234, u16::MAX] {
                harness.step(&Action {
                    client: 0,
                    window: 0,
                    op: Op::RawGlobal(selector as u8, seed),
                    dispatch: true,
                });
            }
        }
        assert_eq!(harness.report.raw_global_binds, count * 4);
        assert!(harness.report.raw_requests > 0);
        assert!(harness.report.raw_structured_requests > 0);
        assert!(harness.report.raw_child_requests > 0);
        harness.finish();
    }
}
