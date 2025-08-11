mod config;
mod layouts;
mod workspace;
mod keybindings;

use layouts::{Layout, TileLayout};
use workspace::Workspace;
use smithay::{
    backend::{
        input::{InputEvent, KeyboardKeyEvent},
        renderer::{
            glow::GlowRenderer,
            Renderer, Frame,
            utils::render_surface_tree,
        },
        winit::{self, WinitEvent},
    },
    reexports::{
        calloop::{EventLoop, LoopHandle, generic::Generic},
        wayland_server::{Display, DisplayHandle, GlobalDispatch, Dispatch, Client},
    },
    input::{Seat, SeatState, SeatHandler, KeyboardHandle, PointerHandle, pointer::{PointerButton, ButtonState}},
    utils::{Point, Scale, Rectangle},
    wayland::{
        compositor::{CompositorState, CompositorHandler, CompositorClientState, self},
        shm::{ShmState, ShmHandler},
        buffer::BufferHandler,
        shell::xdg::{XdgShellState, XdgShellHandler, ToplevelSurface},
        seat::Seat as WaylandSeat,
    },
    output, delegate_compositor, delegate_shm, delegate_xdg_shell, delegate_seat,
};
use smithay::reexports::winit::{
    event_loop::EventLoop as WinitEventLoop,
    event::ModifiersState,
};
use smithay::reexports::wayland_server::protocol::{wl_compositor::WlCompositor, wl_shm::WlShm, wl_buffer::WlBuffer, wl_surface::WlSurface, wl_seat::WlSeat};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase;
use sctk_adwaita::{AdwaitaTheme, Theme};
use std::fs;
use std::os::unix::net::UnixListener;


pub struct Window {
    surface: ToplevelSurface,
    geometry: Rectangle<i32, smithay::utils::Physical>,
}

// The state of our compositor.
struct State {
    display_handle: DisplayHandle,
    handle: LoopHandle<'static, Self>,
    output: output::Output,
    compositor_state: CompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
    seat_state: SeatState<Self>,
    seat: Seat<Self>,
    workspaces: Vec<Workspace>,
    active_workspace: usize,
    theme: AdwaitaTheme,
    modifiers: ModifiersState,
    drag_data: Option<(ToplevelSurface, Point<i32, smithay::utils::Physical>)>,
    resize_data: Option<(ToplevelSurface, Point<i32, smithay::utils::Physical>)>,
}

