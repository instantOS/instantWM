//! Verify the surface-to-window reverse lookup against Smithay's ownership
//! traversal. These tests use a real Wayland client so subsurface and popup
//! parent relationships are established by the protocol handlers.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use wayland_client::Proxy;
use wayland_client::protocol::{
    wl_callback, wl_compositor, wl_registry, wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::types::WindowId;

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

#[derive(Default)]
struct TestClient {
    globals: Vec<(u32, String)>,
    syncs: usize,
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

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
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

delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
delegate_noop!(TestClient: ignore wl_surface::WlSurface);
delegate_noop!(TestClient: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(TestClient: ignore wl_subsurface::WlSubsurface);
delegate_noop!(TestClient: ignore xdg_wm_base::XdgWmBase);
delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(TestClient: ignore xdg_popup::XdgPopup);
delegate_noop!(TestClient: ignore xdg_positioner::XdgPositioner);

struct Fixture {
    event_loop: EventLoop,
    state: WaylandState,
    conn: Connection,
    queue: EventQueue<TestClient>,
    client: TestClient,
    server_client: Client,
    compositor: wl_compositor::WlCompositor,
    subcompositor: wl_subcompositor::WlSubcompositor,
    xdg: xdg_wm_base::XdgWmBase,
}

impl Fixture {
    fn new() -> Self {
        let (event_loop, mut state) = crate::test_support::new_compositor();
        let (client_socket, server_socket) = UnixStream::pair().unwrap();
        let server_client = state
            .native
            .display_handle
            .insert_client(server_socket, Arc::new(WaylandClientState::default()))
            .unwrap();
        let conn = Connection::from_socket(client_socket).unwrap();
        let mut queue = conn.new_event_queue();
        let registry = conn.display().get_registry(&queue.handle(), ());
        let mut client = TestClient::default();
        let mut event_loop = event_loop;
        Self::pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
        let global = |interface: &str| {
            client
                .globals
                .iter()
                .find(|(_, name)| name == interface)
                .unwrap()
                .0
        };
        let compositor = registry.bind(global("wl_compositor"), 1, &queue.handle(), ());
        let subcompositor = registry.bind(global("wl_subcompositor"), 1, &queue.handle(), ());
        let xdg = registry.bind(global("xdg_wm_base"), 1, &queue.handle(), ());
        Self {
            event_loop,
            state,
            conn,
            queue,
            client,
            server_client,
            compositor,
            subcompositor,
            xdg,
        }
    }

    fn pump(
        event_loop: &mut EventLoop,
        state: &mut WaylandState,
        conn: &Connection,
        queue: &mut EventQueue<TestClient>,
        client: &mut TestClient,
    ) {
        let target = client.syncs + 1;
        conn.display().sync(&queue.handle(), ());
        conn.flush().unwrap();
        event_loop
            .dispatch(Some(Duration::from_millis(250)), state)
            .unwrap();
        state.dispatch_pending_commits();
        state.native.display_handle.flush_clients().unwrap();
        while client.syncs < target {
            queue.blocking_dispatch(client).unwrap();
        }
    }

    fn pump_state(&mut self) {
        Self::pump(
            &mut self.event_loop,
            &mut self.state,
            &self.conn,
            &mut self.queue,
            &mut self.client,
        );
    }

    fn surface(&self) -> wl_surface::WlSurface {
        self.compositor.create_surface(&self.queue.handle(), ())
    }

    fn toplevel(
        &mut self,
    ) -> (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    ) {
        let surface = self.surface();
        let xdg = self.xdg.get_xdg_surface(&surface, &self.queue.handle(), ());
        let toplevel = xdg.get_toplevel(&self.queue.handle(), ());
        surface.commit();
        self.pump_state();
        (surface, xdg, toplevel)
    }

    fn register_toplevel(&mut self, surface: &wl_surface::WlSurface) -> WindowId {
        let server_surface = self.server_surface(surface);
        let position = self
            .state
            .native
            .runtime
            .pending_toplevels
            .iter()
            .position(|toplevel| toplevel.wl_surface() == &server_surface)
            .expect("toplevel was not pending");
        let toplevel = self.state.native.runtime.pending_toplevels.remove(position);
        self.state.setup_managed_window(toplevel)
    }

    fn subsurface(&mut self, parent: &wl_surface::WlSurface) -> wl_surface::WlSurface {
        let child = self.surface();
        self.subcompositor
            .get_subsurface(&child, parent, &self.queue.handle(), ());
        self.pump_state();
        child
    }

    fn popup(
        &mut self,
        parent: &xdg_surface::XdgSurface,
    ) -> (
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_popup::XdgPopup,
    ) {
        let surface = self.surface();
        let xdg = self.xdg.get_xdg_surface(&surface, &self.queue.handle(), ());
        let positioner = self.xdg.create_positioner(&self.queue.handle(), ());
        positioner.set_size(60, 60);
        positioner.set_anchor_rect(10, 10, 10, 10);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        let popup = xdg.get_popup(Some(parent), &positioner, &self.queue.handle(), ());
        positioner.destroy();
        self.pump_state();
        (surface, xdg, popup)
    }

    fn server_surface(&self, surface: &wl_surface::WlSurface) -> ServerSurface {
        self.server_client
            .object_from_protocol_id(
                &self.state.native.display_handle,
                surface.id().protocol_id(),
            )
            .expect("client surface is not present on the server")
    }

    fn lookup(&self, surface: &wl_surface::WlSurface) -> Option<WindowId> {
        self.lookup_server(&self.server_surface(surface))
    }

    fn lookup_server(&self, surface: &ServerSurface) -> Option<WindowId> {
        self.state.native.window_id_for_surface(surface)
    }
}

#[test]
fn window_lookup_matches_smithay_surface_ownership() {
    let mut fixture = Fixture::new();
    let (toplevel, toplevel_xdg, _) = fixture.toplevel();
    let window = fixture.register_toplevel(&toplevel);

    let child = fixture.subsurface(&toplevel);
    let grandchild = fixture.subsurface(&child);
    let (popup, popup_xdg, _) = fixture.popup(&toplevel_xdg);
    let (nested_popup, _, _) = fixture.popup(&popup_xdg);
    let popup_child = fixture.subsurface(&popup);

    // The reference traversal and the reverse lookup must agree for every
    // surface the window actually owns.
    let element = fixture
        .state
        .native
        .window_index
        .get(&window)
        .cloned()
        .expect("registered window missing from index");
    // with_surfaces holds the current surface's private-data lock while the
    // callback runs, so collect first and resolve afterwards.
    let mut owned = Vec::new();
    element.with_surfaces(|surface, _| owned.push(surface.clone()));
    assert_eq!(
        owned.len(),
        6,
        "expected toplevel, two subsurfaces, popup, nested popup, popup subsurface"
    );
    for surface in &owned {
        assert_eq!(
            fixture.lookup_server(surface),
            Some(window),
            "lookup disagreed for owned surface {surface:?}",
        );
    }

    for surface in [
        &toplevel,
        &child,
        &grandchild,
        &popup,
        &nested_popup,
        &popup_child,
    ] {
        assert_eq!(fixture.lookup(surface), Some(window));
    }

    // Surfaces that are not in any managed window's tree must stay unknown.
    let standalone = fixture.surface();
    fixture.pump_state();
    assert_eq!(fixture.lookup(&standalone), None);

    // A toplevel becomes owned only when it is registered as a managed window.
    let (pending, _, _) = fixture.toplevel();
    assert_eq!(fixture.lookup(&pending), None);
    let pending_child = fixture.subsurface(&pending);
    assert_eq!(fixture.lookup(&pending_child), None);
}

#[test]
fn window_lookup_distinguishes_windows_with_similar_trees() {
    let mut fixture = Fixture::new();
    let (first, first_xdg, _) = fixture.toplevel();
    let first_window = fixture.register_toplevel(&first);
    let first_child = fixture.subsurface(&first);

    let (second, second_xdg, _) = fixture.toplevel();
    let second_window = fixture.register_toplevel(&second);
    let second_child = fixture.subsurface(&second);
    let (second_popup, _, _) = fixture.popup(&second_xdg);
    let (first_popup, _, _) = fixture.popup(&first_xdg);

    for surface in [&first, &first_child, &first_popup] {
        assert_eq!(fixture.lookup(surface), Some(first_window));
    }
    for surface in [&second, &second_child, &second_popup] {
        assert_eq!(fixture.lookup(surface), Some(second_window));
    }
}
