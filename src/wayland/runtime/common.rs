//! Shared Wayland runtime setup and per-tick logic for all backends.
//!
//! Bootstrap uses [`create_wayland_wm_boxed`] and [`new_wayland_event_loop_and_state`], then
//! [`attach_backend_state`], [`attach_gles_renderer_and_protocols`], and the socket /
//! autostart helpers. DRM inserts session/GPU/libinput between socket setup and autostart.
//!
//! Per-tick logic lives here as well so DRM and winit share scheduling policy.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::backend::Backend as WmBackend;
use crate::backend::wayland::WaylandBackend;
use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;
use smithay::backend::egl::EGLDisplay;
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::Output;
use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::wayland_server::Display;
use smithay::wayland::seat::WaylandFocus;

/// Coalesces callback-only surface commits and delivers them at output refresh
/// cadence without forcing either rendering backend to submit an empty frame.
#[derive(Debug)]
pub(crate) struct FrameCallbackTimerGuard<K> {
    armed: Rc<RefCell<HashMap<K, u64>>>,
    next_generation: Cell<u64>,
}

impl<K> Default for FrameCallbackTimerGuard<K> {
    fn default() -> Self {
        Self {
            armed: Rc::new(RefCell::new(HashMap::new())),
            next_generation: Cell::new(0),
        }
    }
}

