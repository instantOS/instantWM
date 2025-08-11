//! Simplified Smithay compositor for InstantWM
//!
//! This is a minimal compositor implementation to get the project compiling.
//! More advanced features will be added incrementally.

use crate::error::Result;
use crate::types::Config;
use crate::window_manager::WindowManager;

use smithay::{
    desktop::{Space, Window},
    input::{Seat, SeatHandler, SeatState},
    output::Output,
    reexports::{
        calloop::LoopHandle,
        wayland_server::{backend::ClientData, Client, Display, DisplayHandle},
    },
    utils::{Point, Rectangle},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use std::sync::{Arc, Mutex};
use tracing::debug;

/// Main compositor state for InstantWM
pub struct InstantWMState {
    pub config: Config,
    pub start_time: std::time::Instant,
    pub socket_name: Option<String>,

    // Smithay state
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub display_handle: DisplayHandle,

    // Window management
    pub space: Space<Window>,
    pub window_manager: Arc<Mutex<WindowManager>>,
    pub seat: Seat<Self>,
    pub output: Output,

    // Event handling
    pub pointer_location: Point<f64, smithay::utils::Logical>,
}

impl InstantWMState {
    pub fn new(
        config: Config,
        display: Display<Self>,
        _event_loop_handle: LoopHandle<'static, Self>,
    ) -> Result<Self> {
        let start_time = std::time::Instant::now();
        let display_handle = display.handle();

        // Initialize Smithay states
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let mut seat_state = SeatState::new();

        // Create output
        let output = Output::new(
            "WAYLAND-1".to_string(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "InstantWM".into(),
                model: "Wayland".into(),
            },
        );

        // Create seat
        let seat = seat_state.new_wl_seat(&display_handle, "seat-0");

        // Initialize space
        let space = Space::<Window>::default();

        let screen_geometry = Rectangle::from_size((1920, 1080).into());
        let window_manager = Arc::new(Mutex::new(WindowManager::new(
            config.clone(),
            screen_geometry,
        )?));

        Ok(Self {
            config,
            start_time,
            socket_name: None,
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            display_handle,
            space,
            window_manager,
            seat,
            output,
            pointer_location: (0.0, 0.0).into(),
        })
    }

    pub fn handle_window_map(&mut self, window: Window) {
        debug!("Mapping window");

        // Add to window manager
        if let Ok(mut wm) = self.window_manager.lock() {
            wm.manage_window(window.clone());
        }

        // Map in space
        let location = (50, 50);
        self.space.map_element(window, location, true);
    }

    pub fn handle_window_unmap(&mut self, window: &Window) {
        debug!("Unmapping window");

        self.space.unmap_elem(window);

        if let Ok(mut wm) = self.window_manager.lock() {
            wm.unmanage_window(window);
        }
    }

    pub fn handle_keybinding(
        &mut self,
        keysym: xkbcommon::xkb::Keysym,
        modifiers: smithay::input::keyboard::ModifiersState,
    ) {
        if let Ok(mut wm) = self.window_manager.lock() {
            wm.handle_keybinding(keysym, modifiers);
        }
    }
}

// Implement required traits for Smithay

impl CompositorHandler for InstantWMState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<ClientState>() {
            &state.compositor_state
        } else {
            panic!("Unknown client data type")
        }
    }

    fn new_surface(
        &mut self,
        _surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        debug!("New surface created");
    }

    fn new_subsurface(
        &mut self,
        _surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        _parent: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        debug!("New subsurface created");
    }

    fn commit(
        &mut self,
        _surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        // Handle surface commits
    }
}

impl XdgShellHandler for InstantWMState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        debug!("New toplevel window");
        self.handle_window_map(window);
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        debug!("New popup created");
    }

    fn move_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        debug!("Move request for toplevel");
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
        _edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        debug!("Resize request for toplevel");
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        debug!("Popup grab request");
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
        debug!("Popup reposition request");
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let window_to_unmap = self
            .space
            .elements()
            .find(|w| {
                if let Some(toplevel) = w.toplevel() {
                    toplevel.wl_surface() == surface.wl_surface()
                } else {
                    false
                }
            })
            .cloned();

        if let Some(window) = window_to_unmap {
            self.handle_window_unmap(&window);
        }
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        debug!("Popup destroyed");
    }
}

impl ShmHandler for InstantWMState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for InstantWMState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
        // Handle buffer destruction
    }
}

impl SeatHandler for InstantWMState {
    type KeyboardFocus = smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    type PointerFocus = smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
    type TouchFocus = smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(
        &mut self,
        _seat: &Seat<Self>,
        _focus: Option<&smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    ) {
        // Handle focus changes
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
        // Handle cursor image changes
    }
}

#[derive(Debug)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: smithay::reexports::wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _client_id: smithay::reexports::wayland_server::backend::ClientId,
        _reason: smithay::reexports::wayland_server::backend::DisconnectReason,
    ) {
    }
}

// Delegate macro implementations
smithay::delegate_compositor!(InstantWMState);
smithay::delegate_shm!(InstantWMState);
smithay::delegate_xdg_shell!(InstantWMState);
smithay::delegate_seat!(InstantWMState);
