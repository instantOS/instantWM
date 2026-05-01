//! Shared Wayland runtime setup and per-tick logic for all backends.
//!
//! Bootstrap uses [`create_wayland_wm_boxed`] and [`new_wayland_event_loop_and_state`], then
//! [`attach_wayland_backend_state`], [`attach_gles_renderer_and_protocols`], and the socket /
//! autostart helpers. DRM inserts session/GPU/libinput between socket setup and autostart.
//!
//! Per-tick logic: [`event_loop_tick`], [`process_window_animations`].

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;
use smithay::backend::egl::EGLDisplay;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::wayland_server::Display;
use smithay::wayland::seat::WaylandFocus;

/// D-Bus session, boxed [`Wm`] with Wayland backend, and [`crate::wayland::common::init_wayland_globals`].
pub(crate) fn create_wayland_wm_boxed() -> Box<Wm> {
    crate::wayland::common::ensure_dbus_session();
    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        crate::wayland::common::init_wayland_globals(&mut wm.g, wayland);
    }
    wm
}

/// Calloop [`EventLoop`], Wayland [`Display`], and [`WaylandState`].
pub(crate) fn new_wayland_event_loop_and_state() -> (EventLoop<'static, WaylandState>, WaylandState)
{
    let event_loop = EventLoop::try_new().expect("wayland event loop");
    let loop_handle = event_loop.handle();
    let display = Display::new().expect("wayland display");
    let state = WaylandState::new(display, &loop_handle);
    (event_loop, state)
}

/// Attach GLES renderer, dmabuf global, and screencopy protocol (winit and DRM).
pub fn attach_gles_renderer_and_protocols(
    state: &mut WaylandState,
    renderer: &mut GlesRenderer,
    egl_display: Option<&EGLDisplay>,
) {
    state.attach_renderer(renderer);
    let egl_for_dmabuf = egl_display.or_else(|| Some(renderer.egl_context().display()));
    state.init_dmabuf_global(
        ImportDma::dmabuf_formats(renderer).into_iter().collect(),
        egl_for_dmabuf,
    );
    state.init_screencopy_manager();
}

/// Wire the Smithay compositor state into [`WaylandBackend`].
pub fn attach_wayland_backend_state(wm: &mut Box<Wm>, state: &mut WaylandState) {
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.backend.attach_state(state);
    }
}

/// Listening socket, XWayland spawn, and StatusNotifier systray thread — shared by both runtimes.
pub fn setup_wayland_listen_socket_xwayland_systray(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
    wm: &mut Box<Wm>,
) {
    let _socket_name = crate::wayland::common::setup_wayland_socket(loop_handle, state);
    crate::wayland::common::spawn_xwayland(state, loop_handle);
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.wayland_systray_runtime = crate::systray::wayland::WaylandSystrayRuntime::start();
    }
}

/// Startup commands, smoke window, IPC listener registration, and status-bar ping source.
pub fn wayland_autostart_ipc_status_ping(
    loop_handle: &LoopHandle<'static, WaylandState>,
    wm: &crate::wm::Wm,
) -> Option<crate::ipc::IpcServer> {
    crate::runtime::run_startup_commands(wm);
    crate::wayland::common::spawn_wayland_smoke_window();
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    crate::runtime::register_ipc_source(loop_handle, &ipc_server);
    let (status_ping, status_ping_source) = calloop::ping::make_ping().expect("status ping");
    crate::bar::status::set_internal_status_ping(status_ping);
    loop_handle
        .insert_source(status_ping_source, |_, _, _| {})
        .expect("failed to insert status ping source");
    ipc_server
}

/// Run the shared Wayland per-tick housekeeping and return detailed outcome.
pub fn event_loop_tick(
    wm: &mut Wm,
    state: &mut WaylandState,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) -> crate::runtime::TickResult {
    drain_command_queue(wm, state);

    crate::runtime::event_loop_tick_with_options(
        wm,
        ipc_server,
        crate::runtime::TickOptions {
            defer_layout_while_animations_active: true,
            animations_active: state.has_active_window_animations(),
        },
    )
}