fn main() {
    // Setup logging
    tracing_subscriber::fmt::init();

    // Create the event loop and the Wayland display
    let mut event_loop: EventLoop<State> = EventLoop::try_new().unwrap();
    let mut display: Display<State> = Display::new().unwrap();
    let display_handle = display.handle();

    // Create the compositor state
    let compositor_state = CompositorState::new::<State>(&display.handle());
    let shm_state = ShmState::new::<State>(&display.handle(), vec![]);
    let xdg_shell_state = XdgShellState::new::<State>(&display.handle());
    let mut seat_state = SeatState::<State>::new();
    let mut seat = seat_state.new_seat("winit");
    seat.add_keyboard(Default::default(), 200, 25).unwrap();
    seat.add_pointer();
    let theme = AdwaitaTheme::new();

    // Create the winit backend
    let (mut backend, mut winit_event_loop) =
        winit::init::<GlowRenderer>().expect("Failed to initialize winit backend");

    let size = backend.window_size();
    let raw_size: smithay::utils::Size<i32, smithay::utils::Raw> = (size.w, size.h).into();


    // Create the compositor state
    let mut state = State::new(display_handle.clone(), event_loop.handle(), raw_size, compositor_state, shm_state, xdg_shell_state, seat_state, seat, theme);

    // Init globals
    display.create_global::<State, WlCompositor, _>(3, ());
    display.create_global::<State, WlShm, _>(1, ());
    display.create_global::<State, XdgWmBase, _>(1, ());
    display.create_global::<State, WlSeat, _>(7, ());


    println!("Starting the Wayland compositor...");

    // The main event loop
    loop {
        // Process winit events
        winit_event_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { .. } => {
                // Handle resize
            }
            WinitEvent::Input(event) => {
                state.process_input_event(event);
            }
            _ => (),
        });

        let size = backend.window_size();
        let bar_height = 30;

        // Arrange windows
        let output_geometry = Rectangle::from_loc_and_size((0, bar_height), (size.w, size.h - bar_height));
        state.workspaces[state.active_workspace].arrange(output_geometry);

        // Render the frame
        let mut renderer = backend.renderer();
        let mut frame = renderer.frame().unwrap();
        frame.clear([0.1, 0.1, 0.1, 1.0], &[]).unwrap();

        // Draw the bar
        let bar_color = [0.0, 0.0, 0.0, 1.0];
        frame.render_solid(
            Rectangle::from_loc_and_size((0, 0), (size.w, bar_height)),
            &bar_color,
            &[]
        ).unwrap();

        // Draw the bar text
        let mut x = 0;
        for (i, _) in state.workspaces.iter().enumerate() {
            let tag_text = format!(" {} ", i + 1);
            let color = if i == state.active_workspace {
                [1.0, 1.0, 1.0, 1.0] // white for active
            } else {
                [0.5, 0.5, 0.5, 1.0] // gray for inactive
            };
            state.theme.render_text(
                &mut frame,
                &tag_text,
                (x, 0).into(),
                Scale::from(1.0),
                color,
                &[]
            ).unwrap();
            x += state.theme.text_width(&tag_text) as i32;
        }

        // Draw layout symbol
        let layout_symbol = state.workspaces[state.active_workspace].layout_symbol();
        let color = [1.0, 1.0, 1.0, 1.0];
        state.theme.render_text(
            &mut frame,
            &layout_symbol,
            (x, 0).into(),
            Scale::from(1.0),
            color,
            &[]
        ).unwrap();
        x += state.theme.text_width(&layout_symbol) as i32;

        // Draw window titles
        for window in state.workspaces[state.active_workspace].windows() {
            if let Some(title) = window.surface.title() {
                let color = [1.0, 1.0, 1.0, 1.0];
                state.theme.render_text(
                    &mut frame,
                    &title,
                    (x, 0).into(),
                    Scale::from(1.0),
                    color,
                    &[]
                ).unwrap();
                x += state.theme.text_width(&title) as i32;
            }
        }

        for window in state.workspaces[state.active_workspace].windows() {
            let surface = window.surface.get_surface().unwrap();
            render_surface_tree(
                &mut frame,
                surface,
                window.geometry.loc,
                1.0, // scale
                1.0, // alpha
                &mut |_surface, _location| {}
            ).unwrap();
        }

        frame.finish().unwrap();

        // Dispatch Wayland events
        display.flush_clients().unwrap();
        event_loop.dispatch(Some(std::time::Duration::from_millis(16)), &mut state).unwrap();
    }
}

impl State {
    fn new(
        display_handle: DisplayHandle,
        handle: LoopHandle<'static, Self>,
        size: smithay::utils::Size<i32, smithay::utils::Raw>,
        compositor_state: CompositorState,
        shm_state: ShmState,
        xdg_shell_state: XdgShellState,
        seat_state: SeatState<Self>,
        seat: Seat<Self>,
        theme: AdwaitaTheme,
    ) -> Self {
        let physical_properties = output::PhysicalProperties {
            size,
            subpixel: output::Subpixel::Unknown,
            make: "winit".into(),
            model: "winit".into(),
        };
        let output = output::Output::new("winit".to_string(), physical_properties);

        let mut workspaces = Vec::new();
        for _ in 0..9 {
            workspaces.push(Workspace::new());
        }

        Self {
            display_handle,
            handle,
            output,
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            seat,
            workspaces,
            active_workspace: 0,
            theme,
            modifiers: ModifiersState::empty(),
            drag_data: None,
            resize_data: None,
        }
    }

    fn process_input_event(&mut self, event: InputEvent) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let pointer = self.seat.get_pointer().unwrap();

