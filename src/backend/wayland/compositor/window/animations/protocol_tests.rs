//! Exercise geometry intent through native xdg configure dispatch. Observed
//! sizes are supplied explicitly, so no renderer or video decoder is needed.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use wayland_client::protocol::{wl_callback, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::backend::Backend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::client::geometry::FloatingPlacementIntent;
use crate::floating::{WindowModeRequest, set_window_mode};
use crate::test_support::{MonitorBuilder, add_client};
use crate::types::{Client, ClientMode, Rect, WindowId};
use crate::wm::Wm;

#[derive(Default)]
struct NativeClient {
    globals: Vec<(u32, String)>,
    sync_count: usize,
    serials: Vec<u32>,
    sizes: Vec<(i32, i32)>,
}

impl Dispatch<wl_callback::WlCallback, ()> for NativeClient {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.sync_count += 1;
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for NativeClient {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for NativeClient {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.serials.push(serial);
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for NativeClient {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.sizes.push((width, height));
        }
    }
}

delegate_noop!(NativeClient: ignore wl_compositor::WlCompositor);
delegate_noop!(NativeClient: ignore wl_surface::WlSurface);
delegate_noop!(NativeClient: ignore xdg_wm_base::XdgWmBase);

fn pump(
    event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
    state: &mut WaylandState,
    conn: &Connection,
    queue: &mut EventQueue<NativeClient>,
    client: &mut NativeClient,
) {
    let next_sync = client.sync_count + 1;
    conn.display().sync(&queue.handle(), ());
    conn.flush().unwrap();
    event_loop
        .dispatch(Some(Duration::from_millis(250)), state)
        .unwrap();
    state.display_handle.flush_clients().unwrap();
    while client.sync_count < next_sync {
        queue.blocking_dispatch(client).unwrap();
    }
}

fn connect_native_window(
    event_loop: &mut smithay::reexports::calloop::EventLoop<'static, WaylandState>,
    state: &mut WaylandState,
) -> (Connection, EventQueue<NativeClient>, NativeClient, WindowId) {
    let (client_socket, server_socket) = UnixStream::pair().unwrap();
    state
        .display_handle
        .insert_client(server_socket, Arc::new(WaylandClientState::default()))
        .unwrap();
    let conn = Connection::from_socket(client_socket).unwrap();
    let mut queue = conn.new_event_queue::<NativeClient>();
    let mut client = NativeClient::default();
    let registry = conn.display().get_registry(&queue.handle(), ());
    pump(event_loop, state, &conn, &mut queue, &mut client);
    let global = |interface: &str| {
        client
            .globals
            .iter()
            .find(|(_, name)| name == interface)
            .unwrap()
            .0
    };
    let compositor: wl_compositor::WlCompositor =
        registry.bind(global("wl_compositor"), 1, &queue.handle(), ());
    let shell: xdg_wm_base::XdgWmBase =
        registry.bind(global("xdg_wm_base"), 1, &queue.handle(), ());
    let surface = compositor.create_surface(&queue.handle(), ());
    let xdg = shell.get_xdg_surface(&surface, &queue.handle(), ());
    let _toplevel = xdg.get_toplevel(&queue.handle(), ());
    surface.commit();
    pump(event_loop, state, &conn, &mut queue, &mut client);

    // Register without a buffer so this test can exercise real configure
    // dispatch independently of a renderer. Model geometry supplies the
    // intended tiled and floating rectangles.
    let native = state.runtime.pending_toplevels.pop().unwrap();
    let win = state.setup_managed_window(native);
    (conn, queue, client, win)
}

#[test]
fn native_restore_schedules_before_dispatch_and_converges_to_constrained_size() {
    let (mut event_loop, mut state) =
        crate::backend::wayland::compositor::new_event_loop_and_state();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);
    let backend = WaylandBackend::new();
    backend.attach_state(&mut state);
    let mut wm = Wm::new(Backend::new_wayland(backend));
    wm.core.config.animations.enabled = true;
    let monitor = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .bar(0, false)
            .build(),
    );
    let tiled = Rect::new(0, 0, 1920, 1080);
    let floating = Rect::new(900, 500, 640, 360);
    let mut model_client = Client {
        win,
        geo: tiled,
        ..Client::default()
    };
    model_client.save_floating_placement(floating, tiled);
    add_client(&mut wm.core.model, monitor, model_client);
    state.attach_wm(&mut wm);
    state.map_window_in_space(win);
    state.resize_window(win, tiled);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    let tiled_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    let tiled_configured_count = client.sizes.len();

    let _ = set_window_mode(
        &mut wm.ctx(),
        win,
        WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter),
    );
    assert_eq!(wm.core.model.client(win).unwrap().geo, floating);
    assert_eq!(
        state.geometry_sync.get(&win).unwrap().scheduled_size(),
        Some(floating.size())
    );
    assert!(!state.native_commit_may_update_model(win, 1920, 1080, Some(tiled_serial), true));
    assert_eq!(
        wm.core.model.client(win).unwrap().saved_floating_rect(),
        Some(floating)
    );
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    // Leaving tiled placement also sends the unmaximize state, carrying
    // the previous size until the animation dispatches the floating resize.
    assert_eq!(client.sizes.len(), tiled_configured_count + 1);
    assert_eq!(client.sizes.last(), Some(&(1920, 1080)));
    let unmaximize_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(!state.native_commit_may_update_model(win, 1920, 1080, Some(unmaximize_serial), true));
    let configured_count = client.sizes.len();

    // An interruption delivers the scheduled resize once, through the same
    // dispatcher used by normal animation ticks and immediate resizes.
    state.cancel_window_animation(win);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.last(), Some(&(640, 360)));
    assert_eq!(client.sizes.len(), configured_count + 1);
    let restore_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(!state.native_commit_may_update_model(win, 1920, 1080, Some(tiled_serial), true));
    assert!(state.native_commit_may_update_model(win, 640, 352, Some(restore_serial), true));
    let constrained = Rect::new(floating.x, floating.y, 640, 352);
    wm.core.model.sync_client_geometry(win, constrained);
    assert_eq!(
        wm.core.model.client(win).unwrap().saved_floating_rect(),
        Some(constrained)
    );
    state.resize_window(win, constrained);
    state.resize_window(win, constrained);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.last(), Some(&(640, 352)));
    assert_eq!(client.sizes.len(), configured_count + 2);
    let convergence_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(state.native_commit_may_update_model(win, 640, 352, Some(convergence_serial), true));
    assert!(!state.native_commit_may_update_model(win, 1920, 1080, Some(tiled_serial), true));

    // Retarget an unsent resize, then request the final size immediately while
    // preserving its spatial animation. Only the final intent may be sent.
    for size in [(480, 270), (320, 180)] {
        let target = Rect::new(floating.x, floating.y, size.0, size.1);
        wm.core.model.sync_client_geometry(win, target);
        state.set_window_target_rect(
            win,
            target,
            super::WindowMoveMode::AnimateFrom {
                from: constrained,
                duration: Duration::from_millis(500),
            },
        );
    }
    let target = wm.core.model.client(win).unwrap().geo;
    state.resize_window(win, target);
    assert!(state.animation_targets_outer_rect(win, target));
    assert_eq!(
        state.geometry_sync.get(&win).unwrap().scheduled_size(),
        None
    );
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.last(), Some(&(320, 180)));
    assert_eq!(client.sizes.len(), configured_count + 3);
    state.cancel_window_animation(win);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 3);
}