fn drain_command_queue(wm: &mut Wm, state: &mut WaylandState) {
    use crate::backend::wayland::commands::WmCommand;
    let commands: Vec<WmCommand> = state.command_queue.borrow_mut().drain(..).collect();

    for command in commands {
        match command {
            WmCommand::FocusWindow(win) => {
                let mut ctx = wm.ctx();
                crate::focus::focus_soft(&mut ctx, Some(win));
            }
            WmCommand::RaiseWindow(win) => {
                let mut ctx = wm.ctx();
                ctx.core_mut().globals_mut().raise_client_in_z_order(win);
                ctx.raise_window_visual_only(win);
            }
            WmCommand::MapWindow(params) => handle_map_window(wm, state, params),
            WmCommand::UnmapWindow(_) => {}
            WmCommand::UnmanageWindow(win) => handle_unmanage_window(wm, win),
            WmCommand::ActivateWindow(win) => handle_activate_window(wm, win),
            WmCommand::PointerMotion { time_msec } => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    let hit_test = state.contents_under_pointer(state.runtime.pointer_location);
                    crate::wayland::input::pointer::motion::dispatch_pointer_motion(
                        wm, state, &pointer, &keyboard, hit_test, time_msec,
                    );
                }
            }
            WmCommand::PointerButton {
                button,
                state: btn_state,
                time_msec,
            } => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    let loc = state.runtime.pointer_location;
                    crate::wayland::input::pointer::button::handle_pointer_button_raw(
                        wm, state, &pointer, &keyboard, button, btn_state, time_msec, loc,
                    );
                }
            }
            WmCommand::PointerAxis {
                source,
                horizontal,
                vertical,
                time_msec,
            } => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    let loc = state.runtime.pointer_location;
                    crate::wayland::input::pointer::axis::handle_pointer_axis_raw(
                        wm, state, &pointer, &keyboard, source, horizontal, vertical, time_msec,
                        loc,
                    );
                }
            }
            WmCommand::BeginMove(win) => {
                let mut ctx = wm.ctx();
                let point = state.runtime.pointer_location;
                let root = crate::types::Point::new(point.x.round() as i32, point.y.round() as i32);
                crate::mouse::drag::title::title_drag_begin(
                    &mut ctx,
                    win,
                    crate::types::MouseButton::Left,
                    root,
                    true,
                );
            }
            WmCommand::BeginResize { win, dir } => handle_begin_resize(wm, state, win, dir),
            WmCommand::UpdateProperties { win, properties } => {
                let mut ctx = wm.ctx();
                crate::client::handle_property_change(
                    ctx.core_mut().globals_mut(),
                    win,
                    &properties,
                );
            }
            WmCommand::UpdateXWaylandPolicy {
                win,
                hints,
                size_hints,
                is_fullscreen,
                is_hidden,
                is_above,
            } => handle_update_xwayland_policy(
                wm,
                win,
                hints,
                size_hints,
                is_fullscreen,
                is_hidden,
                is_above,
            ),
            WmCommand::UpdateWindowSize { win, w, h } => {
                let mut ctx = wm.ctx();
                let g = ctx.core_mut().globals_mut();
                if let Some(client) = g.clients.get(&win)
                    && (client.geo.w != w || client.geo.h != h)
                {
                    let rect = crate::types::Rect {
                        x: client.geo.x,
                        y: client.geo.y,
                        w,
                        h,
                    };
                    crate::client::sync_client_geometry(g, win, rect);
                }
            }
            WmCommand::SetMaximized { win, maximized } => handle_set_maximized(wm, win, maximized),
            WmCommand::SetFullscreen { win, fullscreen } => {
                let mut ctx = wm.ctx();
                let g = ctx.core_mut().globals_mut();
                crate::client::mode::set_fullscreen(g, win, fullscreen);
                g.queue_layout_for_client(win);
                state.request_space_sync();
            }
            WmCommand::SetMinimized { win, minimized } => {
                let mut ctx = wm.ctx();
                if minimized {
                    crate::client::hide(&mut ctx, win);
                } else {
                    crate::client::show_window(&mut ctx, win);
                }
            }
            WmCommand::ShowScratchpad(name) => {
                let mut ctx = wm.ctx();
                let _ = crate::floating::scratchpad_show_name(&mut ctx, &name);
            }
            WmCommand::SetWindowGeometry { win, rect } => {
                if let Some(client) = wm.g.clients.get_mut(&win) {
                    client.geo = rect;
                    client.float_geo = rect;
                }
            }
            WmCommand::RequestSpaceSync => {
                wm.g.queue_layout_for_all_monitors();
                state.request_space_sync();
            }
            WmCommand::RequestBarRedraw => {
                wm.bar.mark_dirty();
            }
            WmCommand::RecordPendingLaunch { pid } => {
                let mut ctx = wm.ctx();
                let launch_context = crate::client::current_launch_context(ctx.core().globals());
                crate::client::lifecycle::record_pending_launch(
                    ctx.core_mut().globals_mut(),
                    pid,
                    None,
                    launch_context,
                );
            }
            WmCommand::RestoreFocus => {
                let mut ctx = wm.ctx();
                crate::focus::focus_soft(&mut ctx, None);
            }
        }
    }
}

