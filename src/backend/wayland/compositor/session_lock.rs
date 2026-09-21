use smithay::{
    output::Output,
    reexports::wayland_server::Resource,
    utils::SERIAL_COUNTER,
    wayland::session_lock::{
        LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
    },
};

use super::{
    focus::KeyboardFocusTarget,
    state::{SessionLockState, WaylandState},
};

impl WaylandState {
    /// Keyboard focus that must hold while the session is locked.
    ///
    /// The lock client owns the session keyboard for as long as the lock is
    /// active: key events must never be delivered to any other client, and
    /// they must reach a lock surface as soon as one is created. Returns the
    /// lock surface to focus, or `None` while the lock client has not created
    /// one yet (the seat must then focus no client at all rather than
    /// leak keystrokes into the locked session).
    ///
    /// Every keyboard focus change goes through
    /// [`WaylandState::set_keyboard_focus`], which consults this while locked.
    pub(crate) fn locked_keyboard_focus(&self) -> Option<KeyboardFocusTarget> {
        if !self.is_locked() {
            return None;
        }
        // Any live lock surface is a valid keyboard target: the seat has one
        // keyboard regardless of output count. Old surfaces are cleared on
        // unlock and when a replacement lock client takes over.
        let current = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        self.lock_surfaces
            .values()
            .filter(|surface| surface.alive())
            .find(|surface| {
                current.as_ref()
                    == Some(&KeyboardFocusTarget::WlSurface(
                        surface.wl_surface().clone(),
                    ))
            })
            .or_else(|| self.lock_surfaces.values().find(|surface| surface.alive()))
            .map(|surface| KeyboardFocusTarget::WlSurface(surface.wl_surface().clone()))
    }
}

impl SessionLockHandler for WaylandState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_manager_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        log::info!("session lock requested");

        if let SessionLockState::Locked(ref lock) = self.lock_state
            && lock.is_alive()
        {
            log::info!("refusing lock: already locked with an active client");
            return;
        }

        // A touch sequence keeps the surface selected by its initial down
        // event. Cancel it before exposing lock surfaces so a pre-lock client
        // can never retain touch focus while the session is locked.
        self.touch.clone().cancel(self);
        self.cancel_touch_pointer_emulation(smithay::backend::input::InputTime::now());
        self.runtime.wm_gesture_touch_slot = None;

        let lock = confirmation.ext_session_lock().clone();
        confirmation.lock();
        // A replacement lock client must not inherit the previous client's
        // surfaces or any keyboard focus/grab from the unlocked session.
        self.lock_surfaces.clear();
        self.lock_state = SessionLockState::Locked(lock);
        self.set_keyboard_focus(None, SERIAL_COUNTER.next_serial());
        self.push_command(
            crate::backend::wayland::commands::WmCommand::CancelInteractiveDrag(
                crate::core_state::DragCancelReason::SessionLocked,
            ),
        );
        log::info!("session locked");
    }

    fn unlock(&mut self) {
        log::info!("session unlocked");
        // Do not let a sequence focused on the lock client survive after its
        // surfaces are removed.
        self.touch.clone().cancel(self);
        self.cancel_touch_pointer_emulation(smithay::backend::input::InputTime::now());
        self.lock_state = SessionLockState::Unlocked;
        self.lock_surfaces.clear();
        self.clear_seat_focus();
        self.restore_focus_after_overlay();
    }

    fn new_surface(
        &mut self,
        surface: LockSurface,
        output: smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
    ) {
        let Some(output) = Output::from_resource(&output) else {
            log::warn!("session lock: no Output matching WlOutput");
            return;
        };

        // Configure the lock surface to cover the full output.
        let mode = output
            .current_mode()
            .expect("output must have a current mode for lock surface sizing");
        surface.with_pending_state(|states| {
            let (w, h) = mode.size.into();
            states.size = Some((w as u32, h as u32).into());
        });
        surface.send_configure();

        let output_name = output.name();
        log::info!("session lock: new lock surface for output {output_name}");

        // A destroyed wl_surface must stop being rendered and focused at once.
        // Keep the identity check: a replacement surface may already own the
        // same output when an older destruction hook runs.
        let removed_output_name = output_name.clone();
        smithay::wayland::compositor::add_destruction_hook::<Self, _>(
            surface.wl_surface(),
            move |state, destroyed| {
                let is_current = state
                    .lock_surfaces
                    .get(&removed_output_name)
                    .is_some_and(|lock_surface| lock_surface.wl_surface() == destroyed);
                if !is_current {
                    return;
                }
                state.lock_surfaces.remove(&removed_output_name);
                state.request_output_name_render(removed_output_name.clone());
                let focused = state
                    .seat
                    .get_keyboard()
                    .and_then(|keyboard| keyboard.current_focus())
                    == Some(KeyboardFocusTarget::WlSurface(destroyed.clone()));
                if focused {
                    state.set_keyboard_focus(None, SERIAL_COUNTER.next_serial());
                }
            },
        );
        self.lock_surfaces.insert(output_name, surface);
        // The first surface receives focus; later outputs keep the current
        // lock surface focused until it disappears.
        self.set_keyboard_focus(None, SERIAL_COUNTER.next_serial());
    }
}

