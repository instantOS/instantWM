//! Exercise geometry intent through native xdg configure dispatch. Observed
//! sizes are supplied explicitly, so no renderer or video decoder is needed.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::client::geometry::FloatingPlacementIntent;
use crate::floating::{WindowModeRequest, set_window_mode};
use crate::test_support::{MonitorBuilder, add_client};
use crate::types::{Client, ClientMode, Rect, WindowId};
use crate::wm::WaylandWm as Wm;

#[derive(Default)]
struct NativeClient {
    globals: Vec<(u32, String)>,
    sync_count: usize,
    serials: Vec<u32>,
    sizes: Vec<(i32, i32)>,
    states: Vec<Vec<u32>>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    surface: Option<wl_surface::WlSurface>,
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
        if let xdg_toplevel::Event::Configure {
            width,
            height,
            states,
        } = event
        {
            state.sizes.push((width, height));
            state.states.push(
                states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .map(u32::from_ne_bytes)
                    .collect(),
            );
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
    state.dispatch_pending_commits();
    state.native.display_handle.flush_clients().unwrap();
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
        .native
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
    client.toplevel = Some(xdg.get_toplevel(&queue.handle(), ()));
    client.surface = Some(surface.clone());
    surface.commit();
    pump(event_loop, state, &conn, &mut queue, &mut client);

    // Register without a buffer so this test can exercise real configure
    // dispatch independently of a renderer. Model geometry supplies the
    // intended tiled and floating rectangles.
    let native = state.native.runtime.pending_toplevels.pop().unwrap();
    let win = state.setup_managed_window(native);
    (conn, queue, client, win)
}

#[test]
fn native_restore_schedules_before_dispatch_and_converges_to_constrained_size() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);

    state.wm.core.state.config.animations.enabled = true;
    let monitor = state.wm.core.state.model.monitors.push(
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
    add_client(&mut state.wm.core.state.model, monitor, model_client);
    state.map_window_in_space(win);
    state.native.resize_window(&state.wm.core.state, win, tiled);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    let tiled_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    let tiled_configured_count = client.sizes.len();

    let _ = set_window_mode(
        &mut state.ctx(),
        win,
        WindowModeRequest::Floating(FloatingPlacementIntent::RestoreOrCenter),
    );
    assert_eq!(state.wm.core.state.model.client(win).unwrap().geo, floating);
    assert_eq!(
        state
            .native
            .geometry_sync
            .get(&win)
            .unwrap()
            .scheduled_size(),
        Some(floating.size())
    );
    assert!(!state.native.native_commit_may_update_model(
        win,
        1920,
        1080,
        Some(tiled_serial),
        true
    ));
    assert_eq!(
        state
            .wm
            .core
            .state
            .model
            .client(win)
            .unwrap()
            .saved_floating_rect(),
        Some(floating)
    );

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    // Leaving tiled placement also sends the unmaximize state, carrying
    // the previous size until the animation dispatches the floating resize.
    assert_eq!(client.sizes.len(), tiled_configured_count + 1);
    assert_eq!(client.sizes.last(), Some(&(1920, 1080)));
    let unmaximize_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(!state.native.native_commit_may_update_model(
        win,
        1920,
        1080,
        Some(unmaximize_serial),
        true
    ));
    let configured_count = client.sizes.len();

    // An interruption delivers the scheduled resize once, through the same
    // dispatcher used by normal animation ticks and immediate resizes.
    state
        .native
        .cancel_window_animation(&state.wm.core.state, win);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.last(), Some(&(640, 360)));
    assert_eq!(client.sizes.len(), configured_count + 1);
    let restore_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(!state.native.native_commit_may_update_model(
        win,
        1920,
        1080,
        Some(tiled_serial),
        true
    ));
    assert!(
        state
            .native
            .native_commit_may_update_model(win, 640, 352, Some(restore_serial), true)
    );
    let constrained = Rect::new(floating.x, floating.y, 640, 352);
    state
        .wm
        .core
        .state
        .model
        .sync_client_geometry(win, constrained);
    assert_eq!(
        state
            .wm
            .core
            .state
            .model
            .client(win)
            .unwrap()
            .saved_floating_rect(),
        Some(constrained)
    );
    state
        .native
        .resize_window(&state.wm.core.state, win, constrained);
    state
        .native
        .resize_window(&state.wm.core.state, win, constrained);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.last(), Some(&(640, 352)));
    assert_eq!(client.sizes.len(), configured_count + 2);
    let convergence_serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(state.native.native_commit_may_update_model(
        win,
        640,
        352,
        Some(convergence_serial),
        true
    ));
    assert!(!state.native.native_commit_may_update_model(
        win,
        1920,
        1080,
        Some(tiled_serial),
        true
    ));

    // Retarget an unsent resize, then request the final size immediately while
    // preserving its spatial animation. Only the final intent may be sent.
    for size in [(480, 270), (320, 180)] {
        let target = Rect::new(floating.x, floating.y, size.0, size.1);
        state.wm.core.state.model.sync_client_geometry(win, target);
        state.native.set_window_target_rect(
            &state.wm.core.state,
            win,
            target,
            state.wm.core.state.model.client(win).unwrap().border_width,
            super::WindowMoveMode::AnimateFrom {
                from: constrained,
                duration: Duration::from_millis(500),
            },
        );
    }
    let target = state.wm.core.state.model.client(win).unwrap().geo;
    state
        .native
        .resize_window(&state.wm.core.state, win, target);
    assert!(state.native.animation_targets_outer_rect(win, target));
    assert_eq!(
        state
            .native
            .geometry_sync
            .get(&win)
            .unwrap()
            .scheduled_size(),
        None
    );

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.last(), Some(&(320, 180)));
    assert_eq!(client.sizes.len(), configured_count + 3);
    state
        .native
        .cancel_window_animation(&state.wm.core.state, win);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 3);
}

