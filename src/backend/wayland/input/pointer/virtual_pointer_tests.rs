//! `zwlr_virtual_pointer_v1` routing, exercised by a real client.
//!
//! Virtual pointer events carry the same payloads as physical ones and take
//! the shared command path, so these tests assert on what the handlers queue
//! for the WM and on the coordinate space absolute motion is mapped into.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use smithay::backend::input::{AxisSource, ButtonState};
use smithay::output::{Mode as OutputMode, PhysicalProperties, Subpixel};
use wayland_client::protocol::{wl_callback, wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use crate::backend::output::OutputConnectorKind;
use crate::backend::wayland::commands::{PointerMotionCommand, WmCommand};
use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::types::Point;

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

/// BTN_LEFT, the button a regular mouse reports for a primary click.
const BTN_LEFT: u32 = 0x110;

#[derive(Default)]
struct ClientEvents {
    globals: Vec<(u32, String)>,
    syncs: usize,
    outputs: Vec<(u32, wl_output::WlOutput)>,
    output_names: HashMap<u32, String>,
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

/// The registry name doubles as user data, so the name an output reports can
/// be traced back to the proxy the test holds.
impl Dispatch<wl_output::WlOutput, u32> for ClientEvents {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        registry_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.output_names.insert(*registry_name, name);
        }
    }
}

delegate_noop!(ClientEvents: ignore wl_seat::WlSeat);
delegate_noop!(ClientEvents: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(ClientEvents: ignore ZwlrVirtualPointerV1);

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

struct Fixture {
    event_loop: EventLoop,
    state: WaylandState,
    conn: Connection,
    queue: EventQueue<ClientEvents>,
    events: ClientEvents,
    registry: wl_registry::WlRegistry,
    manager: ZwlrVirtualPointerManagerV1,
    seat: wl_seat::WlSeat,
}

impl Fixture {
    fn new() -> Self {
        let (mut event_loop, mut state) = crate::test_support::new_compositor();
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
        roundtrip(&mut event_loop, &mut state, &conn, &mut queue, &mut events);

        let manager = registry.bind::<ZwlrVirtualPointerManagerV1, _, _>(
            global(&events, "zwlr_virtual_pointer_manager_v1"),
            2,
            &queue.handle(),
            (),
        );
        let seat = registry.bind::<wl_seat::WlSeat, _, _>(
            global(&events, "wl_seat"),
            1,
            &queue.handle(),
            (),
        );
        roundtrip(&mut event_loop, &mut state, &conn, &mut queue, &mut events);

        Self {
            event_loop,
            state,
            conn,
            queue,
            events,
            registry,
            manager,
            seat,
        }
    }

    fn roundtrip(&mut self) {
        roundtrip(
            &mut self.event_loop,
            &mut self.state,
            &self.conn,
            &mut self.queue,
            &mut self.events,
        );
    }

    /// Create a virtual pointer for the test seat.
    fn create_pointer(&self) -> ZwlrVirtualPointerV1 {
        self.manager
            .create_virtual_pointer(Some(&self.seat), &self.queue.handle(), ())
    }

    /// Add an output with a mode, map it into the space and bind its global, so
    /// the client can point a virtual pointer at it and read its name back.
    fn add_output(&mut self, name: &str, size: (i32, i32), location: (i32, i32)) {
        let output = self.state.native.create_output_global(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "instantOS".into(),
                model: "instantWM".into(),
                serial_number: "unknown".into(),
            },
            OutputMode {
                size: size.into(),
                refresh: 60_000,
            },
            Point::new(location.0, location.1),
            OutputConnectorKind::External,
        );
        self.state.native.space.map_output(&output, location);
        self.roundtrip();

        let unbound: Vec<u32> = self
            .events
            .globals
            .iter()
            .filter(|(name, interface)| {
                interface == "wl_output"
                    && !self.events.outputs.iter().any(|(bound, _)| bound == name)
            })
            .map(|(name, _)| *name)
            .collect();
        for registry_name in unbound {
            let proxy = self.registry.bind::<wl_output::WlOutput, u32, _>(
                registry_name,
                4,
                &self.queue.handle(),
                registry_name,
            );
            self.events.outputs.push((registry_name, proxy));
        }
        self.roundtrip();
        assert!(
            self.output(name).is_some(),
            "bound output {name} has to report its name"
        );
    }

    /// The client-side proxy of the output reporting this name.
    fn output(&self, name: &str) -> Option<wl_output::WlOutput> {
        self.events
            .outputs
            .iter()
            .find(|(registry_name, _)| {
                self.events
                    .output_names
                    .get(registry_name)
                    .is_some_and(|reported| reported == name)
            })
            .map(|(_, proxy)| proxy.clone())
    }

    /// Take everything the handlers queued for the WM.
    fn drain_commands(&mut self) -> Vec<WmCommand> {
        std::mem::take(&mut *self.state.native.command_queue.borrow_mut())
    }
}

