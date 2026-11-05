use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

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