        match event {
            InputEvent::Keyboard { event } => {
                let keysym = event.keysym();
                keyboard.input(
                    self,
                    event.key_code(),
                    event.state(),
                    smithay::utils::SERIAL_COUNTER.next_serial(),
                    event.time(),
                    |state, _, _| {
                        keybindings::handle_key_event(state, keysym, state.modifiers);
                        FilterResult::Forward
                    },
                );
            }
            InputEvent::PointerMotion { location, .. } => {
                pointer.motion(self, Some(location), smithay::utils::SERIAL_COUNTER.next_serial());

                if let Some((surface, start_pos)) = &self.resize_data {
                    let delta = location.to_i32_round() - *start_pos;
                    for window in &mut self.workspaces[self.active_workspace].windows_mut() {
                        if window.surface == *surface {
                            let new_size = (window.geometry.size.w + delta.x, window.geometry.size.h + delta.y);
                            window.geometry.size = new_size.into();
                            break;
                        }
                    }
                } else if let Some((surface, start_pos)) = &self.drag_data {
                    let delta = location.to_i32_round() - *start_pos;
                    for window in &mut self.workspaces[self.active_workspace].windows_mut() {
                        if window.surface == *surface {
                            window.geometry.loc += delta;
                            break;
                        }
                    }
                } else {
                    let mut found_window = None;
                    for window in &self.workspaces[self.active_workspace].windows() {
                        if window.geometry.contains(location.to_i32_round()) {
                            found_window = Some(window.surface.get_surface().unwrap().clone());
                            break;
                        }
                    }

                    if let Some(surface) = found_window {
                        keyboard.set_focus(self, Some(surface), smithay::utils::SERIAL_COUNTER.next_serial());
                    }
                }
            }
            InputEvent::PointerButton { button, state, .. } => {
                pointer.button(
                    self,
                    button,
                    state,
                    smithay::utils::SERIAL_COUNTER.next_serial(),
                );

                if state == ButtonState::Pressed {
                    if self.modifiers.contains(ModifiersState::SUPER) && self.modifiers.contains(ModifiersState::CTRL) {
                        let mut found_window = None;
                        for window in &self.workspaces[self.active_workspace].windows() {
                            if window.geometry.contains(pointer.current_location().to_i32_round()) {
                                found_window = Some(window.surface.clone());
                                break;
                            }
                        }
                        if let Some(surface) = found_window {
                            self.resize_data = Some((surface, pointer.current_location().to_i32_round()));
                        }
                    } else if self.modifiers.contains(ModifiersState::SUPER) {
                        let mut found_window = None;
                        for window in &self.workspaces[self.active_workspace].windows() {
                            if window.geometry.contains(pointer.current_location().to_i32_round()) {
                                found_window = Some(window.surface.clone());
                                break;
                            }
                        }
                        if let Some(surface) = found_window {
                            self.drag_data = Some((surface, pointer.current_location().to_i32_round()));
                        }
                    }
                } else {
                    self.drag_data = None;
                    self.resize_data = None;
                }
            }
            InputEvent::Modifiers { modifiers } => {
                self.modifiers = modifiers;
            }
            _ => {}
        }
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<CompositorClientState>().unwrap()
    }

    fn commit(&mut self, _surface: &WlSurface) {
        // Handle commit
    }
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        println!("New toplevel window created");
        let window = Window { surface, geometry: Rectangle::from_loc_and_size((0,0), (0,0)) };
        self.workspaces[self.active_workspace].add_window(window);
    }

    fn new_popup(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _data: &smithay::wayland::shell::xdg::XdgPopupSurfaceData,
    ) {
        // Popups are not handled for now
    }

    fn grab(
        &mut self,
        _surface: smithay::wayland::shell::xdg::PopupSurface,
        _seat: &WaylandSeat<Self>,
        _serial: smithay::utils::Serial,
    ) {
        // Popups are not handled for now
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {
        // handle focus change
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: smithay::wayland::seat::CursorImageStatus) {
        // handle cursor image change
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_seat!(State);
