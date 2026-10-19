//! `zwp_idle_inhibit_manager_v1` wiring, exercised by a real client.
//!
//! An inhibitor only has an effect through the idle notifier: it owns the
//! timers behind `ext_idle_notifier_v1`, so a surface that is tracked but never
//! reported leaves clients such as swayidle locking the session mid-playback.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use wayland_client::protocol::{wl_callback, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::wp::idle_inhibit::zv1::client::{
    zwp_idle_inhibit_manager_v1, zwp_idle_inhibitor_v1,
};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

#[derive(Default)]
struct ClientEvents {
    globals: Vec<(u32, String)>,
    syncs: usize,
}

impl Dispatch<wl_registry::WlRegistry, ()> for ClientEvents {
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

impl Dispatch<wl_callback::WlCallback, ()> for ClientEvents {
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

delegate_noop!(ClientEvents: ignore wl_compositor::WlCompositor);
delegate_noop!(ClientEvents: ignore wl_surface::WlSurface);
delegate_noop!(ClientEvents: ignore zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1);
delegate_noop!(ClientEvents: ignore zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1);

fn roundtrip(
    event_loop: &mut EventLoop,
    state: &mut WaylandState,
    conn: &Connection,
    queue: &mut EventQueue<ClientEvents>,
    events: &mut ClientEvents,
) {
    // A sync request guarantees that every preceding client request has been
    // handled before the client queue returns from dispatch.
    let target = events.syncs + 1;
    conn.display().sync(&queue.handle(), ());
    conn.flush().unwrap();
    event_loop
        .dispatch(Some(Duration::from_millis(250)), state)
        .unwrap();
    state.dispatch_pending_commits();
    state.native.display_handle.flush_clients().unwrap();
    while events.syncs < target {
        queue.blocking_dispatch(events).unwrap();
    }
}

fn global(events: &ClientEvents, interface: &str) -> u32 {
    events
        .globals
        .iter()
        .find(|(_, name)| name == interface)
        .unwrap_or_else(|| panic!("missing {interface} global"))
        .0
}

/// The client end of a connected test client, kept together so that dropping it
/// wholesale is the same as the client dying.
struct Client {
    conn: Connection,
    queue: EventQueue<ClientEvents>,
    events: ClientEvents,
    compositor: wl_compositor::WlCompositor,
    manager: zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
}

impl Client {
    fn connect(event_loop: &mut EventLoop, state: &mut WaylandState) -> Self {
        let (client_socket, server_socket) = UnixStream::pair().unwrap();
        state
            .native
            .display_handle
            .insert_client(server_socket, Arc::new(WaylandClientState::default()))
            .unwrap();
        let conn = Connection::from_socket(client_socket).unwrap();
        let mut queue = conn.new_event_queue();
        let registry = conn.display().get_registry(&queue.handle(), ());
        let mut events = ClientEvents::default();
        roundtrip(event_loop, state, &conn, &mut queue, &mut events);

        let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
            global(&events, "wl_compositor"),
            4,
            &queue.handle(),
            (),
        );
        let manager = registry.bind::<zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1, _, _>(
            global(&events, "zwp_idle_inhibit_manager_v1"),
            1,
            &queue.handle(),
            (),
        );
        roundtrip(event_loop, state, &conn, &mut queue, &mut events);

        Self {
            conn,
            queue,
            events,
            compositor,
            manager,
        }
    }

    fn roundtrip(&mut self, event_loop: &mut EventLoop, state: &mut WaylandState) {
        roundtrip(
            event_loop,
            state,
            &self.conn,
            &mut self.queue,
            &mut self.events,
        );
    }

    fn create_surface(&self) -> wl_surface::WlSurface {
        self.compositor.create_surface(&self.queue.handle(), ())
    }

    fn create_inhibitor(
        &self,
        surface: &wl_surface::WlSurface,
    ) -> zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1 {
        self.manager
            .create_inhibitor(surface, &self.queue.handle(), ())
    }
}

fn idle_is_inhibited(state: &mut WaylandState) -> bool {
    state.native.idle_notify_manager_state.is_inhibited()
}

#[test]
fn inhibitors_drive_the_idle_notifier_until_the_last_one_is_gone() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let mut client = Client::connect(&mut event_loop, &mut state);
    assert!(!idle_is_inhibited(&mut state));

    let first_surface = client.create_surface();
    let first = client.create_inhibitor(&first_surface);
    client.roundtrip(&mut event_loop, &mut state);
    assert!(
        idle_is_inhibited(&mut state),
        "a live inhibitor has to suppress idle"
    );

    // The session stays inhibited while any surface still holds an inhibitor.
    let second_surface = client.create_surface();
    let second = client.create_inhibitor(&second_surface);
    client.roundtrip(&mut event_loop, &mut state);
    first.destroy();
    client.roundtrip(&mut event_loop, &mut state);
    assert!(idle_is_inhibited(&mut state));

    second.destroy();
    client.roundtrip(&mut event_loop, &mut state);
    assert!(
        !idle_is_inhibited(&mut state),
        "idle has to resume once nothing inhibits it"
    );
}

/// Clients may die without destroying their inhibitor, which the protocol
/// allows, so a dead surface has to stop inhibiting through the purge.
#[test]
fn a_crashed_client_stops_inhibiting_idle() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let mut client = Client::connect(&mut event_loop, &mut state);

    let surface = client.create_surface();
    let inhibitor = client.create_inhibitor(&surface);
    client.roundtrip(&mut event_loop, &mut state);
    assert!(idle_is_inhibited(&mut state));

    drop(inhibitor);
    drop(surface);
    drop(client);

    let mut released = false;
    for _ in 0..50 {
        state.native.display_handle.flush_clients().unwrap();
        let _ = event_loop.dispatch(Some(Duration::from_millis(50)), &mut state);
        state.purge_idle_inhibitors();
        if !idle_is_inhibited(&mut state) {
            released = true;
            break;
        }
    }
    assert!(
        released,
        "a crashed client must not keep the session inhibited forever"
    );
}
