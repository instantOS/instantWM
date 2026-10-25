//! Real pointer constraints and keyboard grabs without a GPU or running desktop.

use crate::backend::wayland::commands::PointerMotionCommand;
use crate::backend::wayland::compositor::KeyboardFocusTarget;
use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::wayland::input::pointer::motion::process_pointer_motion_command;
use crate::test_support::{MonitorBuilder, add_client};
use crate::types::{Client, ClientMode, MonitorId, Rect, Size, TagMask, WindowId};
use smithay::backend::input::InputTime;
use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::{GrabStartData, KeyboardGrab, KeyboardInnerHandle, ModifiersState};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::utils::Serial;
use smithay::utils::{Point, SERIAL_COUNTER};
use smithay::wayland::pointer_constraints::with_pointer_constraint;
use smithay::wayland::seat::WaylandFocus;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::wp::pointer_constraints::zv1::client::{
    zwp_locked_pointer_v1, zwp_pointer_constraints_v1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::{
    zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

#[derive(Default)]
struct ClientEvents {
    globals: Vec<(u32, String)>,
    syncs: usize,
    enters: usize,
    locks: usize,
    unlocks: usize,
    relative: usize,
    show_cursor: bool,
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
impl Dispatch<wl_pointer::WlPointer, ()> for ClientEvents {
    fn event(
        state: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Enter { serial, .. } = event {
            state.enters += 1;
            if !state.show_cursor {
                pointer.set_cursor(serial, None, 0, 0);
            }
        }
    }
}
impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for ClientEvents {
    fn event(
        state: &mut Self,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        event: zwp_locked_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => state.locks += 1,
            zwp_locked_pointer_v1::Event::Unlocked => state.unlocks += 1,
            _ => {}
        }
    }
}
impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for ClientEvents {
    fn event(
        state: &mut Self,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        _: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.relative += 1;
    }
}
delegate_noop!(ClientEvents: ignore wl_compositor::WlCompositor);
delegate_noop!(ClientEvents: ignore wl_surface::WlSurface);
delegate_noop!(ClientEvents: ignore wl_shm::WlShm);
delegate_noop!(ClientEvents: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ClientEvents: ignore wl_buffer::WlBuffer);
delegate_noop!(ClientEvents: ignore wl_output::WlOutput);
delegate_noop!(ClientEvents: ignore wl_seat::WlSeat);
delegate_noop!(ClientEvents: ignore xdg_wm_base::XdgWmBase);
impl Dispatch<xdg_surface::XdgSurface, ()> for ClientEvents {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}
delegate_noop!(ClientEvents: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(ClientEvents: ignore zwp_pointer_constraints_v1::ZwpPointerConstraintsV1);
delegate_noop!(ClientEvents: ignore zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1);

struct Fixture {
    event_loop: EventLoop,
    state: WaylandState,
    conn: Connection,
    queue: EventQueue<ClientEvents>,
    events: ClientEvents,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    pointer: wl_pointer::WlPointer,
    constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
    _relative: zwp_relative_pointer_v1::ZwpRelativePointerV1,
    xdg: xdg_wm_base::XdgWmBase,
    outputs: Vec<(Output, wl_output::WlOutput, MonitorId)>,
}

impl Fixture {
    fn new(output_count: usize) -> Self {
        let (event_loop, mut state) = crate::test_support::new_compositor();
        state.wm.core.state.config.animations.enabled = false;
        let mut outputs = Vec::new();
        for index in 0..output_count {
            let name = format!("test-{index}");
            let x = index as i32 * 200;
            let monitor = state.wm.core.state.model.monitors.push(
                MonitorBuilder::new()
                    .named(&name)
                    .monitor_rect(Rect::new(x, 0, 200, 200))
                    .bar(0, false)
                    .tag_count(2)
                    .selected_tags(TagMask::single(1).unwrap())
                    .build(),
            );
            let output = state.native.create_output(&name, Size::new(200, 200), None);
            state.native.space.map_output(&output, (x, 0));
            outputs.push((output, monitor));
        }
        state
            .wm
            .core
            .state
            .model
            .monitors
            .set_selected(outputs[0].1);
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
        let mut event_loop = event_loop;
        Self::roundtrip(&mut event_loop, &mut state, &conn, &mut queue, &mut events);
        let global = |interface: &str| {
            events
                .globals
                .iter()
                .find(|(_, name)| name == interface)
                .unwrap()
                .0
        };
        let compositor = registry.bind(global("wl_compositor"), 1, &queue.handle(), ());
        let shm = registry.bind(global("wl_shm"), 1, &queue.handle(), ());
        let seat: wl_seat::WlSeat = registry.bind(global("wl_seat"), 1, &queue.handle(), ());
        let pointer = seat.get_pointer(&queue.handle(), ());
        let constraints =
            registry.bind(global("zwp_pointer_constraints_v1"), 1, &queue.handle(), ());
        let manager: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1 = registry.bind(
            global("zwp_relative_pointer_manager_v1"),
            1,
            &queue.handle(),
            (),
        );
        let relative = manager.get_relative_pointer(&pointer, &queue.handle(), ());
        let xdg = registry.bind(global("xdg_wm_base"), 1, &queue.handle(), ());
        let globals: Vec<_> = events
            .globals
            .iter()
            .filter(|(_, name)| name == "wl_output")
            .map(|(id, _)| *id)
            .collect();
        let outputs = outputs
            .into_iter()
            .zip(globals)
            .map(|((output, monitor), global)| {
                (
                    output,
                    registry.bind(global, 1, &queue.handle(), ()),
                    monitor,
                )
            })
            .collect();
        Self {
            event_loop,
            state,
            conn,
            queue,
            events,
            compositor,
            shm,
            pointer,
            constraints,
            _relative: relative,
            xdg,
            outputs,
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
    fn attach(&mut self, surface: &wl_surface::WlSurface) {
        let file = tempfile::tempfile().unwrap();
        file.set_len(200 * 200 * 4).unwrap();
        let pool = self
            .shm
            .create_pool(file.as_fd(), 200 * 200 * 4, &self.queue.handle(), ());
        let buffer = pool.create_buffer(
            0,
            200,
            200,
            200 * 4,
            wl_shm::Format::Argb8888,
            &self.queue.handle(),
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        self.pump();
        buffer.destroy();
        pool.destroy();
    }
    fn window(&mut self) -> (WindowId, wl_surface::WlSurface) {
        let surface = self.compositor.create_surface(&self.queue.handle(), ());
        let xdg = self.xdg.get_xdg_surface(&surface, &self.queue.handle(), ());
        let _toplevel = xdg.get_toplevel(&self.queue.handle(), ());
        surface.commit();
        self.pump();
        let native = self.state.native.runtime.pending_toplevels.pop().unwrap();
        let win = self.state.setup_managed_window(native);
        add_client(
            &mut self.state.wm.core.state.model,
            self.outputs[0].2,
            Client {
                win,
                geo: Rect::new(0, 0, 200, 200),
                tags: TagMask::single(1).unwrap(),
                ..Client::default()
            },
        );
        self.state.map_window_in_space(win);
        self.attach(&surface);
        (win, surface)
    }
}

impl Fixture {
    fn motion(&mut self, command: PointerMotionCommand) {
        let pointer = self.state.native.seat.get_pointer().unwrap();
        let keyboard = self.state.native.seat.get_keyboard().unwrap();
        process_pointer_motion_command(&mut self.state, &pointer, &keyboard, command);
        self.pump();
        self.pump(); // process the client's set_cursor response to enter
    }
    fn refresh(&mut self) {
        self.motion(PointerMotionCommand::Refresh {
            time: InputTime::now(),
        });
    }
    fn relative(&mut self) {
        self.motion(PointerMotionCommand::Relative {
            dx: 1.0,
            dy: 0.0,
            dx_unaccel: 1.0,
            dy_unaccel: 0.0,
            time: InputTime::now(),
        });
    }
    fn lock(
        &mut self,
        surface: &wl_surface::WlSurface,
        lifetime: zwp_pointer_constraints_v1::Lifetime,
    ) -> zwp_locked_pointer_v1::ZwpLockedPointerV1 {
        let lock = self.constraints.lock_pointer(
            surface,
            &self.pointer,
            None,
            lifetime,
            &self.queue.handle(),
            (),
        );
        self.pump();
        lock
    }
}

#[test]
fn game_lock_recovers_after_leaving_and_returning_to_a_tag() {
    let mut f = Fixture::new(1);
    let (win, surface) = f.window();
    f.state.native.runtime.pointer_location = Point::from((100.0, 100.0));
    f.refresh();
    let _lock = f.lock(&surface, zwp_pointer_constraints_v1::Lifetime::Persistent);
    assert_eq!(f.events.locks, 1);
    assert_eq!(
        f.state.native.cursor_image_status,
        CursorImageStatus::Hidden
    );
    f.relative();
    let before = f.events.relative;
    assert!(before > 0);
    let monitor = f.outputs[0].2;
    f.state
        .wm
        .core
        .state
        .model
        .monitor_mut(monitor)
        .unwrap()
        .set_selected_tags(TagMask::single(2).unwrap());
    f.state.ctx().apply_visibility_plan();
    f.state.sync_space();
    f.refresh();
    assert_eq!(f.events.unlocks, 1);
    assert!(
        f.state
            .native
            .seat
            .get_pointer()
            .unwrap()
            .current_focus()
            .is_none()
    );
    f.state
        .wm
        .core
        .state
        .model
        .monitor_mut(monitor)
        .unwrap()
        .set_selected_tags(TagMask::single(1).unwrap());
    f.state.ctx().apply_visibility_plan();
    f.state.sync_space();
    f.state.focus_window(win, None);
    let root = f
        .state
        .native
        .find_window(win)
        .unwrap()
        .wl_surface()
        .unwrap()
        .into_owned();
    let pointer = f.state.native.seat.get_pointer().unwrap();
    assert!(
        !with_pointer_constraint(&root, &pointer, |c| c.unwrap().is_active()),
        "keyboard focus must not activate a lock before pointer enter"
    );
    // The very first device event must restore enter, activation, and relative
    // delivery. A subsequent refresh cannot be required to unstrand the lock.
    f.relative();
    assert_eq!(f.events.enters, 2);
    assert_eq!(f.events.locks, 2);
    assert_eq!(f.events.relative, before + 1);
    assert_eq!(
        f.state.native.cursor_image_status,
        CursorImageStatus::Hidden
    );
    assert!(pointer.current_focus().is_some());
    let location = f.state.native.runtime.pointer_location;
    f.relative();
    assert_eq!(f.state.native.runtime.pointer_location, location);
    assert_eq!(f.events.relative, before + 2);
}

#[test]
fn a_constraint_on_another_surface_of_the_same_client_waits_for_pointer_enter() {
    let mut f = Fixture::new(1);
    let (first, _) = f.window();
    let (second, surface) = f.window();
    let second_window = f.state.native.find_window(second).unwrap().clone();
    // Both surfaces stay mapped to exercise the same-client activation gate;
    // XWayland's distinct X11 windows all share one Wayland connection.
    f.state.wm.core.state.model.client_mut(second).unwrap().geo = Rect::new(250, 0, 200, 200);
    f.state
        .native
        .space
        .map_element(second_window, (250, 0), false);
    f.state.native.runtime.pointer_location = Point::from((100.0, 100.0));
    f.refresh();
    let _lock = f.lock(&surface, zwp_pointer_constraints_v1::Lifetime::Persistent);
    assert_eq!(f.events.locks, 0);
    f.state.focus_window(second, None);
    f.pump();
    assert_eq!(f.events.locks, 0);
    assert!(f.state.is_seat_focused_on(second));
    assert_ne!(first, second);
}

/// Models the XWayland grab: reject focus changes and re-anchor the keyboard
/// to the grab owner on the next key event.
struct StickyGrab(GrabStartData<WaylandState>);
impl KeyboardGrab<WaylandState> for StickyGrab {
    fn input(
        &mut self,
        state: &mut WaylandState,
        handle: &mut KeyboardInnerHandle<'_, WaylandState>,
        key: Keycode,
        pressed: KeyState,
        modifiers: Option<ModifiersState>,
        serial: Serial,
        time: InputTime,
    ) {
        handle.set_focus(state, self.0.focus.clone(), serial);
        handle.input(state, key, pressed, modifiers, serial, time);
    }
    fn set_focus(
        &mut self,
        _: &mut WaylandState,
        _: &mut KeyboardInnerHandle<'_, WaylandState>,
        _: Option<KeyboardFocusTarget>,
        _: Serial,
    ) {
    }
    fn start_data(&self) -> &GrabStartData<WaylandState> {
        &self.0
    }
    fn unset(&mut self, _: &mut WaylandState) {}
}

impl Fixture {
    fn grab_keyboard(&mut self, win: WindowId) {
        self.state.focus_window(win, None);
        let keyboard = self.state.native.seat.get_keyboard().unwrap();
        keyboard.set_grab(
            &mut self.state,
            StickyGrab(GrabStartData {
                focus: keyboard.current_focus(),
            }),
            SERIAL_COUNTER.next_serial(),
        );
        assert!(keyboard.is_grabbed());
    }
    fn key(&mut self) {
        let keyboard = self.state.native.seat.get_keyboard().unwrap();
        keyboard.input::<(), _>(
            &mut self.state,
            Keycode::from(38u32),
            KeyState::Pressed,
            SERIAL_COUNTER.next_serial(),
            InputTime::now(),
            |_, _, _| smithay::input::keyboard::FilterResult::Forward,
        );
    }
}

#[test]
fn leaving_a_game_for_an_empty_tag_cancels_its_keyboard_grab() {
    let mut f = Fixture::new(1);
    let (game, _) = f.window();
    f.grab_keyboard(game);
    f.state.clear_seat_focus();
    let keyboard = f.state.native.seat.get_keyboard().unwrap();
    assert!(!keyboard.is_grabbed());
    assert!(keyboard.current_focus().is_none());
    f.key();
    assert!(
        keyboard.current_focus().is_none(),
        "the next key must not restore the hidden game"
    );
    f.state.focus_window(game, None);
    f.key();
    assert!(f.state.is_seat_focused_on(game));
}

#[test]
fn a_floating_overlay_can_take_the_keyboard_from_a_grabbed_game() {
    let mut f = Fixture::new(1);
    let (game, _) = f.window();
    let (dialog, _) = f.window();
    f.state
        .wm
        .core
        .state
        .model
        .client_mut(dialog)
        .unwrap()
        .set_mode_for_test(ClientMode::floating());
    f.grab_keyboard(game);
    let target = KeyboardFocusTarget::Window(f.state.native.find_window(dialog).unwrap().clone());
    f.state
        .set_keyboard_focus(Some(target), SERIAL_COUNTER.next_serial());
    f.key();
    assert!(f.state.is_seat_focused_on(dialog));
    f.state.focus_window(game, None);
    f.key();
    assert!(f.state.is_seat_focused_on(game));
}

#[test]
fn popup_navigation_preserves_the_existing_grab() {
    let mut f = Fixture::new(1);
    let (game, _) = f.window();
    let (popup, _) = f.window();
    f.grab_keyboard(game);
    let target = KeyboardFocusTarget::Window(f.state.native.find_window(popup).unwrap().clone());
    f.state
        .set_popup_keyboard_focus(Some(target), SERIAL_COUNTER.next_serial());
    assert!(f.state.native.seat.get_keyboard().unwrap().is_grabbed());
}

#[test]
fn a_hidden_game_cursor_does_not_leak_into_a_floating_window() {
    let mut f = Fixture::new(1);
    let (_, surface) = f.window();
    f.state.native.runtime.pointer_location = Point::from((100.0, 100.0));
    f.refresh();
    let _lock = f.lock(&surface, zwp_pointer_constraints_v1::Lifetime::Persistent);
    assert_eq!(
        f.state.native.cursor_image_status,
        CursorImageStatus::Hidden
    );
    f.events.show_cursor = true;
    let (dialog, _) = f.window();
    f.state
        .wm
        .core
        .state
        .model
        .client_mut(dialog)
        .unwrap()
        .set_mode_for_test(ClientMode::floating());
    f.state
        .native
        .space
        .raise_element(&f.state.native.find_window(dialog).unwrap().clone(), false);
    f.refresh();
    assert_eq!(f.events.unlocks, 1);
    assert_eq!(
        f.state.native.cursor_image_status,
        CursorImageStatus::default_named()
    );
}