#[test]
fn virtual_pointer_events_take_the_shared_command_path() {
    let mut fixture = Fixture::new();
    let pointer = fixture.create_pointer();
    fixture.roundtrip();
    fixture.drain_commands();

    pointer.motion(100, 10.0, -5.0);
    pointer.button(101, BTN_LEFT, wl_pointer::ButtonState::Pressed);
    pointer.axis_source(wl_pointer::AxisSource::Wheel);
    pointer.axis(102, wl_pointer::Axis::VerticalScroll, 15.0);
    pointer.axis_discrete(102, wl_pointer::Axis::VerticalScroll, 15.0, 3);
    pointer.frame();
    fixture.roundtrip();

    let commands = fixture.drain_commands();
    assert_eq!(commands.len(), 3, "motion, button and one axis frame");
    assert!(
        matches!(
            &commands[0],
            WmCommand::PointerMotion(PointerMotionCommand::Relative { dx, dy, .. })
                if *dx == 10.0 && *dy == -5.0
        ),
        "relative motion is passed on unchanged: {commands:?}"
    );
    assert!(
        matches!(
            &commands[1],
            WmCommand::PointerButton(command)
                if command.code == BTN_LEFT && command.state == ButtonState::Pressed
        ),
        "virtual buttons keep their linux code and state: {commands:?}"
    );
    assert!(
        matches!(
            &commands[2],
            WmCommand::PointerAxis(command)
                if command.source == AxisSource::Wheel
                    && command.vertical.amount == Some(15.0)
                    && command.vertical.v120 == Some(360.0)
        ),
        "scroll accumulates into one frame with discrete steps: {commands:?}"
    );
}

/// Absolute positions arrive normalized against the client's own extents, so
/// they have to be mapped into the space: the whole layout by default, and the
/// requested output when the client named one.
#[test]
fn absolute_motion_is_mapped_into_the_requested_region() {
    let mut fixture = Fixture::new();
    fixture.add_output("test-A", (800, 600), (-300, -100));
    fixture.add_output("test-B", (400, 300), (500, 0));
    let target = fixture.output("test-B").expect("test-B is bound");

    let plain = fixture.create_pointer();
    let targeted = fixture.manager.create_virtual_pointer_with_output(
        Some(&fixture.seat),
        Some(&target),
        &fixture.queue.handle(),
        (),
    );
    fixture.roundtrip();
    fixture.drain_commands();

    // Half of a hundred-pixel extent is the middle of whatever region applies:
    // (300, 200) for the layout bounds (-300, -100)..(900, 500), (700, 150) for
    // test-B at (500, 0) with a 400x300 mode.
    plain.motion_absolute(200, 50, 50, 100, 100);
    targeted.motion_absolute(201, 50, 50, 100, 100);
    fixture.roundtrip();

    let commands = fixture.drain_commands();
    assert_eq!(commands.len(), 2);
    assert!(
        matches!(
            &commands[0],
            WmCommand::PointerMotion(PointerMotionCommand::Absolute { x, y, .. })
                if *x == 300.0 && *y == 200.0
        ),
        "a pointer without an output covers the whole layout: {commands:?}"
    );
    assert!(
        matches!(
            &commands[1],
            WmCommand::PointerMotion(PointerMotionCommand::Absolute { x, y, .. })
                if *x == 700.0 && *y == 150.0
        ),
        "a pointer created for an output covers only that output: {commands:?}"
    );
}
