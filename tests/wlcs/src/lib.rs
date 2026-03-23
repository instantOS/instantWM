use std::time::Duration;
use std::sync::mpsc;
use std::thread;
use std::os::unix::io::OwnedFd;
use std::io;

use instantwm::backend::Backend;
use instantwm::backend::wayland::WaylandBackend;
use instantwm::backend::wayland::compositor::WaylandState;
use instantwm::wm::Wm;

use smithay::reexports::calloop::{EventLoop};
use smithay::reexports::wayland_server::Display;

use wlcs::{Wlcs, Pointer, Touch};
use wlcs::ffi_display_server_api::{WlcsServerIntegration, WlcsIntegrationDescriptor, WlcsExtensionDescriptor};
use wlcs::ffi_wrappers::wlcs_server;

/// Events sent from WLCS thread to the compositor thread
enum WlcsEvent {
    Exit,
}

// The handle WLCS holds
struct InstantWmHandle {
    command_tx: mpsc::Sender<WlcsEvent>,
    thread_handle: Option<thread::JoinHandle<()>>,
    descriptor: WlcsIntegrationDescriptor,
}

impl Wlcs for InstantWmHandle {
    type Pointer = InstantWmPointer;
    type Touch = InstantWmTouch;

    fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        
        let thread_handle = thread::spawn(move || {
            let mut event_loop: EventLoop<'static, WaylandState> = EventLoop::try_new().expect("Failed to create event loop");
            let display = Display::new().expect("Failed to create display");
            
            let wayland_backend = WaylandBackend::new();
            let wm = Wm::new(Backend::new_wayland(wayland_backend));

            let mut state = WaylandState::new(
                display,
                &event_loop.handle(),
                wm,
                None,
            );

            // Simple loop that also checks for commands
            loop {
                if let Ok(event) = command_rx.try_recv() {
                    match event {
                        WlcsEvent::Exit => break,
                    }
                }
                event_loop.dispatch(Some(Duration::from_millis(10)), &mut state).expect("Dispatch failed");
                state.flush();
            }
        });

        let extensions = vec![
            WlcsExtensionDescriptor { name: "wl_compositor\0".as_ptr() as *const _, version: 4 },
            WlcsExtensionDescriptor { name: "xdg_wm_base\0".as_ptr() as *const _, version: 1 },
        ];

        Self {
            command_tx,
            thread_handle: Some(thread_handle),
            descriptor: WlcsIntegrationDescriptor {
                version: 1,
                num_extensions: extensions.len(),
                supported_extensions: Box::into_raw(extensions.into_boxed_slice()) as *const _,
            },
        }
    }

    fn start(&mut self) {
    }

    fn stop(&mut self) {
        let _ = self.command_tx.send(WlcsEvent::Exit);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    fn create_client_socket(&self) -> io::Result<OwnedFd> {
        let (s1, _s2) = std::os::unix::net::UnixStream::pair()?;
        Ok(OwnedFd::from(s1))
    }

    fn position_window_absolute(&self, _display: *mut wayland_sys::client::wl_display, _proxy: *mut wayland_sys::client::wl_proxy, _x: i32, _y: i32) {
    }

    fn create_pointer(&mut self) -> Option<Self::Pointer> {
        Some(InstantWmPointer {})
    }

    fn create_touch(&mut self) -> Option<Self::Touch> {
        Some(InstantWmTouch {})
    }

    fn get_descriptor(&self) -> &WlcsIntegrationDescriptor {
        &self.descriptor
    }
}

struct InstantWmPointer {}
impl Pointer for InstantWmPointer {
    fn move_absolute(&mut self, _x: i32, _y: i32) {}
    fn move_relative(&mut self, _dx: i32, _dy: i32) {}
    fn button_down(&mut self, _button: i32) {}
    fn button_up(&mut self, _button: i32) {}
    fn destroy(&mut self) {}
}

struct InstantWmTouch {
}

impl Touch for InstantWmTouch {
    fn touch_down(&mut self, _x: i32, _y: i32) {}
    fn touch_move(&mut self, _x: i32, _y: i32) {}
    fn touch_up(&mut self) {}
    fn destroy(&mut self) {}
}

wlcs::wlcs_server_integration!(InstantWmHandle);
