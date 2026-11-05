//! Exercise libwayland backpressure with a client that temporarily stops reading.

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn slow_client_survives_an_event_burst_larger_than_four_kib() {
    exercise_event_burst(None, true);
}

#[test]
fn four_kib_limit_reproduces_slow_client_disconnection() {
    exercise_event_burst(Some(4096), false);
}

fn exercise_event_burst(buffer_limit: Option<usize>, survives: bool) {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    if !super::buffer::set_default_max_buffer_size(
        &state.native.display_handle,
        buffer_limit.unwrap_or(1024 * 1024),
    ) {
        eprintln!("skipping queue-size regression test: requires libwayland >= 1.23");
        return;
    }
    let (mut peer, server) = UnixStream::pair().unwrap();
    // Force backpressure without depending on the host's default socket size.
    let size: libc::c_int = 4096;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                server.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&size as *const libc::c_int).cast(),
                std::mem::size_of_val(&size) as libc::socklen_t,
            )
        },
        0
    );
    let client = state
        .native
        .display_handle
        .insert_client(server, Arc::new(super::WaylandClientState::default()))
        .unwrap();

    // wl_display.sync requests are handled by libwayland itself. Each creates
    // a callback.done and display.delete_id event. Queue all requests before
    // reading any response, as a stalled toolkit event loop would do.
    const COUNT: u32 = 2000;
    let mut requests = Vec::new();
    for id in 2..COUNT + 2 {
        for word in [1_u32, 12_u32 << 16, id] {
            requests.extend_from_slice(&word.to_ne_bytes());
        }
    }
    peer.write_all(&requests).unwrap();
    for _ in 0..32 {
        event_loop
            .dispatch(Some(Duration::ZERO), &mut state)
            .unwrap();
        state.native.display_handle.flush_clients().unwrap();
    }
    assert_eq!(
        client.get_credentials(&state.native.display_handle).is_ok(),
        survives
    );
    if !survives {
        return;
    }

    // Recover and verify every reply actually arrives, rather than merely
    // checking that a client with unprocessed requests is still connected.
    peer.set_nonblocking(true).unwrap();
    let mut replies = Vec::new();
    let mut chunk = [0_u8; 8192];
    for _ in 0..128 {
        loop {
            match peer.read(&mut chunk) {
                Ok(0) => panic!("server disconnected the slow client"),
                Ok(n) => replies.extend_from_slice(&chunk[..n]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("read replies: {error}"),
            }
        }
        state.native.display_handle.flush_clients().unwrap();
        if replies.len() == COUNT as usize * 24 {
            break;
        }
    }
    assert_eq!(replies.len(), COUNT as usize * 24);
}
