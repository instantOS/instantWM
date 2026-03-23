use std::time::Duration;
use std::sync::mpsc;
use std::thread;
use std::os::unix::io::OwnedFd;
use std::io;

use instantwm::backend::Backend;
use instantwm::backend::wayland::WaylandBackend;
use instantwm::backend::wayland::compositor::WaylandState;
use instantwm::wm::Wm;
use instantwm::wayland::input::pointer::motion::{MotionEvent, handle_pointer_motion};

use smithay::reexports::calloop::{EventLoop};
use smithay::reexports::wayland_server::Display;
use smithay::backend::input::{InputBackend, PointerButtonEvent, ButtonState, UnusedEvent, Device};

use wlcs::{Wlcs, Pointer, Touch};
use wlcs::ffi_display_server_api::{WlcsServerIntegration, WlcsIntegrationDescriptor, WlcsExtensionDescriptor};
use wlcs::ffi_wrappers::wlcs_server;

/// Events sent from WLCS thread to the compositor thread
enum WlcsEvent {
    Exit,
    NewClient(std::os::unix::net::UnixStream),
    PointerMoveAbs { x: f64, y: f64 },
    PointerButton { button: u32, state: ButtonState },
    TouchDown { x: f64, y: f64, id: i32 },
    TouchMove { x: f64, y: f64, id: i32 },
    TouchUp { id: i32 },
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
struct MockDevice;
impl Device for MockDevice {
    fn id(&self) -> String { "mock".into() }
    fn name(&self) -> String { "mock".into() }
    fn has_capability(&self, _cap: smithay::backend::input::DeviceCapability) -> bool { true }
    fn usb_id(&self) -> Option<(u32, u32)> { None }
    fn syspath(&self) -> Option<std::path::PathBuf> { None }
}

struct MockBackend;
impl InputBackend for MockBackend {
    type Device = MockDevice;
    type KeyboardKeyEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;
    type PointerButtonEvent = MockButtonEvent;
    type PointerAxisEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}

struct MockButtonEvent {
    button: u32,
    state: ButtonState,
}

impl smithay::backend::input::Event<MockBackend> for MockButtonEvent {
    fn time_msec(&self) -> u32 { 0 }
    fn time(&self) -> u64 { 0 }
    fn device(&self) -> MockDevice { MockDevice }
}

impl PointerButtonEvent<MockBackend> for MockButtonEvent {
    fn button(&self) -> Option<smithay::backend::input::MouseButton> {
        None
    }
    fn button_code(&self) -> u32 { self.button }
    fn state(&self) -> ButtonState { self.state }
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
            let mut dh = display.handle();
            
            let wayland_backend = WaylandBackend::new();
            let wm = Wm::new(Backend::new_wayland(wayland_backend));

            let mut state = WaylandState::new(
                display,
                &event_loop.handle(),
                wm,
                None,
            );

            // Mock an output so we have some coordinate space
            let output = smithay::output::Output::new(
                "WLCS-Output".into(),
                smithay::output::PhysicalProperties {
                    size: (1920, 1080).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: "WLCS".into(),
                    model: "Virtual".into(),
                },
            );
            let mode = smithay::output::Mode {
                size: (1920, 1080).into(),
                refresh: 60000,
            };
            output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
            output.set_preferred(mode);
            state.space.map_output(&output, (0, 0));
            state.wm.g.cfg.screen_width = 1920;
            state.wm.g.cfg.screen_height = 1080;

            // Simple loop that also checks for commands
            loop {
                // Process all pending commands
                while let Ok(event) = command_rx.try_recv() {
                    match event {
                        WlcsEvent::Exit => return,
                        WlcsEvent::NewClient(stream) => {
                            if let Err(err) = dh.insert_client(stream, std::sync::Arc::new(instantwm::backend::wayland::compositor::WaylandClientState::default())) {
                                log::error!("Failed to insert WLCS client: {}", err);
                            }
                        }
                        WlcsEvent::PointerMoveAbs { x, y } => {
                            let pointer = state.pointer.clone();
                            let keyboard = state.keyboard.clone();
                            handle_pointer_motion(
                                &mut state,
                                &pointer,
                                &keyboard,
                                MotionEvent::Absolute { x, y, time_msec: 0 },
                            );
                        }
                        WlcsEvent::PointerButton { button, state: btn_state } => {
                            let pointer = state.pointer.clone();
                            let keyboard = state.keyboard.clone();
                            let loc = state.pointer_location;
                            instantwm::wayland::input::pointer::button::handle_pointer_button::<MockBackend>(
                                &mut state,
                                &pointer,
                                &keyboard,
                                MockButtonEvent { button, state: btn_state },
                                loc,
                            );
                        }
                        _ => {} // TODO: Touch
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
        let (s1, s2) = std::os::unix::net::UnixStream::pair()?;
        self.command_tx.send(WlcsEvent::NewClient(s2)).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(OwnedFd::from(s1))
    }

    fn position_window_absolute(&self, _display: *mut wayland_sys::client::wl_display, _proxy: *mut wayland_sys::client::wl_proxy, _x: i32, _y: i32) {
    }

    fn create_pointer(&mut self) -> Option<Self::Pointer> {
        Some(InstantWmPointer { command_tx: self.command_tx.clone() })
    }

    fn create_touch(&mut self) -> Option<Self::Touch> {
        Some(InstantWmTouch { command_tx: self.command_tx.clone() })
    }

    fn get_descriptor(&self) -> &WlcsIntegrationDescriptor {
        &self.descriptor
    }
}

struct InstantWmPointer {
    command_tx: mpsc::Sender<WlcsEvent>,
}

impl Pointer for InstantWmPointer {
    fn move_absolute(&mut self, x: i32, y: i32) {
        let _ = self.command_tx.send(WlcsEvent::PointerMoveAbs { x: x as f64, y: y as f64 });
    }
    fn move_relative(&mut self, _dx: i32, _dy: i32) {
    }
    fn button_down(&mut self, button: i32) {
        let _ = self.command_tx.send(WlcsEvent::PointerButton { 
            button: button as u32, 
            state: ButtonState::Pressed 
        });
    }
    fn button_up(&mut self, button: i32) {
        let _ = self.command_tx.send(WlcsEvent::PointerButton { 
            button: button as u32, 
            state: ButtonState::Released 
        });
    }
    fn destroy(&mut self) {}
}

struct InstantWmTouch {
    command_tx: mpsc::Sender<WlcsEvent>,
}

impl Touch for InstantWmTouch {
    fn touch_down(&mut self, x: i32, y: i32) {
        let _ = self.command_tx.send(WlcsEvent::TouchDown { x: x as f64, y: y as f64, id: 0 });
    }
    fn touch_move(&mut self, x: i32, y: i32) {
        let _ = self.command_tx.send(WlcsEvent::TouchMove { x: x as f64, y: y as f64, id: 0 });
    }
    fn touch_up(&mut self) {
        let _ = self.command_tx.send(WlcsEvent::TouchUp { id: 0 });
    }
    fn destroy(&mut self) {}
}

wlcs::wlcs_server_integration!(InstantWmHandle);
