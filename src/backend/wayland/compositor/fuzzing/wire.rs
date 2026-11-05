use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};

/// Keep malformed transport isolated from the typed clients. These complete
/// messages must be rejected; partial frames alone must not block other peers.
pub(super) struct RawPeer {
    peer: UnixStream,
    client: smithay::reexports::wayland_server::Client,
    partial: bool,
}

pub(super) fn malformed(state: &mut WaylandState, variant: u8) -> RawPeer {
    let (mut peer, server) = UnixStream::pair().unwrap();
    let client = state
        .native
        .display_handle
        .insert_client(server, Arc::new(WaylandClientState::default()))
        .unwrap();
    let words = match variant % 5 {
        0 => vec![1_u32, (8 << 16) | 0xffff], // unknown wl_display opcode
        1 => vec![0x7fff_ffff, 8 << 16],      // nonexistent receiver
        2 => vec![1, 12 << 16, 1],            // new_id collides with wl_display
        3 => vec![1, 12 << 16, 0],            // null new_id
        _ => vec![1, 12 << 16],               // partial valid sync frame
    };
    let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_ne_bytes).collect();
    peer.write_all(&bytes).unwrap();
    peer.set_nonblocking(true).unwrap();
    RawPeer { peer, client, partial: variant % 5 == 4 }
}

pub(super) fn check(peer: &mut RawPeer, state: &WaylandState) {
    let mut bytes = [0; 4096];
    while matches!(peer.peer.read(&mut bytes), Ok(n) if n > 0) {}
    assert_eq!(peer.client.get_credentials(&state.native.display_handle).is_ok(), peer.partial,
        "malformed request accepted or partial frame prematurely disconnected");
}