impl<K> FrameCallbackTimerGuard<K>
where
    K: Clone + Debug + Eq + Hash + 'static,
{
    pub(crate) fn arm(
        &self,
        key: K,
        loop_handle: &LoopHandle<'_, WaylandState>,
        output: &Output,
        start_time: Instant,
    ) {
        if self.armed.borrow().contains_key(&key) {
            return;
        }

        let generation = self.next_generation.get().wrapping_add(1);
        self.next_generation.set(generation);
        self.armed.borrow_mut().insert(key.clone(), generation);

        let output = output.clone();
        let delay = output_frame_callback_delay(&output);
        let armed_for_timer = Rc::clone(&self.armed);
        let timer_key = key.clone();
        if let Err(err) = loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(delay),
            move |_, _, state| {
                let is_current = armed_for_timer
                    .borrow()
                    .get(&timer_key)
                    .is_some_and(|current| *current == generation);
                if is_current {
                    armed_for_timer.borrow_mut().remove(&timer_key);
                    crate::wayland::common::send_frame_callbacks(
                        state,
                        &output,
                        start_time.elapsed(),
                    );
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        ) {
            let is_current = self
                .armed
                .borrow()
                .get(&key)
                .is_some_and(|current| *current == generation);
            if is_current {
                self.armed.borrow_mut().remove(&key);
            }
            log::warn!("failed to arm frame-callback timer for {key:?}: {err}");
        }
    }

    pub(crate) fn disarm(&self, key: &K) {
        self.armed.borrow_mut().remove(key);
    }
}

fn output_frame_callback_delay(output: &Output) -> Duration {
    output
        .current_mode()
        .and_then(|mode| {
            let refresh = u64::try_from(mode.refresh).ok()?;
            (refresh > 0).then(|| Duration::from_nanos(1_000_000_000_000u64 / refresh))
        })
        .unwrap_or_else(|| Duration::from_millis(16))
}

/// D-Bus session, boxed [`Wm`] with Wayland backend, and [`crate::wayland::common::init_globals`].
pub(crate) fn create_wayland_wm_boxed() -> Box<Wm> {
    crate::wayland::common::ensure_dbus_session();
    let mut wm = Box::new(Wm::new(WmBackend::new_wayland(WaylandBackend::new())));
    if let Some(wayland) = wm.backend.wayland_data_mut() {
        crate::wayland::common::init_globals(&mut wm.core, wayland);
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
pub fn attach_backend_state(wm: &mut Box<Wm>, state: &mut WaylandState) {
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.backend.attach_state(state);
    }
}

/// Listening socket, XWayland spawn, and StatusNotifier systray thread — shared by both runtimes.
pub fn setup_listen_socket(
    loop_handle: &LoopHandle<'static, WaylandState>,
    state: &WaylandState,
    wm: &mut Box<Wm>,
) {
    let _socket_name = crate::wayland::common::setup_socket(loop_handle, state);
    crate::wayland::common::spawn_xwayland(state, loop_handle);
    if let WmBackend::Wayland(data) = &mut wm.backend {
        data.status_notifier_runtime = Some(
            crate::systray::status_notifier::StatusNotifierRuntime::start(std::sync::Arc::clone(
                &state.runtime.pending_systray_menu,
            )),
        );
    }
}

/// Startup commands, smoke window, IPC listener registration, and status-bar ping source.
pub fn autostart_ipc_status_ping(
    loop_handle: &LoopHandle<'static, WaylandState>,
    wm: &crate::wm::Wm,
) -> Option<crate::ipc::IpcServer> {
    crate::runtime::run_startup_commands(wm);
    crate::wayland::common::spawn_smoke_window();
    let ipc_server = crate::ipc::IpcServer::bind().ok();
    crate::runtime::register_ipc_source(loop_handle, &ipc_server);
    let (status_ping, status_ping_source) = calloop::ping::make_ping().expect("status ping");
    crate::bar::status::set_internal_status_ping(status_ping);
    loop_handle
        .insert_source(status_ping_source, |_, _, _| {})
        .expect("failed to insert status ping source");
    ipc_server
}

/// Run the shared Wayland tick and convert model changes into one compositor
/// redraw request. DRM and winit then consume that request using their own
/// output submission machinery.
pub(crate) fn event_loop_tick_and_request_render(
    wm: &mut Wm,
    state: &mut WaylandState,
    ipc_server: &mut Option<crate::ipc::IpcServer>,
) {
    drain_command_queue(wm, state);
    crate::backend::wayland::compositor::protocols::ext_workspace::refresh(state);
    let tick = crate::runtime::event_loop_tick_with_options(
        wm,
        ipc_server,
        crate::runtime::TickOptions {
            defer_layout_while_animations_active: true,
            animations_active: state.has_active_window_animations(),
        },
    );
    // Moving surfaces under a stationary pointer must update Wayland pointer
    // protocol focus in every mode. The synthetic source is kept distinct so
    // only `force` may turn that protocol refresh into keyboard focus.
    if tick.layout_applied
        && let (Some(pointer), Some(keyboard)) =
            (state.seat.get_pointer(), state.seat.get_keyboard())
    {
        crate::wayland::input::pointer::motion::process_pointer_motion_command(
            wm,
            state,
            &pointer,
            &keyboard,
            crate::backend::wayland::commands::PointerMotionCommand::Refresh { time_msec: 0 },
        );
    }
    dismiss_invalid_native_systray_menu(wm, state);
    if tick.ipc_handled || tick.monitor_config_applied || tick.layout_applied {
        state.request_render();
    }
}

fn dismiss_invalid_native_systray_menu(wm: &Wm, state: &mut WaylandState) {
    let Some(active) = state.active_systray_menu().cloned() else {
        return;
    };
    let opening_view_is_current = wm
        .core
        .monitor(active.monitor_id)
        .is_some_and(|monitor| monitor.selected_tags() == active.opened_tags);
    let item_still_exists = match &wm.backend {
        WmBackend::Wayland(data) => data
            .status_notifier_tray
            .items
            .iter()
            .any(|item| item.service == active.service && item.path == active.path),
        _ => false,
    };
    if !wm.core.config.systray.show || !opening_view_is_current || !item_still_exists {
        state.dismiss_native_systray_menu();
    }
}

fn drain_command_queue(wm: &mut Wm, state: &mut WaylandState) {
    use crate::backend::wayland::commands::WmCommand;
    use crate::wayland::input::pointer::axis::{PointerAxisInput, handle_pointer_axis};
    use crate::wayland::input::pointer::button::{PointerButtonInput, handle_pointer_button};

    let commands = std::mem::take(&mut *state.command_queue.borrow_mut());

    for command in commands {
        match command {
            WmCommand::FocusWindow(win) => {
                handle_focus_window(wm, Some(win));
            }
            WmCommand::RaiseWindow(win) => handle_raise_window(wm, win),
            WmCommand::MapWindow(params) => handle_map_window(wm, state, params),
            WmCommand::UnmapWindow(_) => {}
            WmCommand::UnmanageWindow(win) => handle_unmanage_window(wm, win),
            WmCommand::ActivateWindow(win) => handle_activate_window(wm, win),
            WmCommand::PointerMotion(motion) => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    crate::wayland::input::pointer::motion::process_pointer_motion_command(
                        wm, state, &pointer, &keyboard, motion,
                    );
                }
            }
            WmCommand::PointerButton(event) => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    let loc = state.runtime.pointer_location;
                    handle_pointer_button(
                        wm,
                        state,
                        &pointer,
                        &keyboard,
                        PointerButtonInput {
                            event,
                            location: loc,
                        },
                    );
                }
            }
            WmCommand::PointerAxis(event) => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    let loc = state.runtime.pointer_location;
                    handle_pointer_axis(
                        wm,
                        state,
                        &pointer,
                        &keyboard,
                        PointerAxisInput {
                            event,
                            location: loc,
                        },
                    );
                }
            }
            WmCommand::BeginMove(win) => {
                handle_begin_move(wm, state, win);
            }
            WmCommand::BeginResize { win, dir } => handle_begin_resize(wm, state, win, dir),
            WmCommand::CancelInteractiveDrag(reason) => cancel_interactive_drag(wm, reason),
            WmCommand::UpdateProperties { win, properties } => {
                handle_update_properties(wm, win, &properties);
            }
            WmCommand::UpdateTransientFor { win, parent } => {
                handle_update_transient_for(wm, win, parent);
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
                handle_update_window_size(wm, win, w, h);
            }
            WmCommand::SetMaximized { win, maximized } => handle_set_maximized(wm, win, maximized),
            WmCommand::SetFullscreen { win, fullscreen } => {
                handle_set_fullscreen(wm, state, win, fullscreen);
            }
            WmCommand::SetMinimized { win, minimized } => {
                handle_set_minimized(wm, win, minimized);
            }
            WmCommand::ShowScratchpad(name) => {
                let mut ctx = wm.ctx();
                let _ = crate::floating::scratchpad_show_name(&mut ctx, &name);
            }
            WmCommand::SetWindowGeometry { win, rect } => {
                crate::client::sync_client_geometry(&mut wm.core.model, win, rect);
            }
            WmCommand::RequestSpaceSync => {
                wm.work.layout.mark_all();
                state.request_space_sync();
            }
            WmCommand::RequestBarRedraw => {
                wm.bar.mark_dirty();
            }
            WmCommand::RecordPendingLaunch { pid } => {
                handle_record_pending_launch(wm, pid);
            }
            WmCommand::RestoreFocus => {
                handle_focus_window(wm, None);
            }
            WmCommand::SyncLayerExclusiveZones => {
                if crate::backend::wayland::compositor::layer_shell::apply_available_rects(
                    wm, state,
                ) {
                    wm.work.layout.mark_all_urgent();
                    wm.bar.mark_dirty();
                    state.request_render();
                }
            }
            WmCommand::SelectTag {
                monitor_name,
                tag_index,
            } => {
                handle_select_tag(wm, &monitor_name, tag_index);
            }
        }
    }
}

