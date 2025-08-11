use crate::error::{InstantError, Result};
use crate::types::{Rectangle, WindowId};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        input::{InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent},
        renderer::{
            element::{AsRenderElements, RenderElement},
            gles::{GlesRenderer, GlesTexture},
            multigpu::{gbm::GbmGlesBackend, MultiRenderer},
            Bind, Frame, Renderer,
        },
        winit::{WinitEvent, WinitGraphicsBackend},
    },
    desktop::{
        space::{Space, SpaceElement},
        utils::under_from_surface_tree,
        Window, WindowSurfaceType,
    },
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerTarget, RelativeMotionEvent},
        Seat, SeatHandler, SeatState,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point, Rectangle as SmithayRectangle, Size, Transform},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        dmabuf::{DmabufGlobal, DmabufState},
        shell::xdg::{XdgShellState, XdgToplevelSurfaceData},
        shm::ShmState,
    },
};
use std::sync::{Arc, Mutex};

pub struct InstantCompositor {
    pub display: Display<Self>,
    pub space: Space<Window>,
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: DmabufGlobal,
    pub seat_state: SeatState<Self>,
    pub seat: Seat<Self>,
    pub windows: Arc<Mutex<Vec<Window>>>,
    pub focused_window: Arc<Mutex<Option<WindowId>>>,
}

impl InstantCompositor {
    pub fn new() -> Result<Self> {
        let display = Display::new()?;
        let mut space = Space::default();
        
        // Initialize compositor state
        let compositor_state = CompositorState::new::<Self>(&display.handle());
        let xdg_shell_state = XdgShellState::new::<Self>(&display.handle());
        let shm_state = ShmState::new::<Self>(&display.handle(), vec![]);
        
        // Initialize dmabuf
        let dmabuf_state = DmabufState::new();
        let dmabuf_global = dmabuf_state.create_global::<Self>(
            &display.handle(),
            1,
            vec![],
        );
        
        // Initialize seat
        let mut seat_state = SeatState::new();
        let seat = seat_state.new_wl_seat(&display.handle(), "seat0");
        
        Ok(Self {
            display,
            space,
            compositor_state,
            xdg_shell_state,
            shm_state,
            dmabuf_state,
            dmabuf_global,
            seat_state,
            seat,
            windows: Arc::new(Mutex::new(Vec::new())),
            focused_window: Arc::new(Mutex::new(None)),
        })
    }

    pub fn handle_winit_event(&mut self, event: WinitEvent) -> Result<()> {
        match event {
            WinitEvent::Input { event, .. } => self.handle_input_event(event),
            WinitEvent::Resized { size, .. } => {
                self.space.map_output(&self.seat, 1.0, (0, 0));
                Ok(())
            }
            WinitEvent::Redraw => {
                self.render()?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn handle_input_event(&mut self, event: InputEvent) -> Result<()> {
        match event {
            InputEvent::Keyboard { event } => self.handle_keyboard_event(event),
            InputEvent::PointerMotion { event } => self.handle_pointer_motion(event),
            InputEvent::PointerButton { event } => self.handle_pointer_button(event),
            InputEvent::PointerAxis { event } => self.handle_pointer_axis(event),
            _ => Ok(()),
        }
    }

    fn handle_keyboard_event(&mut self, event: impl KeyboardKeyEvent) -> Result<()> {
        let keycode = event.key_code();
        let state = event.state();
        
        if state == KeyState::Pressed {
            // Handle keybindings here
            tracing::debug!("Key pressed: {}", keycode);
        }
        
        Ok(())
    }

    fn handle_pointer_motion(&mut self, event: impl PointerMotionEvent) -> Result<()> {
        let location = event.position();
        tracing::debug!("Pointer motion: {:?}", location);
        Ok(())
    }

    fn handle_pointer_button(&mut self, event: impl PointerButtonEvent) -> Result<()> {
        let button = event.button();
        let state = event.state();
        tracing::debug!("Pointer button: {} {:?}", button, state);
        Ok(())
    }

    fn handle_pointer_axis(&mut self, event: impl PointerAxisEvent) -> Result<()> {
        let horizontal_amount = event.amount_discrete(Axis::Horizontal);
        let vertical_amount = event.amount_discrete(Axis::Vertical);
        tracing::debug!("Pointer axis: {:?} {:?}", horizontal_amount, vertical_amount);
        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        // Rendering logic will be implemented with the renderer
        Ok(())
    }

    pub fn add_window(&mut self, window: Window) -> WindowId {
        let mut windows = self.windows.lock().unwrap();
        let id = WindowId::new(windows.len() as u32);
        windows.push(window.clone());
        self.space.map_element(window, (0, 0), true);
        id
    }

    pub fn remove_window(&mut self, id: WindowId) -> Result<()> {
        let mut windows = self.windows.lock().unwrap();
        if let Some(index) = id.as_ffi() {
            if index < windows.len() as u32 {
                windows.remove(index as usize);
            }
        }
        Ok(())
    }

    pub fn get_window_geometry(&self, id: WindowId) -> Option<Rectangle> {
        let windows = self.windows.lock().unwrap();
        if let Some(index) = id.as_ffi() {
            if let Some(window) = windows.get(index as usize) {
                let bbox = window.bbox();
                return Some(Rectangle {
                    x: bbox.loc.x,
                    y: bbox.loc.y,
                    width: bbox.size.w as u32,
                    height: bbox.size.h as u32,
                });
            }
        }
        None
    }
}

impl SeatHandler for InstantCompositor {
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

impl CompositorClientState for InstantCompositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
}