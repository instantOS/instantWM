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
    /// they must reach the lock surface as soon as one is mapped. Returns the
    /// lock surface to focus, or `None` while the lock client has not mapped
    /// a surface yet (the seat must then focus no client at all rather than
    /// leak keystrokes into the locked session).
    ///
    /// Every keyboard focus change must go through
    /// [`WaylandState::set_keyboard_focus`], which consults this while locked;
    /// focusing a lock surface directly is reserved for
    /// [`SessionLockHandler::new_surface`], which focuses the surface that
    /// has just been created.
    pub(crate) fn locked_keyboard_focus(&self) -> Option<KeyboardFocusTarget> {
        if !self.is_locked() {
            return None;
        }
        // Any live lock surface is a valid keyboard target: the seat has one
        // keyboard regardless of output count. Surfaces from a previous lock
        // client are dropped by `unlock`, so everything here belongs to the
        // active lock.
        self.lock_surfaces
            .values()
            .find(|surface| surface.alive())
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
        self.lock_state = SessionLockState::Locked(lock);
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

        // Give the lock surface keyboard focus so the user can type their password.
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(
                self,
                Some(KeyboardFocusTarget::WlSurface(surface.wl_surface().clone())),
                serial,
            );
        }

        self.lock_surfaces.insert(output_name, surface);
    }
}

#[cfg(test)]
mod session_lock_focus_tests {
    //! Regression coverage: while a session lock is active, WM focus churn
    //! must never move the keyboard away from the lock client.
    //!
    //! Reported failure mode: after leaving the machine locked for days, the
    //! lock screen stopped receiving keystrokes because an unrelated focus
    //! event (window close, overlay, popup grab, ...) had re-pointed the
    //! keyboard seat at a regular client. These tests drive a raw Wayland
    //! client over a socket pair — no separate client crate needed — and pin
    //! the seat-focus policy while locked.

    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::time::Duration;

    use smithay::reexports::wayland_server::Resource;
    use smithay::reexports::wayland_server::backend::ObjectId;
    use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    use smithay::utils::SERIAL_COUNTER;

    use crate::backend::wayland::compositor::{KeyboardFocusTarget, WaylandState};
    use crate::types::WindowId;

    // -- Minimal raw Wayland wire protocol helpers ---------------------------

    fn u32_bytes(value: u32) -> Vec<u8> {
        value.to_ne_bytes().to_vec()
    }

    fn string_bytes(value: &str) -> Vec<u8> {
        let mut out = u32_bytes(value.len() as u32 + 1);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        out
    }

    fn message(object: u32, opcode: u16, args: &[u8]) -> Vec<u8> {
        let size = (8 + args.len()) as u32;
        let mut out = u32_bytes(object);
        // Wayland packs the second header word as (size << 16) | opcode.
        out.extend_from_slice(&((size << 16) | opcode as u32).to_ne_bytes());
        out.extend_from_slice(args);
        out
    }

    struct RawClient {
        stream: UnixStream,
    }

    impl RawClient {
        fn send(&mut self, msg: &[u8]) {
            self.stream.write_all(msg).unwrap();
        }

        fn read_event(&mut self) -> std::io::Result<(u32, u16, Vec<u8>)> {
            let mut header = [0u8; 8];
            self.stream.read_exact(&mut header)?;
            let object = u32::from_ne_bytes(header[0..4].try_into().unwrap());
            let word = u32::from_ne_bytes(header[4..8].try_into().unwrap());
            let size = (word >> 16) as usize;
            let opcode = (word & 0xFFFF) as u16;
            let mut payload = vec![0u8; size.saturating_sub(8)];
            if !payload.is_empty() {
                self.stream.read_exact(&mut payload)?;
            }
            Ok((object, opcode, payload))
        }
    }

    fn dispatch_and_flush(
        event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
        state: &mut WaylandState,
    ) {
        event_loop
            .dispatch(Some(Duration::from_millis(250)), state)
            .expect("event loop dispatch");
        state.display_handle.flush_clients().expect("flush clients");
    }

