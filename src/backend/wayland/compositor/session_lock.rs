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
    state::{SessionLockState, WaylandRuntime},
};

impl SessionLockHandler for WaylandRuntime {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.state.session_lock_manager_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        log::info!("session lock requested");

        if let SessionLockState::Locked(ref lock) = self.state.lock_state {
            if lock.is_alive() {
                log::info!("refusing lock: already locked with an active client");
                return;
            }
        }

        let lock = confirmation.ext_session_lock().clone();
        confirmation.lock();
        self.state.lock_state = SessionLockState::Locked(lock);
        log::info!("session locked");
    }

    fn unlock(&mut self) {
        log::info!("session unlocked");
        self.state.lock_state = SessionLockState::Unlocked;
        self.state.lock_surfaces.clear();
        self.state.restore_focus_after_overlay();
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
        let mode = output.current_mode().unwrap();
        surface.with_pending_state(|states| {
            let (w, h) = mode.size.into();
            states.size = Some((w as u32, h as u32).into());
        });
        surface.send_configure();

        let output_name = output.name();
        log::info!("session lock: new lock surface for output {output_name}");

        // Give the lock surface keyboard focus so the user can type their password.
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.state.seat.get_keyboard() {
            keyboard.set_focus(
                self,
                Some(KeyboardFocusTarget::WlSurface(surface.wl_surface().clone())),
                serial,
            );
        }

        self.state.lock_surfaces.insert(output_name, surface);
    }
}