fn handle_focus_window(wm: &mut Wm, win: Option<crate::types::WindowId>) {
    let mut ctx = wm.ctx();
    crate::focus::focus(&mut ctx, win);
}

fn handle_raise_window(wm: &mut Wm, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    ctx.raise_client(win);
}

fn handle_begin_move(wm: &mut Wm, state: &WaylandState, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    let point = state.runtime.pointer_location;
    let root = crate::types::Point::from_f64_round(point.x, point.y);
    crate::mouse::drag::title::title_drag_begin(
        &mut ctx,
        win,
        crate::types::MouseButton::Left,
        root,
        true,
    );
}

fn handle_update_properties(
    wm: &mut Wm,
    win: crate::types::WindowId,
    properties: &crate::client::WindowProperties,
) {
    let mut ctx = wm.ctx();
    crate::client::update_window_properties(ctx.core_mut(), win, properties);
}

fn handle_update_transient_for(
    wm: &mut Wm,
    win: crate::types::WindowId,
    parent: Option<crate::types::WindowId>,
) {
    let mut ctx = wm.ctx();
    let Some(monitor_id) = ctx
        .core()
        .model()
        .client(win)
        .map(|client| client.monitor_id)
    else {
        return;
    };
    let needs_float = ctx
        .core()
        .model()
        .client(win)
        .is_some_and(|client| parent.is_some() && !client.mode().is_floating());
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.transient_for = parent;
    }
    if needs_float {
        let _ = crate::floating::set_window_mode(
            &mut ctx,
            win,
            crate::floating::WindowModeRequest::Floating(
                crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
            ),
        );
    }
    ctx.core_mut().queue_layout_for_monitor(monitor_id);
    crate::layouts::sync_monitor_z_order(&mut ctx, monitor_id);
}