#[test]
fn hidden_resizes_dispatch_without_remapping_and_drops_do_not_send_obsolete_intent() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);

    state.wm.core.state.config.animations.enabled = true;
    let monitor = state.wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .build(),
    );
    let initial = Rect::new(100, 100, 800, 600);
    add_client(
        &mut state.wm.core.state.model,
        monitor,
        Client {
            win,
            geo: initial,
            mode: ClientMode::floating(),
            ..Client::default()
        },
    );
    state.map_window_in_space(win);
    state
        .native
        .resize_window(&state.wm.core.state, win, initial);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    let configured_count = client.sizes.len();
    let element = state.native.find_window(win).unwrap().clone();

    let target = Rect::new(100, 100, 640, 480);
    state.wm.core.state.model.sync_client_geometry(win, target);
    state.native.set_window_target_rect(
        &state.wm.core.state,
        win,
        target,
        state.wm.core.state.model.client(win).unwrap().border_width,
        super::WindowMoveMode::AnimateFrom {
            from: initial,
            duration: Duration::from_millis(500),
        },
    );
    assert!(state.native.window_has_active_animation(win));
    state.unmap_window_from_space(win);
    assert_eq!(state.native.space.element_location(&element), None);
    assert!(!state.native.window_has_active_animation(win));
    assert_eq!(
        state
            .native
            .geometry_sync
            .get(&win)
            .unwrap()
            .scheduled_size(),
        None
    );

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.len(), configured_count + 1);
    assert_eq!(client.sizes.last(), Some(&(640, 480)));
    let serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(
        state
            .native
            .native_commit_may_update_model(win, 640, 480, Some(serial), true)
    );

    // An animated resize requested while already hidden has no frame timer.
    // Its protocol request must nevertheless progress immediately.
    let hidden_target = Rect::new(100, 100, 480, 360);
    state
        .wm
        .core
        .state
        .model
        .sync_client_geometry(win, hidden_target);
    state.native.set_window_target_rect(
        &state.wm.core.state,
        win,
        hidden_target,
        state.wm.core.state.model.client(win).unwrap().border_width,
        super::WindowMoveMode::Retarget {
            duration: Duration::from_millis(500),
        },
    );
    assert_eq!(state.native.space.element_location(&element), None);
    assert!(!state.native.window_has_active_animation(win));
    assert_eq!(
        state
            .native
            .geometry_sync
            .get(&win)
            .unwrap()
            .scheduled_size(),
        None
    );
    state.unmap_window_from_space(win);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.len(), configured_count + 2);
    assert_eq!(client.sizes.last(), Some(&(480, 360)));
    let serial = smithay::utils::Serial::from(*client.serials.last().unwrap());
    assert!(
        state
            .native
            .native_commit_may_update_model(win, 480, 360, Some(serial), true)
    );

    state.map_window_in_space(win);
    let obsolete = Rect::new(100, 100, 400, 300);
    state
        .wm
        .core
        .state
        .model
        .sync_client_geometry(win, obsolete);
    state.native.set_window_target_rect(
        &state.wm.core.state,
        win,
        obsolete,
        state.wm.core.state.model.client(win).unwrap().border_width,
        super::WindowMoveMode::AnimateFrom {
            from: hidden_target,
            duration: Duration::from_millis(500),
        },
    );
    state.native.drop_window_animation(win);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.len(), configured_count + 2);
    let replacement = Rect::new(100, 100, 320, 240);
    state
        .wm
        .core
        .state
        .model
        .sync_client_geometry(win, replacement);
    state.native.set_window_target_rect(
        &state.wm.core.state,
        win,
        replacement,
        state.wm.core.state.model.client(win).unwrap().border_width,
        super::WindowMoveMode::AnimateFrom {
            from: hidden_target,
            duration: Duration::from_millis(500),
        },
    );
    state
        .native
        .cancel_window_animation(&state.wm.core.state, win);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    assert_eq!(client.sizes.len(), configured_count + 3);
    assert_eq!(client.sizes.last(), Some(&(320, 240)));

    // End-of-surface cleanup discards pending intent without configuring it.
    state
        .wm
        .core
        .state
        .model
        .sync_client_geometry(win, obsolete);
    state.native.set_window_target_rect(
        &state.wm.core.state,
        win,
        obsolete,
        state.wm.core.state.model.client(win).unwrap().border_width,
        super::WindowMoveMode::AnimateFrom {
            from: replacement,
            duration: Duration::from_millis(500),
        },
    );
    state.remove_window_tracking(win);

    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.len(), configured_count + 3);
    assert!(!state.native.geometry_sync.contains_key(&win));
}