fn handle_map_window(
    wm: &mut Wm,
    state: &mut WaylandState,
    params: crate::backend::wayland::commands::MapWindowParams,
) {
    use crate::backend::wayland::commands::MapWindowParams;

    let MapWindowParams {
        win,
        properties,
        initial_geo,
        launch_pid,
        launch_startup_id,
        x11_hints,
        x11_size_hints,
        parent,
    } = params;

    let mut ctx = wm.ctx();
    let g = ctx.core_mut().globals_mut();

    if g.clients.contains_key(&win) {
        return;
    }

    let element = state.find_window(win).cloned();

    let launch_context =
        crate::client::lifecycle::take_pending_launch(g, launch_pid, launch_startup_id.as_deref())
            .or_else(|| {
                element.as_ref()?.wl_surface().and_then(|wl_surface| {
                    smithay::wayland::compositor::with_states(&wl_surface, |states| {
                        states
                            .data_map
                            .get::<crate::backend::wayland::compositor::PendingLaunchContextMarker>(
                            )
                            .map(|marker| marker.context)
                    })
                })
            });

    let mut client = crate::types::Client::default();
    client.win = win;
    client.name = properties.title.clone();
    client.border_width = g.cfg.border_width_px;

    if let Some(lc) = launch_context {
        client.monitor_id = lc.monitor_id;
        client.set_tag_mask(lc.tags);
        if lc.is_floating {
            client.mode = crate::types::ClientMode::Floating;
        }
    } else {
        client.monitor_id = g.selected_monitor_id();
        client.set_tag_mask(crate::client::lifecycle::initial_tags_for_monitor(
            g,
            client.monitor_id,
        ));
    }

    if let Some(hints) = x11_hints {
        crate::client::x11_policy::apply_wm_hints_to_client(&mut client, Some(hints));
    }
    if let Some(shints) = x11_size_hints {
        crate::client::x11_policy::apply_size_hints_to_client(&mut client, Some(shints));
    }

    if let Some(geo) = initial_geo {
        client.geo = geo;
        client.float_geo = geo;
    } else {
        let monitor_rect = g
            .monitor(client.monitor_id)
            .map(|m| m.work_rect)
            .unwrap_or_default();
        client.geo = crate::types::Rect::new(
            monitor_rect.x,
            monitor_rect.y,
            monitor_rect.w.max(100),
            monitor_rect.h.max(100),
        );
        client.float_geo = client.geo;
    }

    g.clients.insert(win, client);
    crate::client::apply_rules(g, win, &properties, launch_context);

    // Determine if the window should float based on compositor policy.
    let should_float = element.as_ref().map_or(false, |e| {
        if let Some(toplevel) = e.toplevel() {
            state.xdg_toplevel_wants_floating(toplevel)
        } else if let Some(x11) = e.x11_surface() {
            parent.is_some() || x11.is_above()
        } else {
            false
        }
    });

    if should_float {
        if let Some(c) = g.clients.get_mut(&win)
            && !c.mode.is_floating()
        {
            c.float_geo = c.geo;
            c.mode = crate::types::ClientMode::Floating;
        }
        g.raise_client_in_z_order(win);
    }

    if let Some(toplevel) = element.as_ref().and_then(|e| e.toplevel()) {
        state.apply_xdg_toplevel_floating_policy(&toplevel.clone());
    }

    crate::client::resolve_and_sync_floating_geometry(
        g,
        win,
        g.clients.get(&win).unwrap().geo,
        crate::client::FloatingPlacementKind::New,
        parent,
    );

    if let Some(rect) = crate::client::sane_floating_spawn_rect(g, win, parent) {
        crate::client::sync_client_geometry(g, win, rect);
        if let Some(e) = element.as_ref() {
            if e.toplevel().is_some() {
                let size = smithay::utils::Size::from((rect.w, rect.h));
                state.send_toplevel_configure(e, Some(size));
            } else if let Some(x11) = e.x11_surface() {
                let _ = x11.configure(Some(smithay::utils::Rectangle::new(
                    (rect.x, rect.y).into(),
                    (rect.w.max(1), rect.h.max(1)).into(),
                )));
            }
        }
    }

    g.attach(win);
    g.attach_z_order_top(win);
    g.queue_layout_for_client(win);

    let should_focus = g
        .clients
        .get(&win)
        .is_some_and(|c| c.is_visible(g.monitor(c.monitor_id).unwrap().selected_tags()));

    if should_focus {
        state.activate_and_raise_window(win);
    }
    state.request_space_sync();
}