fn handle_update_window_size(wm: &mut Wm, win: crate::types::WindowId, w: i32, h: i32) {
    let mut ctx = wm.ctx();
    let g = ctx.core_mut().state_mut();
    if let Some(client) = g.model.client(win)
        // Tiled, maximized, and fullscreen geometry is owned by the WM. In
        // particular, a native Wayland client may commit a stale startup
        // buffer after layout selected its final size; copying that size back
        // here would overwrite the layout target.
        && client.mode().is_floating()
        && (client.geo.w != w || client.geo.h != h)
    {
        let rect = crate::types::Rect {
            x: client.geo.x,
            y: client.geo.y,
            w,
            h,
        };
        crate::client::sync_client_geometry(&mut g.model, win, rect);
    }
}

fn handle_set_fullscreen(
    wm: &mut Wm,
    state: &mut WaylandState,
    win: crate::types::WindowId,
    fullscreen: bool,
) {
    if !crate::backend::wayland::commands::apply_fullscreen_request(
        &mut wm.core,
        &mut wm.work,
        &mut wm.bar,
        win,
        fullscreen,
    ) {
        return;
    }
    state.request_space_sync();
    state.request_render();
}

fn handle_set_minimized(wm: &mut Wm, win: crate::types::WindowId, minimized: bool) {
    let mut ctx = wm.ctx();
    if minimized {
        crate::client::hide(&mut ctx, win);
    } else {
        crate::client::show_window(&mut ctx, win);
    }
}

fn handle_record_pending_launch(wm: &mut Wm, pid: Option<u32>) {
    let mut ctx = wm.ctx();
    let launch_context = crate::client::current_launch_context(ctx.core().model());
    crate::client::lifecycle::record_pending_launch(
        ctx.core_mut().pending_launches_mut(),
        pid,
        None,
        launch_context,
    );
}