/// Shared focus commits policy and projects configure/seat state synchronously
/// through the production compositor's sole WM owner.
#[test]
fn owned_focus_projects_policy_and_native_state_synchronously() {
    use crate::backend::wayland::commands::WmCommand;
    use crate::backend::wayland::runtime::dispatch::drain_command_queue;

    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn_a, mut queue_a, mut client_a, a) = connect_native_window(&mut event_loop, &mut state);
    let (conn_b, mut queue_b, mut client_b, b) = connect_native_window(&mut event_loop, &mut state);
    state.wm = Wm::new(crate::backend::WaylandBackendData::default());
    let monitor = state.wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .tag_count(1)
            .selected_tags(crate::types::TagMask::single(1).unwrap())
            .build(),
    );
    for win in [a, b] {
        add_client(
            &mut state.wm.core.state.model,
            monitor,
            Client {
                win,
                is_urgent: true,
                tags: crate::types::TagMask::single(1).unwrap(),
                ..Client::default()
            },
        );
    }
    let _ = state.wm.core.state.model.set_fullscreen(b, true).unwrap();
    for win in [a, b] {
        let element = state.native.find_window(win).unwrap().clone();
        state.native.space.map_element(element, (0, 0), false);
    }
    state.native.command_queue.borrow_mut().clear();

    state.native.push_command(WmCommand::FocusWindow(b));
    {
        drain_command_queue(&mut state);
    }
    assert_eq!(state.wm.core.state.model.selected_win(), Some(b));
    assert!(state.is_seat_focused_on(b));
    assert!(!state.wm.core.state.model.client(b).unwrap().is_urgent);
    pump(
        &mut event_loop,
        &mut state,
        &conn_b,
        &mut queue_b,
        &mut client_b,
    );
    let focused_b = client_b.states.last().unwrap();
    assert!(focused_b.contains(&(xdg_toplevel::State::Activated as u32)));
    assert!(focused_b.contains(&(xdg_toplevel::State::Fullscreen as u32)));

    // Focus ordering must be projected immediately, rather than coalesced to
    // one final selection at the next tick. The old window keeps fullscreen
    // while losing activation; the tiled target keeps its maximized flags.
    state.native.push_command(WmCommand::FocusWindow(a));
    {
        drain_command_queue(&mut state);
    }
    assert_eq!(state.wm.core.state.model.selected_win(), Some(a));
    assert_eq!(state.wm.core.focus.last_client, b);
    assert!(state.is_seat_focused_on(a));
    assert!(!state.wm.core.state.model.client(a).unwrap().is_urgent);
    pump(
        &mut event_loop,
        &mut state,
        &conn_a,
        &mut queue_a,
        &mut client_a,
    );
    pump(
        &mut event_loop,
        &mut state,
        &conn_b,
        &mut queue_b,
        &mut client_b,
    );
    let unfocused_b = client_b.states.last().unwrap();
    assert!(!unfocused_b.contains(&(xdg_toplevel::State::Activated as u32)));
    assert!(unfocused_b.contains(&(xdg_toplevel::State::Fullscreen as u32)));
    assert_eq!(
        state
            .native
            .space
            .elements()
            .map(|element| {
                element
                    .user_data()
                    .get::<crate::backend::wayland::compositor::WindowIdMarker>()
                    .unwrap()
                    .id
            })
            .collect::<Vec<_>>(),
        vec![a, b],
        "focusing a tiled client must leave fullscreen in its protected layer",
    );
    let focused_a = client_a.states.last().unwrap();
    assert!(focused_a.contains(&(xdg_toplevel::State::Activated as u32)));
    assert!(focused_a.contains(&(xdg_toplevel::State::Maximized as u32)));

    // Reconcile native focus drift even when selection has not changed.
    state.clear_seat_focus();
    state.native.push_command(WmCommand::FocusWindow(a));
    {
        drain_command_queue(&mut state);
    }
    assert!(state.is_seat_focused_on(a));

    state.native.push_command(WmCommand::RestoreFocus);
    {
        drain_command_queue(&mut state);
    }
    assert_eq!(state.wm.core.state.model.selected_win(), Some(a));
    assert!(state.is_seat_focused_on(a));

    for win in [a, b] {
        state.wm.core.state.model.client_mut(win).unwrap().is_hidden = true;
    }
    state.native.push_command(WmCommand::RestoreFocus);
    {
        drain_command_queue(&mut state);
    }
    assert_eq!(state.wm.core.state.model.selected_win(), None);
    assert!(state.native.keyboard.current_focus().is_none());
    pump(
        &mut event_loop,
        &mut state,
        &conn_a,
        &mut queue_a,
        &mut client_a,
    );
    assert!(
        !client_a
            .states
            .last()
            .unwrap()
            .contains(&(xdg_toplevel::State::Activated as u32))
    );
}

