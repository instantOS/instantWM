//! Real layer-shell commits and SHM buffers, without a GPU or running desktop.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use smithay::desktop::layer_map_for_output;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Point, SERIAL_COUNTER};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::wlr_layer::Layer;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_region, wl_registry, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::LayerFocusRequest;
use crate::backend::wayland::compositor::{WaylandClientState, WaylandState};
use crate::backend::wayland::stacking::{OutputScenePolicy, SceneBand};
use crate::test_support::{MonitorBuilder, add_client};
use crate::types::{Client, MonitorId, Rect, Size, TagMask, WindowId};

type EventLoop = smithay::reexports::calloop::EventLoop<'static, WaylandState>;

#[derive(Default)]
struct ClientEvents {
    globals: Vec<(u32, String)>,
    syncs: usize,
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
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for ClientEvents {
    fn event(
        _: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            surface.ack_configure(serial);
        }
    }
}
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
delegate_noop!(ClientEvents: ignore wl_compositor::WlCompositor);
delegate_noop!(ClientEvents: ignore wl_surface::WlSurface);
delegate_noop!(ClientEvents: ignore wl_region::WlRegion);
delegate_noop!(ClientEvents: ignore wl_shm::WlShm);
delegate_noop!(ClientEvents: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ClientEvents: ignore wl_buffer::WlBuffer);
delegate_noop!(ClientEvents: ignore wl_output::WlOutput);
delegate_noop!(ClientEvents: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(ClientEvents: ignore xdg_wm_base::XdgWmBase);
delegate_noop!(ClientEvents: ignore xdg_toplevel::XdgToplevel);

struct Fixture {
    event_loop: EventLoop,
    state: WaylandState,
    conn: Connection,
    queue: EventQueue<ClientEvents>,
    events: ClientEvents,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    xdg: xdg_wm_base::XdgWmBase,
    outputs: Vec<(Output, wl_output::WlOutput, MonitorId)>,
}

struct LayerClient {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    root: WlSurface,
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
                    .bar(30, true)
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
        let shell = registry.bind(global("zwlr_layer_shell_v1"), 4, &queue.handle(), ());
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
            shell,
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
    fn layer(
        &mut self,
        name: &str,
        output: usize,
        layer: zwlr_layer_shell_v1::Layer,
        interactive: zwlr_layer_surface_v1::KeyboardInteractivity,
        mapped: bool,
    ) -> LayerClient {
        let surface = self.compositor.create_surface(&self.queue.handle(), ());
        let layer = self.shell.get_layer_surface(
            &surface,
            Some(&self.outputs[output].1),
            layer,
            name.to_owned(),
            &self.queue.handle(),
            (),
        );
        layer.set_size(200, 200);
        layer.set_keyboard_interactivity(interactive);
        surface.commit();
        self.pump();
        let root = layer_map_for_output(&self.outputs[output].0)
            .layers()
            .find(|layer| layer.namespace() == name)
            .unwrap()
            .wl_surface()
            .clone();
        if mapped {
            self.attach(&surface);
        }
        LayerClient {
            surface,
            layer,
            root,
        }
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
    fn fullscreen(&mut self, win: WindowId, enabled: bool) {
        let _ = self
            .state
            .wm
            .core
            .state
            .model
            .set_fullscreen(win, enabled)
            .unwrap();
        self.state.sync_space();
    }
    fn hit(
        &self,
        x: f64,
        y: f64,
    ) -> crate::backend::wayland::compositor::window::hit_test::PointerContents {
        self.state
            .native
            .contents_under_pointer(&self.state.wm.core.state, Point::from((x, y)))
    }
    fn keyboard_root(&self) -> Option<WlSurface> {
        self.state
            .native
            .keyboard
            .current_focus()
            .and_then(|focus| focus.wl_surface().map(|s| s.into_owned()))
    }
}

#[test]
fn fullscreen_preserves_overlay_render_band_input_and_exclusive_focus() {
    let mut f = Fixture::new(1);
    let (win, _) = f.window();
    let top = f.layer(
        "panel",
        0,
        zwlr_layer_shell_v1::Layer::Top,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    assert_eq!(f.keyboard_root(), Some(top.root.clone()));
    f.fullscreen(win, true);
    let policy = OutputScenePolicy::for_output(&f.state.wm.core.state, &f.outputs[0].0);
    assert_eq!(policy.layer_band(Layer::Top), None);
    assert_eq!(f.hit(100.0, 100.0).hovered_win, Some(win));
    assert_ne!(f.keyboard_root(), Some(top.root.clone()));
    let overlay = f.layer(
        "selector",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    assert_eq!(
        policy.layer_band(Layer::Overlay),
        Some(SceneBand::OverlayLayer)
    );
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, overlay.root);
    assert_eq!(f.keyboard_root(), Some(overlay.root.clone()));
    f.state.focus_window(win, None);
    assert_eq!(f.keyboard_root(), Some(overlay.root.clone()));
    overlay.layer.destroy();
    f.pump();
    assert!(!f.state.exclusive_layer_has_keyboard_focus());
    f.fullscreen(win, false);
    assert_eq!(f.keyboard_root(), Some(top.root));
}

#[test]
fn unmapped_and_noninteractive_layers_release_keyboard_focus() {
    let mut f = Fixture::new(1);
    let layer = f.layer(
        "launcher",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        false,
    );
    assert!(!f.state.focus_layer_keyboard(
        &layer.root,
        SERIAL_COUNTER.next_serial(),
        LayerFocusRequest::Automatic
    ));
    f.attach(&layer.surface);
    assert_eq!(f.keyboard_root(), Some(layer.root.clone()));
    layer
        .layer
        .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
    layer.surface.commit();
    f.pump();
    assert_eq!(f.keyboard_root(), None);
    layer
        .layer
        .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
    layer.surface.commit();
    f.pump();
    assert_eq!(f.keyboard_root(), Some(layer.root.clone()));
    layer.surface.attach(None, 0, 0);
    layer.surface.commit();
    f.pump();
    assert_eq!(f.keyboard_root(), None);
    assert!(f.hit(100.0, 100.0).surface.is_none());
}

#[test]
fn shell_layers_follow_scene_order_instead_of_creation_order() {
    let mut f = Fixture::new(1);
    let (win, _) = f.window();
    let bottom = f.layer(
        "bottom",
        0,
        zwlr_layer_shell_v1::Layer::Bottom,
        zwlr_layer_surface_v1::KeyboardInteractivity::None,
        true,
    );
    assert_eq!(f.hit(100.0, 100.0).hovered_win, Some(win));
    let hit = f.hit(100.0, 100.0);
    assert!(
        hit.surface
            .as_ref()
            .is_none_or(|(surface, _)| { f.state.native.layer_root_surface(surface).is_none() })
    );
    let overlay = f.layer(
        "overlay",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::None,
        true,
    );
    let top = f.layer(
        "newer-top",
        0,
        zwlr_layer_shell_v1::Layer::Top,
        zwlr_layer_surface_v1::KeyboardInteractivity::None,
        true,
    );
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, overlay.root);
    overlay.layer.destroy();
    f.pump();
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, top.root);
    top.layer.destroy();
    f.pump();
    // Both built-in bars and application windows block lower shell input.
    assert!(f.hit(100.0, 10.0).surface.is_none());
    assert_eq!(f.hit(100.0, 100.0).hovered_win, Some(win));
    f.state.unmap_window_from_space(win);
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, bottom.root);
}