fn handle_select_tag(wm: &mut Wm, monitor_name: &str, tag_index: usize) {
    let mut ctx = wm.ctx();
    let monitor_id = ctx
        .core()
        .model()
        .monitors
        .iter()
        .find(|(_, monitor)| monitor.name == monitor_name)
        .map(|(id, _)| id);
    let Some(monitor_id) = monitor_id else {
        return;
    };

    crate::focus::select_monitor(&mut ctx, monitor_id);

    // ext-workspace-v1 uses zero-based indices; TagMask performs the conversion
    // to its one-based external tag numbering.
    if let Some(mask) = crate::types::TagMask::from_index(tag_index) {
        crate::tags::view::view_tags(&mut ctx, mask);
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
        mut initial_position_is_explicit,
        launch_pid,
        launch_startup_id,
        x11_hints,
        x11_size_hints,
        parent,
    } = params;

    let mut ctx = wm.ctx();
    let g = ctx.core_mut().state_mut();

    if g.model.client(win).is_some() {
        return;
    }

    let element = state.find_window(win).cloned();

    let launch_context = crate::client::lifecycle::take_pending_launch(
        &mut g.pending_launches,
        launch_pid,
        launch_startup_id.as_deref(),
    )
    .or_else(|| {
        element.as_ref()?.wl_surface().and_then(|wl_surface| {
            smithay::wayland::compositor::with_states(&wl_surface, |states| {
                states
                    .data_map
                    .get::<crate::backend::wayland::compositor::PendingLaunchContextMarker>()
                    .map(|marker| marker.context)
            })
        })
    });

    let mut client = crate::types::Client::new(win);
    client.name = properties.title.clone();
    client.transient_for = parent;
    if let Some(size_hints) = properties.size_hints {
        client.size_hints = size_hints;
        client.size_hints_valid = true;
    }
    client.border_width = g.config.window.border_width_px;
    client.old_border_width = g.config.window.border_width_px;

    if let Some(lc) = launch_context {
        client.monitor_id = lc.monitor_id;
        client.set_tag_mask(lc.tags);
        if lc.is_floating {
            client.replace_mode_with_base(crate::types::BaseClientMode::Floating);
        }
    } else {
        let Some(selected_monitor) = g.model.selected_monitor() else {
            return;
        };
        client.monitor_id = selected_monitor.id();
        client.set_tag_mask(selected_monitor.selected_tags());
    }

    // Pending launch contexts can outlive an output. Normalize that stale
    // relationship once, before any geometry or visibility policy uses it.
    if g.monitor(client.monitor_id).is_none() {
        let Some(selected_monitor) = g.model.selected_monitor() else {
            return;
        };
        client.monitor_id = selected_monitor.id();
        client.set_tag_mask(selected_monitor.selected_tags());
    }

    if let Some(hints) = x11_hints {
        crate::backend::x11::policy::apply_wm_hints_to_client(&mut client, Some(hints));
    }
    if let Some(shints) = x11_size_hints {
        crate::backend::x11::policy::apply_size_hints_to_client(&mut client, Some(shints));
    }

    if let Some(geo) = initial_geo {
        client.geo = geo;
        client.set_preferred_floating_size(geo.size());
    } else {
        let Some(monitor_rect) = g.monitor(client.monitor_id).map(|m| m.work_rect()) else {
            return;
        };
        client.geo = crate::types::Rect::new(
            monitor_rect.x,
            monitor_rect.y,
            monitor_rect.w.max(100),
            monitor_rect.h.max(100),
        );
    }

    if !g.model.insert_client(client) {
        return;
    }
    let rule_outcome = crate::client::apply_initial_rules(g, win, &properties, launch_context);
    initial_position_is_explicit = match rule_outcome.placement {
        crate::client::InitialRulePlacement::Default => initial_position_is_explicit,
        crate::client::InitialRulePlacement::Center => false,
        crate::client::InitialRulePlacement::Preserve => true,
    };

    if let Some(toplevel) = element.as_ref().and_then(|e| e.toplevel())
        && state.xdg_toplevel_has_fixed_size_constraints(toplevel)
        && let Some(client) = g.model.client_mut(win)
    {
        client.is_fixed_size = true;
    }

    // Determine if the window should float based on compositor policy.
    let should_float = element.as_ref().is_some_and(|e| {
        if let Some(toplevel) = e.toplevel() {
            state.xdg_toplevel_wants_floating(toplevel)
        } else if let Some(x11) = e.x11_surface() {
            parent.is_some()
                || x11.is_above()
                || g.model.client(win).is_some_and(|c| c.is_fixed_size)
                || crate::backend::x11::policy::should_float_for_x11_type(x11.window_type())
        } else {
            false
        }
    });

    if should_float {
        if let Some(c) = g.model.client_mut(win)
            && c.base_mode() != crate::types::BaseClientMode::Floating
        {
            c.set_base_mode(crate::types::BaseClientMode::Floating);
        }
        g.raise_client_in_z_order(win);
    }

    if let Some(toplevel) = element.as_ref().and_then(|e| e.toplevel()) {
        state.apply_floating_policy(&toplevel.clone());
    }

    if let Some(rect) =
        crate::client::sane_floating_spawn_rect(&g.model, win, parent, initial_position_is_explicit)
    {
        crate::client::sync_client_geometry(&mut g.model, win, rect);
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

    let attached = g.model.attach_client(win);
    debug_assert!(attached, "managed Wayland client must have a valid monitor");
    if g.model
        .client(win)
        .is_some_and(|client| client.mode().is_floating())
    {
        let current = g.model.client(win).map(|client| client.geo);
        if let Some(current) = current {
            crate::client::sync_client_geometry(&mut g.model, win, current);
        }
    }

    let Some((monitor_id, should_focus)) = g.model.client_view(win).map(|view| {
        (
            view.client.monitor_id,
            view.client.is_visible(view.monitor.selected_tags()),
        )
    }) else {
        return;
    };
    ctx.core_mut().queue_layout_for_monitor(monitor_id);

    if should_focus {
        state.request_window_focus(win);
    }
    state.request_space_sync();
}

fn handle_unmanage_window(wm: &mut Wm, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    let cancelled_drag = if let crate::contexts::WmCtx::Wayland(wl_ctx) = &mut ctx {
        crate::mouse::drag::lifecycle::cancel_window(
            wl_ctx.core.drag_state_mut(),
            wl_ctx.wayland,
            win,
            crate::core_state::DragCancelReason::WindowDestroyed,
        )
        .is_some()
    } else {
        false
    };
    if cancelled_drag {
        ctx.set_cursor_style(crate::types::AltCursor::Default);
        ctx.update_layout_preview(None);
        crate::mouse::drag::clear_bar_hover(&mut ctx);
    }
    crate::client::lifecycle::remove_managed_client(&mut ctx, win);
}

fn cancel_interactive_drag(wm: &mut Wm, reason: crate::core_state::DragCancelReason) {
    let mut ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(wl_ctx) = &mut ctx else {
        return;
    };
    if crate::mouse::drag::lifecycle::cancel(wl_ctx.core.drag_state_mut(), wl_ctx.wayland, reason)
        .is_some()
    {
        ctx.set_cursor_style(crate::types::AltCursor::Default);
        ctx.update_layout_preview(None);
        crate::mouse::drag::clear_bar_hover(&mut ctx);
    }
}

fn handle_activate_window(wm: &mut Wm, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    let is_currently_visible = ctx
        .core()
        .model()
        .client_view(win)
        .is_some_and(|view| view.client.is_visible(view.monitor.selected_tags()));

    if is_currently_visible {
        crate::focus::activate_client(&mut ctx, win);
    } else if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
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
        let start = crate::types::Point::from_f64_round(point.x, point.y);
        let Some(geometry) = wl_ctx.core.client_geo(win) else {
            return;
        };
        if crate::mouse::drag::lifecycle::begin_resize(
            wl_ctx.core.drag_state_mut(),
            wl_ctx.wayland,
            crate::mouse::drag::lifecycle::ResizeDragParams {
                win,
                button: crate::types::MouseButton::Left,
                direction: dir,
                start,
                geometry,
            },
        )
        .is_err()
        {
            return;
        }
        crate::contexts::WmCtx::Wayland(wl_ctx.reborrow())
            .set_cursor_style(crate::types::AltCursor::Resize(dir));
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
    let outcome = crate::backend::x11::policy::apply_xwayland_policy(
        ctx.core_mut().model_mut(),
        win,
        crate::backend::x11::policy::XWaylandPolicyUpdate {
            hints,
            size_hints,
            is_fullscreen,
            is_hidden,
            is_above,
        },
    );
    if let Some(outcome) = outcome {
        if outcome.layout_changed() {
            ctx.core_mut()
                .queue_layout_for_monitor(outcome.monitor_id());
        }
        if outcome.bar_changed() {
            ctx.request_bar_update();
        }
    }
}

fn handle_set_maximized(wm: &mut Wm, win: crate::types::WindowId, maximized: bool) {
    let mut ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(ctx_wayland) = &mut ctx {
        let outcome = ctx_wayland.core.model_mut().set_maximized(win, maximized);
        if let Some(transition) = outcome {
            if transition.entered() {
                crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                    win,
                    transition.work_rect(),
                    crate::geometry::MoveResizeOptions::hinted_immediate(false),
                );
            } else if transition.exited() {
                crate::contexts::WmCtx::Wayland(ctx_wayland.reborrow()).move_resize(
                    win,
                    transition.restore_rect(),
                    crate::geometry::MoveResizeOptions::hinted_immediate(false),
                );
            }
        }
    }
}