/// A client request must commit shared policy before any later configure in
/// the same dispatch. The runtime tick is deliberately never run in this test.
#[test]
fn protocol_presentation_requests_commit_before_later_native_configures() {
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);

    let floating = Rect::new(120, 80, 640, 360);
    let desktop = Rect::new(0, 0, 1920, 1080);
    {
        state.wm.core.state.config.animations.enabled = false;
        let monitor = state.wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(desktop)
                .bar(0, false)
                .tag_count(1)
                .selected_tags(crate::types::TagMask::single(1).unwrap())
                .build(),
        );
        state
            .wm
            .core
            .state
            .model
            .monitor_mut(monitor)
            .unwrap()
            .per_tag_state()
            .presentation = crate::layouts::PresentationMode::Floating;
        add_client(
            &mut state.wm.core.state.model,
            monitor,
            Client {
                win,
                geo: floating,
                mode: ClientMode::floating(),
                tags: crate::types::TagMask::single(1).unwrap(),
                ..Client::default()
            },
        );
        state.map_window_in_space(win);
        state
            .native
            .resize_window(&state.wm.core.state, win, floating);
        crate::focus::focus(&mut state.ctx(), Some(win));
    }
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    client.toplevel.as_ref().unwrap().set_fullscreen(None);
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    {
        assert!(
            state
                .wm
                .core
                .state
                .model
                .client(win)
                .unwrap()
                .mode()
                .is_true_fullscreen()
        );
        // Floating placement remains saved in the model; native geometry
        // immediately projects the fullscreen transition.
        assert_eq!(client.sizes.last(), Some(&(desktop.w, desktop.h)));
        // Reproduce the ordering constraint that originally motivated the WM
        // back-reference: another native configure before a shared WM tick.
        let window = state.native.find_window(win).unwrap().clone();
        state
            .native
            .send_toplevel_configure(&state.wm.core.state, &window, None);
    }
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert!(
        client
            .states
            .last()
            .unwrap()
            .contains(&(xdg_toplevel::State::Fullscreen as u32))
    );

    client.toplevel.as_ref().unwrap().unset_fullscreen();
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(state.wm.core.state.model.client(win).unwrap().geo, floating);
    assert!(
        !client
            .states
            .last()
            .unwrap()
            .contains(&(xdg_toplevel::State::Fullscreen as u32))
    );
    assert_eq!(client.sizes.last(), Some(&(floating.w, floating.h)));

    client.toplevel.as_ref().unwrap().set_maximized();
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert!(
        state
            .wm
            .core
            .state
            .model
            .client(win)
            .unwrap()
            .mode()
            .is_maximized()
    );
    assert!(
        client
            .states
            .last()
            .unwrap()
            .contains(&(xdg_toplevel::State::Maximized as u32))
    );
    client.toplevel.as_ref().unwrap().unset_maximized();
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(state.wm.core.state.model.client(win).unwrap().geo, floating);
    assert!(
        !client
            .states
            .last()
            .unwrap()
            .contains(&(xdg_toplevel::State::Maximized as u32))
    );
}