#[test]
fn exclusive_overlay_priority_is_global_across_outputs() {
    let mut f = Fixture::new(2);
    let overlay = f.layer(
        "left-selector",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    let top = f.layer(
        "right-launcher",
        1,
        zwlr_layer_shell_v1::Layer::Top,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    assert_eq!(f.keyboard_root(), Some(overlay.root.clone()));
    assert!(!f.state.focus_layer_keyboard(
        &top.root,
        SERIAL_COUNTER.next_serial(),
        LayerFocusRequest::UserInteraction
    ));
    assert_eq!(f.hit(300.0, 100.0).surface.unwrap().0, top.root);
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, overlay.root);
    overlay.layer.destroy();
    f.pump();
    assert_eq!(f.keyboard_root(), Some(top.root));
}

#[test]
fn layer_changes_and_input_holes_follow_the_current_scene() {
    let mut f = Fixture::new(1);
    let (win, _) = f.window();
    let overlay = f.layer(
        "selector",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    let original = f.state.native.find_window(win).unwrap().clone();
    let application_overlay =
        smithay::desktop::Window::new_wayland_window(original.toplevel().unwrap().clone());
    application_overlay.user_data().insert_if_missing(|| {
        crate::backend::wayland::compositor::WindowIdMarker {
            id: win,
            is_overlay: true,
        }
    });
    application_overlay.on_commit();
    f.state.native.space.unmap_elem(&original);
    f.state
        .native
        .space
        .map_element(application_overlay, (0, 0), false);
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, overlay.root);
    let empty_region = f.compositor.create_region(&f.queue.handle(), ());
    overlay.surface.set_input_region(Some(&empty_region));
    overlay.surface.commit();
    f.pump();
    assert_eq!(f.hit(100.0, 100.0).band, SceneBand::WindowOverlay);
    overlay.surface.set_input_region(None);
    overlay.layer.set_layer(zwlr_layer_shell_v1::Layer::Bottom);
    overlay.surface.commit();
    f.pump();
    assert!(!f.state.exclusive_layer_has_keyboard_focus());
    assert_eq!(f.hit(100.0, 100.0).band, SceneBand::WindowOverlay);
    overlay.layer.set_layer(zwlr_layer_shell_v1::Layer::Overlay);
    overlay.surface.commit();
    f.pump();
    assert_eq!(f.hit(100.0, 100.0).surface.unwrap().0, overlay.root);
    assert_eq!(f.keyboard_root(), Some(overlay.root));
}

#[test]
fn fullscreen_policy_is_per_output_and_tracks_tags_fake_fullscreen_and_overview() {
    let mut f = Fixture::new(2);
    let (win, _) = f.window();
    let top = f.layer(
        "panel",
        0,
        zwlr_layer_shell_v1::Layer::Top,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    f.state
        .wm
        .core
        .state
        .model
        .client_mut(win)
        .unwrap()
        .enter_fake_fullscreen();
    f.state.sync_space();
    assert_eq!(f.keyboard_root(), Some(top.root.clone()));
    f.fullscreen(win, true);
    assert_eq!(
        OutputScenePolicy::for_output(&f.state.wm.core.state, &f.outputs[0].0)
            .layer_band(Layer::Top),
        None
    );
    assert_eq!(
        OutputScenePolicy::for_output(&f.state.wm.core.state, &f.outputs[1].0)
            .layer_band(Layer::Top),
        Some(SceneBand::TopLayer)
    );
    f.state
        .wm
        .core
        .state
        .model
        .monitor_mut(f.outputs[0].2)
        .unwrap()
        .set_selected_tags(TagMask::single(2).unwrap());
    f.state.sync_space();
    assert_eq!(f.keyboard_root(), Some(top.root.clone()));
    f.state
        .wm
        .core
        .state
        .model
        .monitor_mut(f.outputs[0].2)
        .unwrap()
        .set_selected_tags(TagMask::single(1).unwrap());
    f.state.sync_space();
    assert_ne!(f.keyboard_root(), Some(top.root.clone()));
    f.state
        .wm
        .core
        .state
        .model
        .monitor_mut(f.outputs[0].2)
        .unwrap()
        .overview_state = Some(crate::overview::OverviewState::new(
        TagMask::single(1).unwrap(),
        vec![win],
        Default::default(),
        Some(win),
    ));
    f.state.sync_space();
    assert_eq!(f.keyboard_root(), Some(top.root));
}

#[test]
fn closing_one_selector_keeps_the_remaining_output_selector_focused() {
    let mut f = Fixture::new(2);
    let first = f.layer(
        "left-selector",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    let second = f.layer(
        "right-selector",
        1,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    second.layer.destroy();
    f.pump();
    assert_eq!(f.keyboard_root(), Some(first.root));
    first.layer.destroy();
    f.pump();
    assert_eq!(f.keyboard_root(), None);
}

#[test]
fn selector_pointer_motion_does_not_offer_resize_of_the_window_below() {
    use crate::backend::wayland::commands::PointerMotionCommand;
    use crate::backend::wayland::input::pointer::motion::process_pointer_motion_command;
    use smithay::backend::input::InputTime;

    let mut f = Fixture::new(1);
    let (win, _) = f.window();
    f.state
        .wm
        .core
        .state
        .model
        .client_mut(win)
        .unwrap()
        .set_placement(crate::types::ClientPlacement::Floating);
    f.state.wm.core.state.model.client_mut(win).unwrap().geo = Rect::new(20, 40, 160, 120);
    f.state.sync_space();
    let pointer = f.state.native.pointer.clone();
    let keyboard = f.state.native.keyboard.clone();
    let motion = || PointerMotionCommand::Absolute {
        x: 185.0,
        y: 100.0,
        time: InputTime::now(),
    };
    process_pointer_motion_command(&mut f.state, &pointer, &keyboard, motion());
    assert!(
        f.state.native.cursor_icon_override.is_some(),
        "fixture must expose a floating resize border"
    );
    let selector = f.layer(
        "selector",
        0,
        zwlr_layer_shell_v1::Layer::Overlay,
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        true,
    );
    process_pointer_motion_command(&mut f.state, &pointer, &keyboard, motion());
    assert_eq!(f.state.native.cursor_icon_override, None);
    assert_eq!(f.keyboard_root(), Some(selector.root));
}
