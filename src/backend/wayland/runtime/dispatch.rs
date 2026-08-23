//! Translation of queued Wayland protocol and input commands into WM operations.

use crate::backend::wayland::compositor::WaylandState;
use crate::wm::Wm;
use smithay::wayland::seat::WaylandFocus;

pub(crate) fn drain_command_queue(wm: &mut Wm, state: &mut WaylandState) {
    use crate::backend::wayland::commands::WmCommand;
    use crate::backend::wayland::input::pointer::axis::{PointerAxisInput, handle_pointer_axis};
    use crate::backend::wayland::input::pointer::button::{
        PointerButtonInput, handle_pointer_button,
    };

    let commands = std::mem::take(&mut *state.command_queue.borrow_mut());
    let mut commands = commands.into_iter().peekable();
    let mut pointer_hit_cache = None;
    // Selection as last advertised to foreign-toplevel clients. Compared
    // after every command below: whichever command moves focus (explicit
    // focus, activate, minimize of the focused window, scratchpad show,
    // hover focus during pointer motion, ...), both the old and the new
    // selection get their `activated` state refreshed.
    let mut advertised_selection = wm.core.model.selected_win();

    while let Some(command) = commands.next() {
        let next_is_pointer_motion = matches!(commands.peek(), Some(WmCommand::PointerMotion(_)));
        if !matches!(&command, WmCommand::PointerMotion(_)) {
            pointer_hit_cache = None;
        }
        match command {
            WmCommand::FocusWindow(win) => {
                handle_focus_window(wm, Some(win));
            }
            WmCommand::RaiseWindow(win) => handle_raise_window(wm, win),
            WmCommand::MapWindow(params) => handle_map_window(wm, state, params),
            WmCommand::UnmapWindow(_) => {}
            WmCommand::UnmanageWindow(win) => handle_unmanage_window(wm, win),
            WmCommand::ActivateWindow(win) => {
                // Selection refresh is handled by the post-command diff
                // below; activating an invisible window only marks it
                // urgent, which is not part of the advertised snapshot.
                handle_activate_window(wm, win);
            }
            WmCommand::PointerMotion(motion) => {
                if let (Some(pointer), Some(keyboard)) =
                    (state.seat.get_pointer(), state.seat.get_keyboard())
                {
                    let update_active_drag = should_update_active_drag(
                        wm.core.drag.active_interaction().is_some(),
                        next_is_pointer_motion,
                    );
                    pointer_hit_cache = Some(
                        crate::backend::wayland::input::pointer::motion::process_pointer_motion_command_cached(
                            wm,
                            state,
                            &pointer,
                            &keyboard,
                            motion,
                            pointer_hit_cache.take(),
                            update_active_drag,
                        ),
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
                state.refresh_foreign_toplevel(win);
            }
            WmCommand::UpdateTransientFor { win, parent } => {
                handle_update_transient_for(wm, win, parent);
                state.refresh_foreign_toplevel(win);
            }
            WmCommand::UpdateXWaylandPolicy { win, update } => {
                handle_update_xwayland_policy(wm, win, update);
            }
            WmCommand::UpdateWindowSize {
                win,
                w,
                h,
                acknowledged_configure,
            } => {
                handle_update_window_size(wm, state, win, w, h, acknowledged_configure);
            }
            WmCommand::SetMaximized { win, maximized } => {
                handle_set_maximized(wm, state, win, maximized);
                state.refresh_foreign_toplevel(win);
            }
            WmCommand::SetFullscreen { win, fullscreen } => {
                handle_set_fullscreen(wm, state, win, fullscreen);
                state.refresh_foreign_toplevel(win);
            }
            WmCommand::SetMinimized { win, minimized } => {
                handle_set_minimized(wm, win, minimized);
                state.refresh_foreign_toplevel(win);
            }
            WmCommand::CloseWindow(win) => {
                let mut ctx = wm.ctx();
                // Same funnel as IPC and keybindings: a locked window refuses
                // to close, no matter which client asks.
                crate::client::close_win(&mut ctx, win);
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
                // Output membership for foreign-toplevel clients is
                // refreshed *after* the pending layout has been applied and
                // the space synced (see
                // `engine::process_animations_and_request_render`); doing it
                // here would advertise pre-arrange geometry.
                state.request_space_sync();
            }
            WmCommand::RequestBarRedraw => {
                wm.bar.mark_dirty();
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

        // Selection moved by whichever command ran (or by pointer hover focus
        // inside it): refresh both windows' advertised `activated` state.
        // Refreshing an unmanaged window is a no-op.
        let selected = wm.core.model.selected_win();
        if selected != advertised_selection {
            if let Some(previous) = advertised_selection {
                state.refresh_foreign_toplevel(previous);
            }
            if let Some(current) = selected {
                state.refresh_foreign_toplevel(current);
            }
            advertised_selection = selected;
        }
    }
}

fn should_update_active_drag(active: bool, next_is_pointer_motion: bool) -> bool {
    !active || !next_is_pointer_motion
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
        crate::core_state::ArmedDragOrigin::Client,
        crate::types::InteractionSource::Pointer,
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
    let needs_float = ctx.core().model().client(win).is_some_and(|client| {
        parent.is_some() && client.placement() != crate::types::ClientPlacement::Floating
    });
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.transient_for = parent;
    }
    if needs_float {
        let _ = crate::floating::set_window_placement_from_policy(
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

fn handle_update_window_size(
    wm: &mut Wm,
    state: &mut WaylandState,
    win: crate::types::WindowId,
    w: i32,
    h: i32,
    acknowledged_configure: Option<smithay::utils::Serial>,
) {
    let client_size_is_authoritative = wm
        .core
        .model
        .client(win)
        .is_some_and(|client| client.client_size_is_authoritative());
    if !state.committed_size_may_update_model(
        win,
        w,
        h,
        acknowledged_configure,
        client_size_is_authoritative,
    ) {
        return;
    }
    apply_committed_window_size(wm, win, w, h);
}

fn apply_committed_window_size(wm: &mut Wm, win: crate::types::WindowId, w: i32, h: i32) {
    let mut ctx = wm.ctx();
    let state = ctx.core_mut().state_mut();
    if let Some(client) = state.model.client(win)
        // Tiled, maximized, fullscreen, and scratchpad geometry is owned by the
        // WM. In particular, a native Wayland client may commit a stale startup
        // buffer after layout selected its final size; copying that size back
        // here would overwrite the layout or scratchpad target.
        && client.client_size_is_authoritative()
        && (client.geo.w != w || client.geo.h != h)
    {
        let rect = crate::types::Rect {
            x: client.geo.x,
            y: client.geo.y,
            w,
            h,
        };
        crate::client::sync_client_geometry(&mut state.model, win, rect);
    }
}

fn handle_set_fullscreen(
    wm: &mut Wm,
    state: &mut WaylandState,
    win: crate::types::WindowId,
    fullscreen: bool,
) {
    let Some(transition) = crate::backend::wayland::commands::apply_fullscreen_request(
        &mut wm.core,
        &mut wm.work,
        &mut wm.bar,
        win,
        fullscreen,
    ) else {
        return;
    };
    crate::backend::wayland::commands::apply_fullscreen_geometry(state, win, transition);
    state.sync_window_presentation(win);
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

    crate::overview::exit_overview(&mut ctx, crate::overview::ExitMode::RestorePrevious);
    crate::focus::select_monitor(&mut ctx, monitor_id);

    // ext-workspace-v1 uses zero-based indices; TagMask performs the conversion
    // to its one-based external tag numbering.
    if let Some(mask) = crate::types::TagMask::from_index(tag_index) {
        crate::tags::view::view_tags(&mut ctx, mask);
    }
}

fn handle_map_window(
    wm: &mut Wm,
    wl_state: &mut WaylandState,
    params: crate::backend::wayland::commands::MapWindowParams,
) {
    use crate::backend::wayland::commands::MapWindowParams;

    let MapWindowParams {
        win,
        properties,
        initial_geo,
        initial_position_is_explicit,
        launch_pid,
        launch_startup_id,
        x11_hints,
        x11_size_hints,
        parent,
    } = params;

    let mut ctx = wm.ctx();
    let state = ctx.core_mut().state_mut();

    if state.model.client(win).is_some() {
        return;
    }

    let element = wl_state.find_window(win).cloned();
    let launch_context = take_wayland_launch_context(
        state,
        element.as_ref(),
        launch_pid,
        launch_startup_id.as_deref(),
    );
    let Some(client) = build_initial_wayland_client(
        state,
        win,
        &properties,
        initial_geo,
        parent,
        launch_context,
        (x11_hints, x11_size_hints),
    ) else {
        return;
    };

    if !state.model.insert_client(client) {
        return;
    }
    let rule_outcome = crate::client::apply_initial_rules(state, win, &properties, launch_context);
    let position_is_explicit = rule_outcome
        .placement
        .position_is_explicit(initial_position_is_explicit);

    apply_wayland_surface_policy(state, wl_state, element.as_ref(), win, parent);
    apply_initial_surface_presentation(state, element.as_ref(), win);
    position_new_wayland_floating_window(
        state,
        wl_state,
        element.as_ref(),
        win,
        parent,
        position_is_explicit,
    );

    let Some((monitor_id, should_focus)) = finalize_wayland_client(state, win) else {
        return;
    };
    ctx.core_mut().queue_initial_window_layout(win, monitor_id);

    if should_focus {
        wl_state.request_window_focus(win);
    }
    wl_state.sync_window_presentation(win);
    wl_state.refresh_foreign_toplevel(win);
    wl_state.request_space_sync();
}

fn apply_initial_surface_presentation(
    state: &mut crate::core_state::CoreState,
    element: Option<&smithay::desktop::Window>,
    win: crate::types::WindowId,
) {
    let Some(element) = element else {
        return;
    };
    let presentation = if let Some(toplevel) = element.toplevel() {
        smithay::wayland::compositor::with_states(toplevel.wl_surface(), |surface_states| {
            surface_states
                .data_map
                .get::<std::sync::Mutex<crate::client::mode::InitialPresentationIntent>>()
                .map(|state| *state.lock().unwrap())
                .unwrap_or_default()
        })
    } else if let Some(x11) = element.x11_surface() {
        crate::client::mode::InitialPresentationIntent {
            fullscreen: x11.is_fullscreen(),
            maximized: x11.is_maximized(),
        }
    } else {
        return;
    };

    state
        .model
        .apply_initial_presentation_intent(win, presentation);
}

fn take_wayland_launch_context(
    state: &mut crate::core_state::CoreState,
    element: Option<&smithay::desktop::Window>,
    launch_pid: Option<u32>,
    launch_startup_id: Option<&str>,
) -> Option<crate::client::LaunchContext> {
    crate::client::lifecycle::take_pending_launch(
        &mut state.pending_launches,
        launch_pid,
        launch_startup_id,
    )
    .or_else(|| {
        element?.wl_surface().and_then(|wl_surface| {
            smithay::wayland::compositor::with_states(&wl_surface, |states| {
                states
                    .data_map
                    .get::<crate::backend::wayland::compositor::PendingLaunchContextMarker>()
                    .map(|marker| marker.context)
            })
        })
    })
}

fn build_initial_wayland_client(
    state: &crate::core_state::CoreState,
    win: crate::types::WindowId,
    properties: &crate::client::WindowProperties,
    initial_geo: Option<crate::types::Rect>,
    parent: Option<crate::types::WindowId>,
    launch_context: Option<crate::client::LaunchContext>,
    x11_policy_hints: (
        Option<x11rb::properties::WmHints>,
        Option<x11rb::properties::WmSizeHints>,
    ),
) -> Option<crate::types::Client> {
    let mut client = crate::types::Client::new(win);
    client.name = properties.title.clone();
    client.transient_for = parent;
    if let Some(size_hints) = properties.size_hints {
        client.size_hints = size_hints;
        client.size_hints_valid = true;
    }
    client.border_width = state.config.window.border_width_px;
    client.old_border_width = state.config.window.border_width_px;

    if !crate::client::lifecycle::assign_initial_monitor_and_tags(
        &state.model,
        &mut client,
        parent,
        launch_context,
    ) {
        return None;
    }

    crate::backend::x11::policy::apply_wm_hints_to_client(&mut client, x11_policy_hints.0);
    crate::backend::x11::policy::apply_size_hints_to_client(&mut client, x11_policy_hints.1);

    if let Some(geo) = initial_geo {
        client.geo = geo;
        client.set_preferred_floating_size(geo.size());
    } else {
        let monitor_rect = state.model.monitor(client.monitor_id)?.work_rect();
        client.geo = crate::types::Rect::new(
            monitor_rect.x,
            monitor_rect.y,
            monitor_rect.w.max(100),
            monitor_rect.h.max(100),
        );
    }
    Some(client)
}

fn apply_wayland_surface_policy(
    state: &mut crate::core_state::CoreState,
    wl_state: &mut WaylandState,
    element: Option<&smithay::desktop::Window>,
    win: crate::types::WindowId,
    parent: Option<crate::types::WindowId>,
) {
    if let Some(toplevel) = element.and_then(|element| element.toplevel())
        && wl_state.xdg_toplevel_has_fixed_size_constraints(toplevel)
        && let Some(client) = state.model.client_mut(win)
    {
        client.is_fixed_size = true;
    }

    let should_float = element.is_some_and(|element| {
        if let Some(toplevel) = element.toplevel() {
            wl_state.xdg_toplevel_wants_floating(toplevel)
        } else if let Some(x11) = element.x11_surface() {
            parent.is_some()
                || x11.is_above()
                || state
                    .model
                    .client(win)
                    .is_some_and(|client| client.is_fixed_size)
                || crate::backend::x11::policy::should_float_for_x11_type(x11.window_type())
        } else {
            false
        }
    });

    if should_float {
        if let Some(client) = state.model.client_mut(win) {
            client.set_placement(crate::types::ClientPlacement::Floating);
        }
        state.model.raise_client_in_z_order(win);
    }

    if let Some(toplevel) = element.and_then(|element| element.toplevel()) {
        wl_state.apply_floating_policy(toplevel);
    }
}

fn position_new_wayland_floating_window(
    state: &mut crate::core_state::CoreState,
    wl_state: &mut WaylandState,
    element: Option<&smithay::desktop::Window>,
    win: crate::types::WindowId,
    parent: Option<crate::types::WindowId>,
    position_is_explicit: bool,
) {
    let Some(rect) =
        crate::client::sane_floating_spawn_rect(&state.model, win, parent, position_is_explicit)
    else {
        return;
    };
    crate::client::sync_client_geometry(&mut state.model, win, rect);

    let Some(element) = element else {
        return;
    };
    if element.toplevel().is_some() {
        wl_state
            .send_toplevel_configure(element, Some(smithay::utils::Size::from((rect.w, rect.h))));
    } else if let Some(x11) = element.x11_surface() {
        let _ = x11.configure(Some(smithay::utils::Rectangle::new(
            (rect.x, rect.y).into(),
            (rect.w.max(1), rect.h.max(1)).into(),
        )));
    }
}

fn finalize_wayland_client(
    state: &mut crate::core_state::CoreState,
    win: crate::types::WindowId,
) -> Option<(crate::types::MonitorId, bool)> {
    let attached = state.model.attach_client(win);
    debug_assert!(attached, "managed Wayland client must have a valid monitor");

    if state
        .model
        .client(win)
        .is_some_and(|client| client.mode().is_normal_floating())
        && let Some(current) = state.model.client(win).map(|client| client.geo)
    {
        crate::client::sync_client_geometry(&mut state.model, win, current);
    }

    state.model.client_view(win).map(|view| {
        (
            view.client.monitor_id,
            view.client.is_visible(view.monitor.visible_tags()),
        )
    })
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
    let _ = crate::mouse::interaction::handle(
        &mut ctx,
        crate::mouse::interaction::InteractionEvent {
            source: crate::mouse::interaction::InteractionSource::Pointer,
            phase: crate::mouse::interaction::InteractionPhase::Cancel { reason },
            root: Default::default(),
            modifiers: 0,
            sidebar_hover: None,
        },
    );
}

fn handle_activate_window(wm: &mut Wm, win: crate::types::WindowId) {
    let mut ctx = wm.ctx();
    let is_currently_visible = ctx
        .core()
        .model()
        .client_view(win)
        .is_some_and(|view| view.client.is_visible(view.monitor.visible_tags()));

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
    crate::client::fullscreen::leave_maximized(&mut ctx, win);
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
                source: crate::types::InteractionSource::Pointer,
                direction: dir,
                start,
                geometry,
                policy: crate::core_state::ResizePolicy::Free,
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
    update: crate::backend::x11::policy::XWaylandPolicyUpdate,
) {
    let mut ctx = wm.ctx();
    let outcome =
        crate::backend::x11::policy::apply_xwayland_policy(ctx.core_mut().model_mut(), win, update);
    if let Some(outcome) = outcome {
        if let Some(rect) = outcome.presentation_rect() {
            ctx.move_resize(win, rect, crate::geometry::MoveResizeOptions::immediate());
        }
        if outcome.should_raise() {
            ctx.raise_client(win);
        }
        crate::client::fullscreen::sync_client_maximized_signal(&mut ctx, win);
        if outcome.layout_changed() {
            ctx.core_mut()
                .queue_layout_for_monitor(outcome.monitor_id());
        }
        if outcome.bar_changed() {
            ctx.request_bar_update();
        }
    }
}

fn handle_set_maximized(
    wm: &mut Wm,
    state: &mut WaylandState,
    win: crate::types::WindowId,
    maximized: bool,
) {
    let Some(transition) = crate::backend::wayland::commands::apply_maximized_request(
        &mut wm.core,
        &mut wm.work,
        &mut wm.bar,
        win,
        maximized,
    ) else {
        return;
    };
    crate::backend::wayland::commands::apply_maximized_geometry(state, win, transition);
    if transition.entered_floating_presentation() {
        state.raise_window_visual_only(win);
    }
    state.sync_window_presentation(win);
    state.request_space_sync();
    state.request_render();
}
#[cfg(test)]
mod tests {
    use super::{
        apply_committed_window_size, handle_set_minimized, handle_update_xwayland_policy,
        should_update_active_drag,
    };
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, ClientMode, ClientPlacement, Monitor, Rect, WindowId};
    use crate::wm::Wm;

    #[test]
    fn only_the_last_consecutive_motion_updates_an_active_drag() {
        assert!(!should_update_active_drag(true, true));
        assert!(should_update_active_drag(true, false));
        assert!(should_update_active_drag(false, true));
        assert!(should_update_active_drag(false, false));
    }

    #[test]
    fn unminimizing_reveals_without_activating() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        wm.core.model.monitors.set_selected(monitor_id);
        let focused = WindowId(80);
        let minimized = WindowId(81);

        for (win, is_hidden) in [(focused, false), (minimized, true)] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                is_hidden,
                ..Client::default()
            });
            wm.core
                .model
                .monitor_mut(monitor_id)
                .unwrap()
                .clients
                .push(win);
        }
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected(Some(focused));

        handle_set_minimized(&mut wm, minimized, false);

        assert!(!wm.core.model.client(minimized).unwrap().is_hidden);
        assert_eq!(wm.core.model.selected_win(), Some(focused));
    }

    #[test]
    fn initial_fullscreen_intent_is_applied_after_window_creation() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        let win = WindowId(72);
        wm.core.model.insert_client(Client {
            win,
            monitor_id,
            ..Client::default()
        });

        wm.core.model.apply_initial_presentation_intent(
            win,
            crate::client::mode::InitialPresentationIntent {
                fullscreen: true,
                maximized: false,
            },
        );

        assert!(
            wm.core
                .model
                .client(win)
                .unwrap()
                .mode()
                .is_true_fullscreen()
        );
    }

    #[test]
    fn initial_maximize_becomes_the_fullscreen_restore_mode() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        let win = WindowId(73);
        let mut client = Client {
            win,
            monitor_id,
            ..Client::default()
        };
        client.set_placement(ClientPlacement::Floating);
        wm.core.model.insert_client(client);

        wm.core.model.apply_initial_presentation_intent(
            win,
            crate::client::mode::InitialPresentationIntent {
                fullscreen: true,
                maximized: true,
            },
        );
        let mode = wm.core.model.client(win).unwrap().mode();

        assert!(mode.is_true_fullscreen());
        assert_eq!(mode.restored(), ClientMode::tiled());
    }

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
            mode: ClientMode::tiled(),
            ..Client::default()
        });
        wm.work.layout.clear();
        let bar_seq = wm.bar.update_seq();

        let update = || crate::backend::x11::policy::XWaylandPolicyUpdate {
            hints: None,
            size_hints: None,
            is_fullscreen: true,
            is_maximized: false,
            is_hidden: false,
            is_above: true,
        };
        handle_update_xwayland_policy(&mut wm, win, update());

        let client = wm.core.model.client(win).unwrap();
        assert!(client.mode().is_true_fullscreen());
        assert_eq!(client.placement(), ClientPlacement::Floating);
        assert_eq!(client.mode().restored(), ClientMode::floating());
        assert_eq!(client.saved_floating_rect(), Some(geo));
        assert!(wm.work.layout.is_pending());
        assert_ne!(wm.bar.update_seq(), bar_seq);

        wm.work.layout.clear();
        let bar_seq = wm.bar.update_seq();
        handle_update_xwayland_policy(&mut wm, win, update());
        assert!(!wm.work.layout.is_pending());
        assert_eq!(wm.bar.update_seq(), bar_seq);
    }

    #[test]
    fn stale_wayland_commit_does_not_override_scratchpad_geometry() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        let win = WindowId(71);
        let geo = Rect::new(480, 216, 960, 648);
        let mut client = Client {
            win,
            monitor_id,
            geo,
            mode: ClientMode::floating(),
            ..Client::default()
        };
        client
            .promote_to_scratchpad("insmenu", None, 1920, 1080)
            .unwrap();
        wm.core.model.insert_client(client);

        apply_committed_window_size(&mut wm, win, 1920, 1080);

        assert_eq!(wm.core.model.client(win).unwrap().geo, geo);
    }
}