/// Unblocking a native commit is protocol dispatch, even when a render/timing
/// path signals its barrier. Its callback must see an available WM owner.
#[test]
fn blocked_native_commits_dispatch_after_runtime_borrows_end() {
    use smithay::reexports::wayland_server::Resource;
    use smithay::wayland::compositor::{Barrier, add_blocker, add_post_commit_hook};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);
    let surface = state
        .native
        .find_window(win)
        .unwrap()
        .toplevel()
        .unwrap()
        .wl_surface()
        .clone();
    let applied = Arc::new(AtomicUsize::new(0));
    let applied_in_hook = applied.clone();
    add_post_commit_hook::<WaylandState, _>(&surface, move |state, _, _| {
        // This represents native callbacks that synchronously read shared
        // policy, such as placing a newly committed native systray menu.
        let _core = &state.wm.core.state;
        applied_in_hook.fetch_add(1, Ordering::SeqCst);
    });
    let barrier = Barrier::new(false);
    add_blocker(&surface, barrier.clone());
    client.surface.as_ref().unwrap().commit();
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(applied.load(Ordering::SeqCst), 0);

    {
        let runtime_borrow = &mut state.wm;
        barrier.signal();
        // Signaling during a runtime lease must not invoke the post-commit hook.
        state.native.defer_commit_client(surface.client().unwrap());
        state.native.defer_commit_client(surface.client().unwrap());
        assert_eq!(applied.load(Ordering::SeqCst), 0);
        // Keep the model borrow live until after both native queue operations.
        assert!(runtime_borrow.core.is_running());
    }
    state.dispatch_pending_commits();
    assert_eq!(applied.load(Ordering::SeqCst), 1);
    state.dispatch_pending_commits();
    assert_eq!(applied.load(Ordering::SeqCst), 1);
}