#[cfg(test)]
mod session_lock_focus_tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::time::Duration;

    use smithay::backend::input::{InputTime, KeyState};
    use smithay::input::keyboard::{
        GrabStartData, KeyboardGrab, KeyboardInnerHandle, Keycode, ModifiersState,
    };
    use smithay::reexports::wayland_server::{Client as ServerClient, Resource, backend::ObjectId};
    use smithay::utils::{SERIAL_COUNTER, Serial};
    use wayland_client::protocol::{
        wl_callback, wl_compositor, wl_output, wl_registry, wl_surface,
    };
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols::ext::session_lock::v1::client::{
        ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
    };

    use crate::backend::wayland::compositor::{
        KeyboardFocusTarget, WaylandClientState, WaylandState,
    };
    use crate::types::{Size, WindowId};

    #[derive(Default)]
    struct TestClient {
        globals: Vec<(u32, String)>,
        sync_count: usize,
    }

    impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
        fn event(
            state: &mut Self,
            _: &wl_callback::WlCallback,
            _: wl_callback::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            state.sync_count += 1;
        }
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
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

    delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
    delegate_noop!(TestClient: ignore wl_surface::WlSurface);
    delegate_noop!(TestClient: ignore wl_output::WlOutput);
    delegate_noop!(TestClient: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
    delegate_noop!(TestClient: ignore ext_session_lock_v1::ExtSessionLockV1);
    delegate_noop!(TestClient: ignore ext_session_lock_surface_v1::ExtSessionLockSurfaceV1);

    fn pump(
        event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
        state: &mut WaylandState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) {
        // A sync request guarantees that every preceding client request has
        // been handled before the client queue returns from dispatch.
        let next_sync = client.sync_count + 1;
        conn.display().sync(&queue.handle(), ());
        conn.flush().expect("flush test client");
        event_loop
            .dispatch(Some(Duration::from_millis(250)), state)
            .expect("event loop dispatch");
        state.display_handle.flush_clients().expect("flush clients");
        while client.sync_count < next_sync {
            queue
                .blocking_dispatch(client)
                .expect("valid Wayland protocol exchange");
        }
    }

    fn global(client: &TestClient, interface: &str) -> u32 {
        client
            .globals
            .iter()
            .find(|(_, name)| name == interface)
            .unwrap_or_else(|| panic!("missing {interface} global"))
            .0
    }

    fn seat_focus(state: &WaylandState) -> Option<KeyboardFocusTarget> {
        state.seat.get_keyboard().unwrap().current_focus()
    }

    fn connect_client(
        event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
        state: &mut WaylandState,
    ) -> (
        Connection,
        EventQueue<TestClient>,
        TestClient,
        wl_registry::WlRegistry,
        ServerClient,
    ) {
        let (client_socket, server_socket) = UnixStream::pair().unwrap();
        let mut dh = state.display_handle.clone();
        let server_client = dh
            .insert_client(server_socket, Arc::new(WaylandClientState::default()))
            .expect("insert test client");
        let conn = Connection::from_socket(client_socket).expect("connect test client");
        let mut queue = conn.new_event_queue::<TestClient>();
        let mut client = TestClient::default();
        let registry = conn.display().get_registry(&queue.handle(), ());
        pump(event_loop, state, &conn, &mut queue, &mut client);
        (conn, queue, client, registry, server_client)
    }

    /// Models a client grab that refuses compositor focus changes.
    struct StickyGrab {
        start: GrabStartData<WaylandState>,
    }

    impl KeyboardGrab<WaylandState> for StickyGrab {
        fn input(
            &mut self,
            data: &mut WaylandState,
            handle: &mut KeyboardInnerHandle<'_, WaylandState>,
            keycode: Keycode,
            state: KeyState,
            modifiers: Option<ModifiersState>,
            serial: Serial,
            time: InputTime,
        ) {
            handle.input(data, keycode, state, modifiers, serial, time);
        }

        fn set_focus(
            &mut self,
            _: &mut WaylandState,
            _: &mut KeyboardInnerHandle<'_, WaylandState>,
            _: Option<KeyboardFocusTarget>,
            _: Serial,
        ) {
            // Like a popup grab, reject unrelated focus requests.
        }

        fn start_data(&self) -> &GrabStartData<WaylandState> {
            &self.start
        }

        fn unset(&mut self, _: &mut WaylandState) {}
    }

    fn install_sticky_grab(state: &mut WaylandState, target: KeyboardFocusTarget) {
        let keyboard = state.seat.get_keyboard().unwrap();
        keyboard.set_grab(
            state,
            StickyGrab {
                start: GrabStartData {
                    focus: Some(target),
                },
            },
            SERIAL_COUNTER.next_serial(),
        );
        assert!(keyboard.is_grabbed());
    }

    #[test]
    fn lock_focus_survives_grabs_and_output_surface_changes() {
        let (mut event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        state.create_output("lock-test", Size::new(800, 600), None);

        let (conn, mut queue, mut client, registry, server_client) =
            connect_client(&mut event_loop, &mut state);
        let dh = state.display_handle.clone();
        let qh = queue.handle();

        let compositor: wl_compositor::WlCompositor =
            registry.bind(global(&client, "wl_compositor"), 1, &qh, ());
        let regular = compositor.create_surface(&qh, ());
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        let regular_id: ObjectId = dh
            .backend_handle()
            .object_for_protocol_id(
                server_client.id(),
                smithay::reexports::wayland_server::protocol::wl_surface::WlSurface::interface(),
                regular.id().protocol_id(),
            )
            .expect("regular surface exists");
        let regular_server =
            smithay::reexports::wayland_server::protocol::wl_surface::WlSurface::from_id(
                &dh, regular_id,
            )
            .expect("regular surface resource");
        let regular_target = KeyboardFocusTarget::WlSurface(regular_server);
        state.set_keyboard_focus(Some(regular_target.clone()), SERIAL_COUNTER.next_serial());
        assert_eq!(seat_focus(&state), Some(regular_target.clone()));
        install_sticky_grab(&mut state, regular_target.clone());

        let manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 =
            registry.bind(global(&client, "ext_session_lock_manager_v1"), 1, &qh, ());
        let lock = manager.lock(&qh, ());
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        assert!(state.is_locked());
        assert!(!state.seat.get_keyboard().unwrap().is_grabbed());
        assert_eq!(
            seat_focus(&state),
            None,
            "lock must revoke old focus immediately"
        );

        state.set_keyboard_focus(Some(regular_target.clone()), SERIAL_COUNTER.next_serial());
        assert_eq!(seat_focus(&state), None);
        state.set_focus(WindowId::from(9999));
        assert_eq!(seat_focus(&state), None);

        let output: wl_output::WlOutput = registry.bind(global(&client, "wl_output"), 1, &qh, ());
        let lock_wl_surface = compositor.create_surface(&qh, ());
        let lock_surface = lock.get_lock_surface(&lock_wl_surface, &output, &qh, ());
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        let expected = KeyboardFocusTarget::WlSurface(
            state
                .lock_surfaces
                .get("lock-test")
                .expect("lock surface registered")
                .wl_surface()
                .clone(),
        );
        assert_eq!(seat_focus(&state), Some(expected.clone()));
        // A grab installed after the lock must also be removed before a
        // compositor focus request can hand input to a regular client.
        install_sticky_grab(&mut state, regular_target.clone());
        state.set_keyboard_focus(Some(regular_target), SERIAL_COUNTER.next_serial());
        assert!(!state.seat.get_keyboard().unwrap().is_grabbed());
        assert_eq!(seat_focus(&state), Some(expected.clone()));

        // A new output's lock surface must not move the sole seat keyboard
        // away from the first lock surface.
        state.create_output("lock-test-two", Size::new(640, 480), None);
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        let second_output_name = client
            .globals
            .iter()
            .rev()
            .find(|(_, interface)| interface == "wl_output")
            .expect("second output global")
            .0;
        let second_output: wl_output::WlOutput = registry.bind(second_output_name, 1, &qh, ());
        let second_wl_surface = compositor.create_surface(&qh, ());
        let _second_lock_surface =
            lock.get_lock_surface(&second_wl_surface, &second_output, &qh, ());
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        let second_target = KeyboardFocusTarget::WlSurface(
            state
                .lock_surfaces
                .get("lock-test-two")
                .expect("second lock surface registered")
                .wl_surface()
                .clone(),
        );
        assert_eq!(seat_focus(&state), Some(expected));

        // If the focused output disappears, keyboard focus should move to a
        // surviving lock surface before another key is handled.
        lock_surface.destroy();
        lock_wl_surface.destroy();
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        assert_eq!(seat_focus(&state), Some(second_target));
    }

    #[test]
    fn disconnected_lock_client_stays_locked_until_a_new_client_takes_over() {
        let (mut event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        state.create_output("lock-test", Size::new(800, 600), None);

        {
            let (conn, mut queue, mut client, registry, _) =
                connect_client(&mut event_loop, &mut state);
            let qh = queue.handle();
            let compositor: wl_compositor::WlCompositor =
                registry.bind(global(&client, "wl_compositor"), 1, &qh, ());
            let manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 =
                registry.bind(global(&client, "ext_session_lock_manager_v1"), 1, &qh, ());
            let output: wl_output::WlOutput =
                registry.bind(global(&client, "wl_output"), 1, &qh, ());
            let lock = manager.lock(&qh, ());
            let wl_surface = compositor.create_surface(&qh, ());
            let _lock_surface = lock.get_lock_surface(&wl_surface, &output, &qh, ());
            pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
            assert!(state.is_locked());
            assert!(seat_focus(&state).is_some());
        }

        // Disconnecting must blank the old surface without unlocking the
        // session or leaving the keyboard on the dead client.
        event_loop
            .dispatch(Some(Duration::from_millis(250)), &mut state)
            .expect("process client disconnect");
        assert!(state.is_locked());
        assert!(state.lock_surfaces.is_empty());
        assert_eq!(seat_focus(&state), None);

        let (conn, mut queue, mut client, registry, _) =
            connect_client(&mut event_loop, &mut state);
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor =
            registry.bind(global(&client, "wl_compositor"), 1, &qh, ());
        let manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 =
            registry.bind(global(&client, "ext_session_lock_manager_v1"), 1, &qh, ());
        let output: wl_output::WlOutput = registry.bind(global(&client, "wl_output"), 1, &qh, ());
        let lock = manager.lock(&qh, ());
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        assert!(state.is_locked());
        assert_eq!(seat_focus(&state), None);

        let wl_surface = compositor.create_surface(&qh, ());
        let _lock_surface = lock.get_lock_surface(&wl_surface, &output, &qh, ());
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        let replacement = state
            .lock_surfaces
            .get("lock-test")
            .expect("replacement lock surface registered")
            .wl_surface()
            .clone();
        assert_eq!(
            seat_focus(&state),
            Some(KeyboardFocusTarget::WlSurface(replacement))
        );

        lock.unlock_and_destroy();
        pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        assert!(!state.is_locked());
        assert!(state.lock_surfaces.is_empty());
        assert_eq!(seat_focus(&state), None);
    }
}