fn handle_unmanage_window(wm: &mut Wm, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    let g = ctx.core_mut().globals_mut();
    g.detach(win);
    g.detach_z_order(win);
    g.clients.remove(&win);
    crate::focus::focus_soft(&mut ctx, None);
}

fn handle_activate_window(wm: &mut Wm, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    let is_currently_visible = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .and_then(|c| {
            c.monitor(ctx.core().globals())
                .map(|m| c.is_visible(m.selected_tags()))
        })
        .unwrap_or(false);

    if is_currently_visible {
        crate::focus::activate_client(&mut ctx, win);
    } else if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.is_urgent = true;
    }
}

fn handle_begin_resize(
    wm: &mut Wm,
    state: &mut WaylandState,
    win: crate::types::WindowId,
    dir: crate::types::ResizeDirection,
) {
    let mut ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(wl_ctx) = &mut ctx {
        let point = state.runtime.pointer_location;
        crate::wayland::input::pointer::drag::wayland_hover_resize_drag_begin(
            wl_ctx,
            crate::types::Point::new(point.x.round() as i32, point.y.round() as i32),
            crate::types::MouseButton::Left,
        );
        state.begin_interactive_resize(win);
        crate::mouse::set_cursor_style(
            &mut crate::contexts::WmCtx::Wayland(wl_ctx.reborrow()),
            crate::types::AltCursor::Resize(dir),
        );
    }
}

fn handle_update_xwayland_policy(
    wm: &mut Wm,
    win: crate::types::WindowId,
    hints: Option<x11rb::properties::WmHints>,
    size_hints: Option<x11rb::properties::WmSizeHints>,
    is_fullscreen: bool,
    is_hidden: bool,
    is_above: bool,
) {
    let mut ctx = wm.ctx();
    let g = ctx.core_mut().globals_mut();
    if let Some(client) = g.clients.get_mut(&win) {
        crate::client::x11_policy::apply_wm_hints_to_client(client, hints);
        crate::client::x11_policy::apply_size_hints_to_client(client, size_hints);
    }

    crate::client::mode::set_fullscreen(g, win, is_fullscreen);

    if let Some(client) = g.clients.get_mut(&win) {
        client.is_hidden = is_hidden;

        if is_above && !client.mode.is_floating() {
            client.float_geo = client.geo;
            client.mode = crate::types::ClientMode::Floating;
            g.queue_layout_for_client(win);
        }
    }
}

fn handle_set_maximized(wm: &mut Wm, win: crate::types::WindowId, maximized: bool) {
    let mut ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(ctx_wayland) = &mut ctx {
        let work_rect = ctx_wayland
            .core
            .globals()
            .clients
            .monitor_id(win)
            .and_then(|mid| ctx_wayland.core.globals().monitor(mid))
            .map(|mon| mon.work_rect);
        let outcome =
            crate::client::mode::set_maximized(ctx_wayland.core.globals_mut(), win, maximized);
        if maximized {
            if let Some(work_rect) = work_rect {
                crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                    win,
                    work_rect,
                    crate::geometry::MoveResizeOptions::hinted_immediate(false),
                );
            }
        } else if let (Some(crate::client::mode::MaximizedOutcome::Exited { .. }), Some(client)) =
            (outcome, ctx_wayland.core.globals().clients.get(&win))
        {
            let restore_rect = client.float_geo;
            crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                win,
                restore_rect,
                crate::geometry::MoveResizeOptions::hinted_immediate(false),
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum AnimationTick {
    Idle,
    SpaceSynced,
    AnimationAdvanced,
    SpaceSyncedAndAnimationAdvanced,
}

impl AnimationTick {
    pub fn needs_redraw(self) -> bool {
        !matches!(self, AnimationTick::Idle)
    }
}

/// Run compositor-space sync and animation progression in one place.
pub fn process_window_animations(state: &mut WaylandState) -> AnimationTick {
    let space_synced = if state.take_space_sync_pending() {
        state.sync_space_from_globals();
        true
    } else {
        false
    };
    let animation_advanced = if state.has_active_window_animations() {
        state.tick_window_animations();
        true
    } else {
        false
    };

    match (space_synced, animation_advanced) {
        (false, false) => AnimationTick::Idle,
        (true, false) => AnimationTick::SpaceSynced,
        (false, true) => AnimationTick::AnimationAdvanced,
        (true, true) => AnimationTick::SpaceSyncedAndAnimationAdvanced,
    }
}
