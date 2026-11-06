//! Exercise capture completion over the wire without needing a GPU or portal.
use std::{os::fd::AsFd, os::unix::net::UnixStream, sync::Arc, time::Duration};

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_output, wl_registry, wl_shm, wl_shm_pool,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1,
        ext_image_copy_capture_session_v1,
    },
};

use super::super::{WaylandClientState, WaylandState};

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

#[derive(Default)]
struct Events {
    globals: Vec<(u32, String)>,
    syncs: usize,
    damage: Vec<(i32, i32, i32, i32)>,
    ready: usize,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Events {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            state.globals.push((name, interface));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Events {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.syncs += 1;
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for Events {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Damage {
                x,
                y,
                width,
                height,
            } => state.damage.push((x, y, width, height)),
            Event::Ready => {
                assert_eq!(state.damage, [(0, 0, 64, 128)], "damage must precede ready");
                state.ready += 1;
            }
            Event::Failed { reason } => panic!("capture failed: {reason:?}"),
            _ => {}
        }
    }
}

delegate_noop!(Events: ignore wl_output::WlOutput);
delegate_noop!(Events: ignore wl_shm::WlShm);
delegate_noop!(Events: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Events: ignore wl_buffer::WlBuffer);
delegate_noop!(Events: ignore ext_image_capture_source_v1::ExtImageCaptureSourceV1);
delegate_noop!(Events: ignore ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Events: ignore ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);
delegate_noop!(Events: ignore ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1);

fn pump(
    event_loop: &mut EventLoop,
    state: &mut WaylandState,
    conn: &Connection,
    queue: &mut EventQueue<Events>,
    events: &mut Events,
) {
    let target = events.syncs + 1;
    conn.display().sync(&queue.handle(), ());
    conn.flush().unwrap();
    event_loop
        .dispatch(Some(Duration::from_millis(250)), state)
        .unwrap();
    state.native.display_handle.flush_clients().unwrap();
    while events.syncs < target {
        queue.blocking_dispatch(events).unwrap();
    }
}

#[test]
fn repeated_full_captures_report_damage_before_ready() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    state
        .native
        .create_output("capture-test", crate::types::Size::new(64, 128), None);
    let (client_socket, server_socket) = UnixStream::pair().unwrap();
    state
        .native
        .display_handle
        .insert_client(server_socket, Arc::new(WaylandClientState::default()))
        .unwrap();
    let conn = Connection::from_socket(client_socket).unwrap();
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let registry = conn.display().get_registry(&qh, ());
    let mut events = Events::default();
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut events);
    let global = |name: &str| {
        events
            .globals
            .iter()
            .find(|(_, interface)| interface == name)
            .unwrap()
            .0
    };
    let output = registry.bind::<wl_output::WlOutput, _, _>(global("wl_output"), 1, &qh, ());
    let shm = registry.bind::<wl_shm::WlShm, _, _>(global("wl_shm"), 1, &qh, ());
    let sources = registry.bind::<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1, _, _>(
        global("ext_output_image_capture_source_manager_v1"), 1, &qh, ());
    let manager = registry
        .bind::<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1, _, _>(
            global("ext_image_copy_capture_manager_v1"),
            1,
            &qh,
            (),
        );
    let source = sources.create_source(&output, &qh, ());
    let session = manager.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        &qh,
        (),
    );
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut events);

    let file = tempfile::tempfile().unwrap();
    file.set_len(64 * 128 * 4).unwrap();
    let pool = shm.create_pool(file.as_fd(), 64 * 128 * 4, &qh, ());
    let buffer = pool.create_buffer(0, 64, 128, 64 * 4, wl_shm::Format::Xrgb8888, &qh, ());
    for expected in 1..=3 {
        events.damage.clear();
        let frame = session.create_frame(&qh, ());
        frame.attach_buffer(&buffer);
        if expected == 1 {
            frame.damage_buffer(0, 0, 64, 128);
        }
        frame.capture();
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut events);
        assert_eq!(state.native.runtime.pending_image_captures.len(), 1);
        // Simulate a completed full-buffer copy. Both the offscreen/SHM and
        // direct DRM dmabuf paths use this completion method.
        state
            .native
            .runtime
            .pending_image_captures
            .pop()
            .unwrap()
            .success();
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut events);
        assert_eq!(events.ready, expected);
        frame.destroy();
    }
}