/// Run compositor-space sync and animation progression in one place, then
/// preserve the resulting redraw in the shared Wayland scheduler.
pub(crate) fn process_animations_and_request_render(state: &mut WaylandState) {
    let space_synced = if state.take_space_sync_pending() {
        state.sync_space_from_globals();
        true
    } else {
        false
    };
    if state.has_active_animations() {
        state.tick_animations();
    }

    // Animation ticks enqueue output-local redraws themselves. Space sync can
    // affect arbitrary windows, so it remains conservatively global.
    if space_synced {
        state.request_render();
    }
}

#[cfg(test)]
mod tests {
    use super::handle_update_xwayland_policy;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{BaseClientMode, Client, ClientMode, Monitor, Rect, WindowId};
    use crate::wm::Wm;

    #[test]
    fn xwayland_above_policy_changes_fullscreen_restore_mode_without_exiting() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        let win = WindowId(70);
        let geo = Rect::new(20, 30, 800, 600);
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            geo,
            mode: ClientMode::Tiling,
            ..Client::default()
        });
        wm.work.layout.clear();
        let bar_seq = wm.bar.update_seq();

        handle_update_xwayland_policy(&mut wm, win, None, None, true, false, true);

        let client = wm.core.model.client(win).unwrap();
        assert!(client.mode().is_true_fullscreen());
        assert_eq!(client.base_mode(), BaseClientMode::Floating);
        assert_eq!(client.mode().restored(), ClientMode::Floating);
        assert_eq!(client.saved_floating_rect(), Some(geo));
        assert!(wm.work.layout.is_pending());
        assert_ne!(wm.bar.update_seq(), bar_seq);

        wm.work.layout.clear();
        let bar_seq = wm.bar.update_seq();
        handle_update_xwayland_policy(&mut wm, win, None, None, true, false, true);
        assert!(!wm.work.layout.is_pending());
        assert_eq!(wm.bar.update_seq(), bar_seq);
    }
}