#[test]
fn hidden_resizes_dispatch_without_remapping_and_drops_do_not_send_obsolete_intent() {
    let (mut event_loop, mut state) =
        crate::backend::wayland::compositor::new_event_loop_and_state();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);
    let backend = WaylandBackend::new();
    backend.attach_state(&mut state);
    let mut wm = Wm::new(Backend::new_wayland(backend));
    wm.core.config.animations.enabled = true;
    let monitor = wm.core.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    let initial = Rect::new(100, 100, 800, 600);
    add_client(
        &mut wm.core.model,
        monitor,
        Client {
            win,
            geo: initial,
            mode: ClientMode::floating(),
            ..Client::default()
        },
    );
    state.attach_wm(&mut wm);
    state.map_window_in_space(win);
    state.resize_window(win, initial);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    let configured_count = client.sizes.len();
    let element = state.find_window(win).unwrap().clone();

    let target = Rect::new(100, 100, 640, 480);
    wm.core.model.sync_client_geometry(win, target);
    state.set_window_target_rect(
        win,
        target,
        super::WindowMoveMode::AnimateFrom {
            from: initial,
            duration: Duration::from_millis(500),
        },
    );
    assert!(state.window_has_active_animation(win));
    state.unmap_window_from_space(win);
    assert_eq!(state.space.element_location(&element), None);
    assert!(!state.window_has_active_animation(win));
    assert_eq!(
        state.geometry_sync.get(&win).unwrap().scheduled_size(),
        None
    );
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 1);
    assert_eq!(client.sizes.last(), Some(&(640, 480)));
    let serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(state.native_commit_may_update_model(win, 640, 480, Some(serial), true));

    // An animated resize requested while already hidden has no frame timer.
    // Its protocol request must nevertheless progress immediately.
    let hidden_target = Rect::new(100, 100, 480, 360);
    wm.core.model.sync_client_geometry(win, hidden_target);
    state.set_window_target_rect(
        win,
        hidden_target,
        super::WindowMoveMode::Retarget {
            duration: Duration::from_millis(500),
        },
    );
    assert_eq!(state.space.element_location(&element), None);
    assert!(!state.window_has_active_animation(win));
    assert_eq!(
        state.geometry_sync.get(&win).unwrap().scheduled_size(),
        None
    );
    state.unmap_window_from_space(win);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 2);
    assert_eq!(client.sizes.last(), Some(&(480, 360)));
    let serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(state.native_commit_may_update_model(win, 480, 360, Some(serial), true));

    state.map_window_in_space(win);
    let obsolete = Rect::new(100, 100, 400, 300);
    wm.core.model.sync_client_geometry(win, obsolete);
    state.set_window_target_rect(
        win,
        obsolete,
        super::WindowMoveMode::AnimateFrom {
            from: hidden_target,
            duration: Duration::from_millis(500),
        },
    );
    state.drop_window_animation(win);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 2);
    let replacement = Rect::new(100, 100, 320, 240);
    wm.core.model.sync_client_geometry(win, replacement);
    state.set_window_target_rect(
        win,
        replacement,
        super::WindowMoveMode::AnimateFrom {
            from: hidden_target,
            duration: Duration::from_millis(500),
        },
    );
    state.cancel_window_animation(win);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 3);
    assert_eq!(client.sizes.last(), Some(&(320, 240)));

    // End-of-surface cleanup discards pending intent without configuring it.
    wm.core.model.sync_client_geometry(win, obsolete);
    state.set_window_target_rect(
        win,
        obsolete,
        super::WindowMoveMode::AnimateFrom {
            from: replacement,
            duration: Duration::from_millis(500),
        },
    );
    state.remove_window_tracking(win);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 3);
    assert!(!state.geometry_sync.contains_key(&win));
}
