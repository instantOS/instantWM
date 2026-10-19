//! `zwp_virtual_keyboard_v1` keymap handling, exercised by a real client.
//!
//! A virtual keyboard's keycodes only mean something under the keymap it sent,
//! so these tests drive the protocol from the client side and then read the
//! seat keyboard, which is the same lookup the compositor's key filter runs.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use smithay::input::keyboard::xkb;
use wayland_client::protocol::{wl_callback, wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::types::KeyboardLayout;

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

/// xkb keycode of `Z` (evdev 44 + 8): 'z' on the US layout, 'y' on the German
/// one, so one lookup tells the two keymaps apart.
const Z_KEYCODE: u16 = 52;

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

delegate_noop!(ClientEvents: ignore wl_seat::WlSeat);
delegate_noop!(ClientEvents: ignore zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
delegate_noop!(ClientEvents: ignore zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

/// Read the keysym the seat keyboard resolves `keycode` to.
fn key_symbol(state: &mut WaylandState, keycode: u16) -> Option<char> {
    state
        .native
        .keyboard
        .clone()
        .with_xkb_state(state, |context| {
            let xkb = context.xkb().lock().unwrap();
            xkb.raw_syms_for_key_in_layout(keycode.into(), xkb.active_layout())
                .first()
                .and_then(|symbol| symbol.key_char())
        })
}

/// Compile a keymap for `layout`, as a client would before sending it.
fn compiled_keymap(layout: &str) -> String {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "evdev",
        "",
        layout,
        "",
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .expect("compile keymap");
    keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
}

struct Fixture {
    event_loop: EventLoop,
    state: WaylandState,
    conn: Connection,
    queue: EventQueue<ClientEvents>,
    events: ClientEvents,
    seat: wl_seat::WlSeat,
    manager: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
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
        Self::roundtrip(&mut event_loop, &mut state, &conn, &mut queue, &mut events);

        let global = |events: &ClientEvents, interface: &str| {
            events
                .globals
                .iter()
                .find(|(_, name)| name == interface)
                .unwrap()
                .0
        };
        let seat = registry.bind(global(&events, "wl_seat"), 1, &queue.handle(), ());
        let manager = registry.bind(
            global(&events, "zwp_virtual_keyboard_manager_v1"),
            1,
            &queue.handle(),
            (),
        );

        Self {
            event_loop,
            state,
            conn,
            queue,
            events,
            seat,
            manager,
        }
    }

    fn roundtrip(
        event_loop: &mut EventLoop,
        state: &mut WaylandState,
        conn: &Connection,
        queue: &mut EventQueue<ClientEvents>,
        events: &mut ClientEvents,
    ) {
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

    fn pump(&mut self) {
        Self::roundtrip(
            &mut self.event_loop,
            &mut self.state,
            &self.conn,
            &mut self.queue,
            &mut self.events,
        );
    }

    fn virtual_keyboard(&mut self) -> zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1 {
        self.manager
            .create_virtual_keyboard(&self.seat, &self.queue.handle(), ())
    }

    fn send_keymap(
        &mut self,
        keyboard: &zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
        layout: &str,
    ) {
        let keymap = compiled_keymap(layout);
        // Clients send the keymap as a NUL-terminated string and include the
        // terminator in the size.
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(keymap.as_bytes()).unwrap();
        file.write_all(b"\0").unwrap();
        keyboard.keymap(
            wl_keyboard::KeymapFormat::XkbV1.into(),
            file.as_fd(),
            keymap.len() as u32 + 1,
        );
        self.pump();
    }
}

#[test]
fn a_virtual_keyboard_keymap_replaces_the_seat_keymap_and_is_restored_when_gone() {
    let mut fixture = Fixture::new();
    // Pin the configured layout instead of trusting the environment: it is what
    // the seat has to fall back to.
    fixture
        .state
        .wm
        .core
        .state
        .interaction
        .keyboard_layout
        .layouts = vec![KeyboardLayout::new("us")];
    crate::keyboard_layout::set_keyboard_layout(&mut fixture.state.ctx(), 0);
    assert_eq!(key_symbol(&mut fixture.state, Z_KEYCODE), Some('z'));

    let keyboard = fixture.virtual_keyboard();
    fixture.send_keymap(&keyboard, "de");
    assert_eq!(
        key_symbol(&mut fixture.state, Z_KEYCODE),
        Some('y'),
        "the keymap sent by the client has to be active while it exists"
    );

    keyboard.destroy();
    fixture.pump();
    assert_eq!(
        key_symbol(&mut fixture.state, Z_KEYCODE),
        Some('z'),
        "the configured layout has to be back once the virtual keyboard is gone"
    );
}