#[test]
fn immediate_layout_suppresses_native_spawn_without_disabling_configured_animations() {
    use crate::layouts::ArrangeAnimation;
    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);
    state.wm.core.state.config.animations.enabled = true;
    let monitor = state.wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1920, 1080))
            .bar(0, false)
            .tag_count(1)
            .selected_tags(crate::types::TagMask::single(1).unwrap())
            .build(),
    );
    add_client(
        &mut state.wm.core.state.model,
        monitor,
        Client {
            win,
            geo: Rect::new(100, 100, 300, 200),
            tags: crate::types::TagMask::single(1).unwrap(),
            ..Client::default()
        },
    );
    state.wm.core.queue_initial_window_layout(win, monitor);
    crate::layouts::arrange(&mut state.ctx(), Some(monitor), ArrangeAnimation::Immediate);
    assert!(state.wm.core.state.config.animations.enabled);
    assert!(state.wm.core.work.spawn_animations.is_empty());
    assert!(!state.ctx().window_animation_active(win));
    let target = state.wm.core.state.model.client(win).unwrap().geo;
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(client.sizes.last().copied(), Some((target.w, target.h)));

    // The immediate override belongs to one transaction. A subsequent normal
    // spawn still uses the enabled user setting and starts its native slide.
    state.wm.core.queue_initial_window_layout(win, monitor);
    crate::layouts::arrange(
        &mut state.ctx(),
        Some(monitor),
        ArrangeAnimation::Configured,
    );
    assert!(state.wm.core.state.config.animations.enabled);
    assert!(state.ctx().window_animation_active(win));
}

delegate_noop!(NativeClient: ignore wl_shm::WlShm);
delegate_noop!(NativeClient: ignore wl_shm_pool::WlShmPool);
delegate_noop!(NativeClient: ignore wl_buffer::WlBuffer);

#[test]
fn immediate_moves_damage_old_new_and_border_only_outputs_without_client_commits() {
    use crate::backend::wayland::compositor::render::PendingRenderTargets;
    use crate::geometry::MoveResizeOptions;
    use crate::types::{Size, TagMask};
    use std::os::fd::AsFd;

    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let (conn, mut queue, mut client, win) = connect_native_window(&mut event_loop, &mut state);
    state.wm.core.state.config.animations.enabled = false;
    let tags = TagMask::single(1).unwrap();
    let monitor = state.wm.core.state.model.monitors.push(
        MonitorBuilder::new()
            .monitor_rect(Rect::new(0, 0, 1000, 800))
            .tag_count(1)
            .selected_tags(tags)
            .build(),
    );
    add_client(
        &mut state.wm.core.state.model,
        monitor,
        Client {
            win,
            geo: Rect::new(800, 100, 400, 200),
            border_width: 2,
            mode: ClientMode::floating(),
            tags,
            ..Client::default()
        },
    );
    for (name, x) in [("left", 0), ("right", 1000), ("unrelated", 2000)] {
        let output = state.native.create_output(name, Size::new(1000, 800), None);
        state.native.space.map_output(&output, (x, 0));
    }
    state.ctx().move_resize(
        win,
        Rect::new(800, 100, 400, 200),
        MoveResizeOptions::immediate(),
    );
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);

    // Commit an actual surface once. Every subsequent movement is compositor
    // work only: no client commits and no DRM pointer events can mask damage.
    let registry = conn.display().get_registry(&queue.handle(), ());
    let global = client
        .globals
        .iter()
        .find(|(_, name)| name == "wl_shm")
        .unwrap()
        .0;
    let shm: wl_shm::WlShm = registry.bind(global, 1, &queue.handle(), ());
    let file = tempfile::tempfile().unwrap();
    file.set_len(400 * 200 * 4).unwrap();
    let pool = shm.create_pool(file.as_fd(), 400 * 200 * 4, &queue.handle(), ());
    let buffer = pool.create_buffer(
        0,
        400,
        200,
        400 * 4,
        wl_shm::Format::Argb8888,
        &queue.handle(),
        (),
    );
    let surface = client.surface.as_ref().unwrap();
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    pump(&mut event_loop, &mut state, &conn, &mut queue, &mut client);
    assert_eq!(
        state.native.find_window(win).unwrap().geometry().size,
        (400, 200).into()
    );

    for (x, expected) in [
        (900, vec!["left", "right"]),
        (1300, vec!["left", "right"]),
        (100, vec!["left", "right"]),
        (597, vec!["left", "right"]), // Only the right border reaches the right output.
        (100, vec!["left", "right"]), // Erase that border again.
        (-1000, vec!["left"]),
    ] {
        state.native.take_render_targets();
        state.ctx().move_resize(
            win,
            Rect::new(x, 100, 400, 200),
            MoveResizeOptions::immediate(),
        );
        let PendingRenderTargets::Outputs(outputs) = state.native.take_render_targets() else {
            panic!("move to {x} must schedule targeted damage");
        };
        assert_eq!(
            outputs,
            expected.into_iter().map(str::to_owned).collect(),
            "move to {x}"
        );
    }
}