    /// Bind the globals the test needs and create a `wl_surface`.
    ///
    /// Returns the surface resource, the raw client end, and the lock manager
    /// global name.
    fn setup_client(
        event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
        state: &mut WaylandState,
    ) -> (WlSurface, RawClient, u32) {
        let mut dh = state.display_handle.clone();
        let (client_stream, server_stream) = UnixStream::pair().unwrap();
        client_stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let client = dh
            .insert_client(server_stream, Arc::new(()))
            .expect("insert test client");
        let mut wire = RawClient {
            stream: client_stream,
        };

        // wl_display.get_registry(new_id = 2)
        wire.send(&message(1, 1, &u32_bytes(2)));
        dispatch_and_flush(event_loop, state);

        let mut compositor_name = None;
        let mut lock_manager_name = None;
        for _ in 0..256 {
            let (object, opcode, payload) = wire.read_event().expect("read registry globals");
            assert_ne!(
                (object, opcode),
                (1, 3),
                "wl_display.error during registry handshake"
            );
            if object != 2 || opcode != 0 {
                continue; // ignore wl_display.delete_id and other objects
            }
            // wl_registry.global: name, interface (string), version.
            let name = u32::from_ne_bytes(payload[0..4].try_into().unwrap());
            let len = u32::from_ne_bytes(payload[4..8].try_into().unwrap()) as usize;
            let iface = String::from_utf8(payload[8..8 + len - 1].to_vec()).unwrap();
            // Offset of the version word: 4-byte length prefix consumed
            // above, then the NUL-terminated string padded to 4 bytes.
            let version_offset = 8 + ((len + 3) & !3);
            let _version = u32::from_ne_bytes(
                payload[version_offset..version_offset + 4]
                    .try_into()
                    .unwrap(),
            );
            match iface.as_str() {
                "wl_compositor" => compositor_name = Some(name),
                "ext_session_lock_manager_v1" => lock_manager_name = Some(name),
                _ => {}
            }
            if compositor_name.is_some() && lock_manager_name.is_some() {
                break;
            }
        }
        let compositor_name = compositor_name.expect("wl_compositor global");
        let lock_manager_name = lock_manager_name.expect("ext_session_lock_manager_v1 global");

        // wl_registry.bind(name, "wl_compositor", 1, new_id = 3)
        let mut args = u32_bytes(compositor_name);
        args.extend(string_bytes("wl_compositor"));
        args.extend(u32_bytes(1));
        args.extend(u32_bytes(3));
        wire.send(&message(2, 0, &args));
        // wl_compositor.create_surface(new_id = 4)
        wire.send(&message(3, 0, &u32_bytes(4)));
        dispatch_and_flush(event_loop, state);

        let surface_id: ObjectId = dh
            .backend_handle()
            .object_for_protocol_id(client.id(), WlSurface::interface(), 4)
            .expect("surface object exists");
        let surface = WlSurface::from_id(&dh, surface_id).expect("surface resource");

        (surface, wire, lock_manager_name)
    }

    fn seat_focus(state: &WaylandState) -> Option<KeyboardFocusTarget> {
        state.seat.get_keyboard().unwrap().current_focus()
    }

    fn lock_session(
        event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
        state: &mut WaylandState,
        wire: &mut RawClient,
        registry_object: u32,
        lock_manager_name: u32,
    ) {
        // wl_registry.bind(name, "ext_session_lock_manager_v1", 1, new_id = 5)
        let mut args = u32_bytes(lock_manager_name);
        args.extend(string_bytes("ext_session_lock_manager_v1"));
        args.extend(u32_bytes(1));
        args.extend(u32_bytes(5));
        wire.send(&message(registry_object, 0, &args));
        // ext_session_lock_manager_v1.get_lock(new_id = 6)
        wire.send(&message(5, 1, &u32_bytes(6)));
        // ext_session_lock_v1.lock()
        wire.send(&message(6, 1, &[]));
        dispatch_and_flush(event_loop, state);
        assert!(
            state.is_locked(),
            "session must be locked after lock request"
        );
    }

    #[test]
    fn locked_session_never_gives_keyboard_to_a_regular_surface() {
        let (mut event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let (surface, mut wire, lock_manager_name) = setup_client(&mut event_loop, &mut state);
        let target = KeyboardFocusTarget::WlSurface(surface.clone());

        // Control: while unlocked, an explicit focus request is honored.
        state.set_keyboard_focus(Some(target.clone()), SERIAL_COUNTER.next_serial());
        assert_eq!(seat_focus(&state), Some(target.clone()));
        state.set_keyboard_focus(None, SERIAL_COUNTER.next_serial());
        assert_eq!(seat_focus(&state), None);

        lock_session(&mut event_loop, &mut state, &mut wire, 2, lock_manager_name);

        // Regression: with the session locked and no lock surface mapped yet,
        // a focus request pointing at a regular client must land on no client
        // at all — previously it handed the keyboard (and the user's
        // password) to that client.
        state.set_keyboard_focus(Some(target.clone()), SERIAL_COUNTER.next_serial());
        assert_eq!(seat_focus(&state), None);

        // The same holds for the WM focus funnel with an unknown window.
        state.set_focus(WindowId::from(9999));
        assert_eq!(seat_focus(&state), None);
    }
}
