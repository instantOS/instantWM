//! Unified calloop source setup for all backends.
//!
//! This module provides helper functions to set up common calloop sources
//! (IPC, animation timer) that are shared across X11, Wayland/DRM, and winit
//! backends. Each helper accepts callbacks so backend-specific behaviour
//! (e.g. DRM dirty-marking, LED state checks) can be injected without
//! duplicating the boilerplate.

use std::time::Duration;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, LoopHandle, Mode, PostAction};

/// Setup IPC as a calloop event source.
///
/// When a client connects, `on_ipc` is called with mutable access to the
/// IpcServer and the event loop data.
pub fn setup_ipc_source<Data: 'static>(
    loop_handle: &LoopHandle<'static, Data>,
    ipc_server: crate::ipc::IpcServer,
    mut on_ipc: impl FnMut(&mut crate::ipc::IpcServer, &mut Data) + 'static,
) {
    let source = Generic::new(ipc_server, Interest::READ, Mode::Level);
    loop_handle
        .insert_source(source, move |_, ipc_server, data| {
            // SAFETY: We're not dropping the IpcServer, just calling process_pending
            let ipc = unsafe { ipc_server.get_mut() };
            on_ipc(ipc, data);
            Ok(PostAction::Continue)
        })
        .expect("ipc source");
}

/// Setup an animation timer that ticks at ~60fps when active.
///
/// `tick` is called every 16ms when animations might be active.
/// `is_active` should return true if animations are still running.
/// `on_tick` is called each tick and receives the data.
///
/// When `is_active` returns false, the timer sleeps for a long time
/// to avoid unnecessary CPU usage.
pub fn setup_animation_timer<Data: 'static>(
    loop_handle: &LoopHandle<'static, Data>,
    mut tick: impl FnMut(&mut Data) + 'static,
    mut is_active: impl FnMut(&Data) -> bool + 'static,
) {
    let timer = Timer::from_duration(Duration::from_millis(16));
    loop_handle
        .insert_source(timer, move |_, _, data| {
            tick(data);
            if is_active(data) {
                TimeoutAction::ToDuration(Duration::from_millis(16))
            } else {
                // No animations, sleep until woken by something else
                TimeoutAction::ToDuration(Duration::from_secs(86400))
            }
        })
        .expect("animation timer source");
}