#[test]
fn arranging_either_monitor_keeps_spanning_floats_above_all_tiles() {
    use crate::layouts::{ArrangeAnimation, arrange};
    use crate::types::TagMask;

    let (mut event_loop, mut state) = crate::test_support::new_compositor();
    let left_client = connect_native_window(&mut event_loop, &mut state);
    let float_client = connect_native_window(&mut event_loop, &mut state);
    let right_client = connect_native_window(&mut event_loop, &mut state);
    let (left_tile, floating, right_tile) = (left_client.3, float_client.3, right_client.3);
    let tags = TagMask::single(1).unwrap();
    let monitors: Vec<_> = [0, 1000]
        .into_iter()
        .map(|x| {
            state.wm.core.state.model.monitors.push(
                MonitorBuilder::new()
                    .rect(Rect::new(x, 0, 1000, 800), Rect::new(x, 0, 1000, 800))
                    .bar(0, false)
                    .tag_count(1)
                    .selected_tags(tags)
                    .build(),
            )
        })
        .collect();
    for (win, monitor, mode) in [
        (left_tile, monitors[0], ClientMode::tiled()),
        (floating, monitors[0], ClientMode::floating()),
        (right_tile, monitors[1], ClientMode::tiled()),
    ] {
        add_client(
            &mut state.wm.core.state.model,
            monitor,
            Client {
                win,
                mode,
                tags,
                geo: Rect::new(800, 100, 400, 200),
                ..Client::default()
            },
        );
    }
    state.wm.core.state.model.monitors.set_selected(monitors[0]);
    state.wm.core.state.config.animations.enabled = false;
    for monitor in [
        None,
        Some(monitors[0]),
        Some(monitors[1]),
        Some(monitors[0]),
    ] {
        arrange(&mut state.ctx(), monitor, ArrangeAnimation::Immediate);
        let stack: Vec<_> = state.native.space.elements().cloned().collect();
        let position = |win| {
            stack
                .iter()
                .position(|window| Some(window) == state.native.find_window(win))
                .unwrap()
        };
        assert!(position(floating) > position(left_tile));
        assert!(position(floating) > position(right_tile));
        // Explicit raises of tiled clients must also obey the global layers.
        state.ctx().raise_client(right_tile);
        let stack: Vec<_> = state.native.space.elements().cloned().collect();
        let position = |win| {
            stack
                .iter()
                .position(|window| Some(window) == state.native.find_window(win))
                .unwrap()
        };
        assert!(position(floating) > position(right_tile));
    }
}
