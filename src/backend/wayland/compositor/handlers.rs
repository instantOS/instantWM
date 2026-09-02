use smithay::{
    backend::renderer::ImportDma,
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::PopupKind,
    reexports::wayland_server::{Client, Resource},
    wayland::{
        buffer::BufferHandler,
        commit_timing::CommitTimerStateUserData,
        compositor::{
            CompositorHandler, SurfaceAttributes, TraversalAction, get_parent, is_sync_subsurface,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_syncobj::{DrmSyncobjCachedState, DrmSyncobjHandler},
        fifo::FifoCachedState,
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
        input_method::{InputMethodHandler, PopupSurface},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
        output::OutputHandler,
        pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint},
        pointer_warp::PointerWarpHandler,
        seat::WaylandFocus,
        shm::ShmHandler,
        xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
        xwayland_shell::XWaylandShellHandler,
    },
    xwayland::XWaylandClientData,
};

use super::protocols::output_management::{OutputManagementHandler, OutputManagementState};
use super::{
    focus::KeyboardFocusTarget,
    state::{WaylandClientState, WaylandState},
};

impl CompositorHandler for WaylandState {
    fn compositor_state(&mut self) -> &mut smithay::wayland::compositor::CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(
        &self,
        client: &'a Client,
    ) -> &'a smithay::wayland::compositor::CompositorClientState {
        if let Some(data) = client.get_data::<WaylandClientState>() {
            &data.compositor_state
        } else if let Some(data) = client.get_data::<XWaylandClientData>() {
            &data.compositor_state
        } else {
            panic!("client missing compositor client state");
        }
    }

    fn new_surface(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        smithay::wayland::compositor::add_destruction_hook::<Self, _>(surface, |state, surface| {
            state.fifo_constraint_surfaces.remove(surface);
            state.commit_timing_surfaces.remove(surface);
        });
        smithay::wayland::compositor::add_pre_commit_hook::<Self, _>(
            surface,
            move |state, _dh, surface| {
                // These protocol hooks are installed after the compositor's
                // new-surface hook. Record intent from their public pending
                // state before they create blockers, so even a blocked first
                // commit is indexed without scanning every live surface.
                let (sets_fifo_barrier, has_commit_timestamp) =
                    smithay::wayland::compositor::with_states(surface, |surface_data| {
                        let sets_fifo_barrier = surface_data
                            .cached_state
                            .get::<FifoCachedState>()
                            .pending()
                            .set_barrier;
                        let has_commit_timestamp = surface_data
                            .data_map
                            .get::<CommitTimerStateUserData>()
                            .is_some_and(|timer| timer.borrow().timestamp.is_some());
                        (sets_fifo_barrier, has_commit_timestamp)
                    });
                if sets_fifo_barrier {
                    state.fifo_constraint_surfaces.insert(surface.clone());
                }
                if has_commit_timestamp {
                    state.commit_timing_surfaces.insert(surface.clone());
                }

                let mut acquire_point = None;
                let maybe_dmabuf =
                    smithay::wayland::compositor::with_states(surface, |surface_data| {
                        acquire_point.clone_from(
                            &surface_data
                                .cached_state
                                .get::<DrmSyncobjCachedState>()
                                .pending()
                                .acquire_point,
                        );
                        surface_data
                            .cached_state
                            .get::<SurfaceAttributes>()
                            .pending()
                            .buffer
                            .as_ref()
                            .and_then(|assignment| match assignment {
                                smithay::wayland::compositor::BufferAssignment::NewBuffer(
                                    buffer,
                                ) => smithay::wayland::dmabuf::get_dmabuf(buffer).cloned().ok(),
                                _ => None,
                            })
                    });

                if let Some(_dmabuf) = maybe_dmabuf
                    && let Some(acquire_point) = acquire_point
                    && let Ok((blocker, source)) = acquire_point.generate_blocker()
                    && let Some(client) = surface.client()
                {
                    let res = state.loop_handle.insert_source(source, move |_, _, data| {
                        let dh = data.display_handle.clone();
                        data.client_compositor_state(&client)
                            .blocker_cleared(data, &dh);
                        Ok(())
                    });
                    if res.is_ok() {
                        smithay::wayland::compositor::add_blocker(surface, blocker);
                    }
                }
            },
        );
    }

    fn commit(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        let commit_kind = surface_commit_render_service(surface);
        on_commit_buffer_handler::<Self>(surface);

        // Check if this commit is from a pending toplevel that has finally
        // produced a buffer.  If so, promote it to a managed window.
        if let Some(pos) =
            self.runtime.pending_toplevels.iter().position(
                |t: &smithay::wayland::shell::xdg::ToplevelSurface| t.wl_surface() == surface,
            )
        {
            let has_buffer =
                smithay::backend::renderer::utils::with_renderer_surface_state(surface, |state| {
                    state.buffer().is_some()
                })
                .unwrap_or(false);
            if has_buffer {
                let mut toplevel = self.runtime.pending_toplevels.swap_remove(pos);
                let client_pid = toplevel
                    .wl_surface()
                    .client()
                    .and_then(|client| client.get_credentials(&self.display_handle).ok())
                    .and_then(|credentials| u32::try_from(credentials.pid).ok());
                let systray_menu = self.take_expected_systray_menu_toplevel(client_pid);
                if let Some(request) = systray_menu {
                    match self.setup_native_systray_menu(toplevel, request) {
                        Ok(_) => {
                            service_surface_commit(self, commit_kind, None, None);
                            return;
                        }
                        Err(surface) => toplevel = surface,
                    }
                }

                let parent = toplevel
                    .parent()
                    .and_then(|parent| self.window_id_for_surface(&parent));
                let window_id = self.setup_managed_window(toplevel);

                let properties = self.window_properties(window_id);
                let initial_geo = self.find_window(window_id).map(|w| {
                    let g = w.geometry();
                    crate::types::Rect::new(g.loc.x, g.loc.y, g.size.w, g.size.h)
                });

                self.push_command(crate::backend::wayland::commands::WmCommand::MapWindow(
                    crate::backend::wayland::commands::MapWindowParams {
                        win: window_id,
                        properties,
                        initial_geo,
                        initial_position_is_explicit: false,
                        launch_pid: client_pid,
                        launch_startup_id: None,
                        x11_hints: None,
                        x11_size_hints: None,
                        parent,
                    },
                ));
            }
        }

        self.popups.commit(surface);

        if let Some(popup) = self.popups.find_popup(surface)
            && let PopupKind::Xdg(ref popup_surface) = popup
            && !popup_surface.is_initial_configure_sent()
        {
            let _ = popup_surface.send_configure();
        }

        // Find the root surface by walking up the surface tree
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        // Resolve protocol ownership from the complete managed-window index,
        // not from Smithay Space. Tag visibility temporarily removes windows
        // from Space, but clients may still acknowledge activation or size
        // configures while hidden. Those commits must continue to refresh the
        // Window cache and settle configure bookkeeping so the retained buffer
        // is immediately presentable when its tag becomes visible again.
        let committed_window = self
            .window_id_for_surface(&root)
            .and_then(|id| self.window_index.get(&id))
            .cloned();
        let committed_layer_output = super::layer_shell::layer_output_for_surface(self, &root);

        // Skip sync subsurfaces - they don't receive their own commits.
        // Their render request still targets the parent window's outputs.
        if is_sync_subsurface(surface) {
            service_surface_commit(
                self,
                commit_kind,
                committed_window.as_ref(),
                committed_layer_output.as_ref(),
            );
            return;
        }

        // Only call on_commit for the root surface, not for subsurfaces
        if surface != &root {
            service_surface_commit(
                self,
                commit_kind,
                committed_window.as_ref(),
                committed_layer_output.as_ref(),
            );
            return;
        }

        if let Some(window) = committed_window.as_ref() {
            window.on_commit();
            if let Some(id) = window
                .user_data()
                .get::<super::state::WindowIdMarker>()
                .filter(|marker| !marker.is_overlay)
                .map(|marker| marker.id)
            {
                self.reconcile_completed_window_animation(id, window.geometry().size);
                self.sync_client_size_from_window(id);
                // xdg min/max sizes are double-buffered surface state and do
                // not have a dedicated XdgShellHandler callback. Refresh the
                // core snapshot on root commits; unchanged properties are
                // deduplicated by the shared update path.
                if window.x11_surface().is_none() && self.native_size_hints_changed(id) {
                    let properties = self.window_properties(id);
                    self.push_command(
                        crate::backend::wayland::commands::WmCommand::UpdateProperties {
                            win: id,
                            properties,
                        },
                    );
                }
            }
        }

        let committed_layer_output =
            super::layer_shell::handle_layer_commit(self, surface).or(committed_layer_output);

        service_surface_commit(
            self,
            commit_kind,
            committed_window.as_ref(),
            committed_layer_output.as_ref(),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceCommitService {
    None,
    FrameCallbacks,
    Render,
}

fn surface_commit_render_service(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> SurfaceCommitService {
    smithay::wayland::compositor::with_states(surface, |states| {
        let mut guard = states.cached_state.get::<SurfaceAttributes>();
        let attrs = guard.current();
        classify_surface_commit(
            attrs.buffer.is_some() || attrs.buffer_delta.is_some() || !attrs.damage.is_empty(),
            !attrs.frame_callbacks.is_empty(),
            states
                .cached_state
                .get::<smithay::wayland::fifo::FifoBarrierCachedState>()
                .current()
                .barrier
                .is_some(),
        )
    })
}

fn classify_surface_commit(
    has_pixels: bool,
    has_frame_callbacks: bool,
    has_fifo_barrier: bool,
) -> SurfaceCommitService {
    if has_pixels {
        SurfaceCommitService::Render
    } else if has_frame_callbacks || has_fifo_barrier {
        // A FIFO set_barrier commit needs a real output refresh deadline even
        // when it changes no pixels and requests no wl_surface.frame.
        // Otherwise the following wait_barrier commit can never progress.
        SurfaceCommitService::FrameCallbacks
    } else {
        SurfaceCommitService::None
    }
}

fn service_surface_commit(
    state: &mut WaylandState,
    service: SurfaceCommitService,
    window: Option<&smithay::desktop::Window>,
    layer_output: Option<&smithay::output::Output>,
) {
    match service {
        SurfaceCommitService::Render => match window {
            Some(window) => {
                // Hidden managed windows still commit protocol state, but have
                // no pixels to invalidate. Mapping them later requests the
                // first visible render explicitly.
                if state.space.element_location(window).is_some() {
                    state.request_window_render(window);
                }
            }
            None => match layer_output {
                Some(output) => state.request_output_render(output),
                None => state.request_render(),
            },
        },
        SurfaceCommitService::FrameCallbacks => match window {
            Some(window) => {
                // A hidden surface is not being presented and must not be
                // frame-driven. Its callbacks become eligible as soon as the
                // remap render is submitted.
                if state.space.element_location(window).is_some() {
                    state.request_window_frame_callbacks(window);
                }
            }
            None => match layer_output {
                Some(output) => state.request_output_frame_callbacks(output),
                None => state.request_frame_callbacks(),
            },
        },
        SurfaceCommitService::None => {}
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&self) -> &smithay::wayland::shm::ShmState {
        &self.shm_state
    }
}

impl BufferHandler for WaylandState {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl FractionalScaleHandler for WaylandState {
    fn new_fractional_scale(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        smithay::wayland::compositor::with_states(&surface, |states| {
            let Some(output) =
                smithay::desktop::utils::surface_primary_scanout_output(&surface, states)
            else {
                return;
            };
            with_fractional_scale(states, |fractional_scale| {
                fractional_scale.set_preferred_scale(output.current_scale().fractional_scale());
            });
        });
    }
}

impl DmabufHandler for WaylandState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: ImportNotifier,
    ) {
        // Tag the dmabuf with the render node so clients know which device to use.
        if let Some(node) = self.render_node {
            dmabuf.set_node(node);
        }

        let imported = self
            .renderer_mut()
            .and_then(|renderer| renderer.import_dmabuf(&dmabuf, None).ok())
            .is_some();
        if imported {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}

impl OutputHandler for WaylandState {}

impl smithay::wayland::foreign_toplevel_list::ForeignToplevelListHandler for WaylandState {
    fn foreign_toplevel_list_state(
        &mut self,
    ) -> &mut smithay::wayland::foreign_toplevel_list::ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
    }
}

impl XWaylandShellHandler for WaylandState {
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XWaylandKeyboardGrabHandler for WaylandState {
    fn grab(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        seat: smithay::input::Seat<Self>,
        grab: smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrab<Self>,
    ) {
        if self.shortcut_recovery_bypasses(&surface) {
            log::debug!("denied XWayland keyboard re-grab after user recovery");
            return;
        }
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_grab(self, grab, smithay::utils::SERIAL_COUNTER.next_serial());
        }
    }

    fn keyboard_focus_for_xsurface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Self::KeyboardFocus> {
        if let Some(win) = self.window_id_for_surface(surface)
            && let Some(window) = self.window_index.get(&win)
        {
            return Some(KeyboardFocusTarget::Window(window.clone()));
        }

        // Override-redirect X11 windows are intentionally not in window_index,
        // but their XWayland wl_surface is still a valid keyboard focus target.
        // The grab itself ensures events remain routed to this surface.
        Some(KeyboardFocusTarget::WlSurface(surface.clone()))
    }
}

impl KeyboardShortcutsInhibitHandler for WaylandState {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Grant requests immediately. Input routing below still scopes the
        // inhibitor to its associated surface and the seat's current focus.
        if !self.shortcut_recovery_bypasses(inhibitor.wl_surface()) {
            inhibitor.activate();
        }
    }
}

impl smithay::wayland::idle_inhibit::IdleInhibitHandler for WaylandState {
    fn inhibit(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.idle_inhibiting_surfaces.insert(surface);
        log::debug!("idle inhibited for surface");
    }

    fn uninhibit(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.idle_inhibiting_surfaces.remove(&surface);
        log::debug!("idle uninhibited for surface");
    }
}

impl smithay::wayland::idle_notify::IdleNotifierHandler for WaylandState {
    fn idle_notifier_state(
        &mut self,
    ) -> &mut smithay::wayland::idle_notify::IdleNotifierState<Self> {
        &mut self.idle_notify_manager_state
    }
}

impl WaylandState {
    fn root_surface_for(
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> smithay::reexports::wayland_server::protocol::wl_surface::WlSurface {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        root
    }

    fn pointer_constraint_surface_origin(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<smithay::utils::Point<f64, smithay::utils::Logical>> {
        use smithay::backend::renderer::utils::RendererSurfaceStateUserData;

        let requested_root = Self::root_surface_for(surface);
        self.space.elements().find_map(|window| {
            let window_root = window.wl_surface()?;
            if window_root.as_ref() != &requested_root {
                return None;
            }

            let loc = self.space.element_location(window).unwrap_or_default();
            let surface_origin = loc - window.geometry().loc;
            let found = std::cell::RefCell::new(None);
            smithay::wayland::compositor::with_surface_tree_downward(
                window_root.as_ref(),
                surface_origin,
                |_, states, parent_loc: &smithay::utils::Point<i32, smithay::utils::Logical>| {
                    let data = states.data_map.get::<RendererSurfaceStateUserData>();
                    let Some(surface_view) = data.and_then(|d| d.lock().ok()?.view()) else {
                        return TraversalAction::SkipChildren;
                    };
                    TraversalAction::DoChildren(*parent_loc + surface_view.offset)
                },
                |candidate,
                 states,
                 parent_loc: &smithay::utils::Point<i32, smithay::utils::Logical>| {
                    let data = states.data_map.get::<RendererSurfaceStateUserData>();
                    let Some(surface_view) = data.and_then(|d| d.lock().ok()?.view()) else {
                        return;
                    };
                    let candidate_loc = *parent_loc + surface_view.offset;
                    if candidate == surface {
                        *found.borrow_mut() = Some(candidate_loc.to_f64());
                    }
                },
                |_, _, _| found.borrow().is_none(),
            );

            found.into_inner()
        })
    }
}

impl PointerConstraintsHandler for WaylandState {
    fn new_constraint(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Self>,
    ) {
        with_pointer_constraint(surface, pointer, |constraint| {
            if let Some(constraint) = constraint {
                constraint.activate();
            }
        });
    }

    fn cursor_position_hint(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Self>,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) {
        let active = with_pointer_constraint(surface, pointer, |constraint| {
            constraint.is_some_and(|constraint| constraint.is_active())
        });
        if !active {
            return;
        }

        self.cursor_position_hint = Some((surface.clone(), location));
    }

    fn remove_constraint(
        &mut self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        pointer: &smithay::input::pointer::PointerHandle<Self>,
    ) {
        if let Some((hint_surface, hint_location)) = self.cursor_position_hint.take() {
            if &hint_surface == surface {
                if let Some(origin) = self.pointer_constraint_surface_origin(&hint_surface) {
                    let target = origin + hint_location;
                    pointer.set_location(target);
                    self.runtime.pointer_location = target;
                }
            } else {
                self.cursor_position_hint = Some((hint_surface, hint_location));
            }
        }
    }
}

impl PointerWarpHandler for WaylandState {
    fn warp_pointer(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        _pointer: smithay::reexports::wayland_server::protocol::wl_pointer::WlPointer,
        pos: smithay::utils::Point<f64, smithay::utils::Logical>,
        _serial: smithay::utils::Serial,
    ) {
        if let Some(origin) = self.pointer_constraint_surface_origin(&surface) {
            let target = origin + pos;
            if let Some(pointer) = self.seat.get_pointer() {
                pointer.set_location(target);
                self.runtime.pointer_location = target;
            }
        }
    }
}

impl DrmSyncobjHandler for WaylandState {
    fn drm_syncobj_state(&mut self) -> Option<&mut smithay::wayland::drm_syncobj::DrmSyncobjState> {
        self.drm_syncobj_state.as_mut()
    }
}

impl InputMethodHandler for WaylandState {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            log::warn!("Failed to track input method popup: {err}");
        }
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ =
                smithay::desktop::PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {}

    fn parent_geometry(
        &self,
        parent: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> smithay::utils::Rectangle<i32, smithay::utils::Logical> {
        self.space
            .elements()
            .find_map(|window| {
                (window.wl_surface().as_deref() == Some(parent)).then(|| window.geometry())
            })
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// wlr-output-management-unstable-v1 handler
// ---------------------------------------------------------------------------

impl OutputManagementHandler for WaylandState {
    fn output_management_state(&mut self) -> &mut OutputManagementState {
        &mut self.output_management_state
    }

    fn submit_output_transaction(
        &mut self,
        kind: crate::backend::output::OutputTransactionKind,
        transaction: crate::backend::output::OutputTransaction,
        configuration: smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_v1::ZwlrOutputConfigurationV1,
    ) {
        let id = self.runtime.output_transactions.submit(kind, transaction);
        self.output_management_state
            .track_transaction(id, configuration);
        self.request_render();
    }
}

// Wire `WaylandState`'s dispatching into `OutputManagementState`.  After this
// macro expands, `WaylandState` implements every `Dispatch<…, …>` and
// `GlobalDispatch<…, …>` for the wlr-output-management interfaces, forwarding
// to the impls on `OutputManagementState`.  Without this, the
// `where D: Dispatch<…> + OutputManagementHandler` bounds on
// `OutputManagementState::new`, `add_heads`, etc. are not satisfied when
// `D = WaylandState`.
crate::delegate_output_management!(WaylandState);

// ---------------------------------------------------------------------------
// wlr-output-power-management-unstable-v1 handler
// ---------------------------------------------------------------------------

impl crate::backend::wayland::compositor::protocols::output_power::OutputPowerHandler
    for WaylandState
{
    fn output_power_state(
        &mut self,
    ) -> &mut crate::backend::wayland::compositor::protocols::output_power::OutputPowerState {
        &mut self.output_power_state
    }

    fn output_power_mode(
        &self,
        output: &smithay::output::Output,
    ) -> Option<crate::backend::output::OutputPowerMode> {
        self.runtime.output_power_modes.get(&output.name()).copied()
    }

    fn submit_output_power_request(
        &mut self,
        output: crate::backend::output::OutputId,
        mode: crate::backend::output::OutputPowerMode,
    ) -> crate::backend::output::OutputPowerRequestId {
        let id = self.runtime.output_power.submit(output, mode);
        self.request_render();
        id
    }

    fn cancel_output_power_requests(
        &mut self,
        requests: &[crate::backend::output::OutputPowerRequestId],
    ) {
        self.runtime.output_power.cancel(requests);
    }
}

crate::delegate_output_power!(WaylandState);

// ---------------------------------------------------------------------------
// wlr-foreign-toplevel-management-unstable-v1 handler
// ---------------------------------------------------------------------------

impl crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelHandler
    for WaylandState
{
    fn foreign_toplevel_state(
        &mut self,
    ) -> &mut crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelManagementState
    {
        &mut self.foreign_toplevel_management_state
    }

    fn foreign_toplevel_snapshot(
        &self,
        window: crate::types::WindowId,
    ) -> Option<crate::backend::wayland::compositor::protocols::foreign_toplevel::ToplevelSnapshot>
    {
        self.foreign_toplevel_snapshot(window)
    }

    fn foreign_toplevel_request(
        &mut self,
        window: crate::types::WindowId,
        request: crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelRequest,
    ) {
        use crate::backend::wayland::commands::WmCommand;
        use crate::backend::wayland::compositor::protocols::foreign_toplevel::ForeignToplevelRequest;

        let command = match request {
            ForeignToplevelRequest::Activate => WmCommand::FocusWindow(window),
            ForeignToplevelRequest::Close => WmCommand::CloseWindow(window),
            ForeignToplevelRequest::SetMaximized(maximized) => WmCommand::SetMaximized {
                win: window,
                maximized,
            },
            ForeignToplevelRequest::SetMinimized(minimized) => WmCommand::SetMinimized {
                win: window,
                minimized,
            },
            ForeignToplevelRequest::SetFullscreen(fullscreen) => WmCommand::SetFullscreen {
                win: window,
                fullscreen,
            },
        };
        self.push_command(command);
    }
}

crate::delegate_foreign_toplevel_management!(WaylandState);

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Point;

    #[test]
    fn fifo_only_commit_requests_a_refresh() {
        assert_eq!(
            classify_surface_commit(false, false, true),
            SurfaceCommitService::FrameCallbacks
        );
    }

    #[test]
    fn pixel_changes_take_precedence_over_protocol_only_work() {
        assert_eq!(
            classify_surface_commit(true, true, true),
            SurfaceCommitService::Render
        );
    }

    #[test]
    fn empty_commit_does_not_schedule_work() {
        assert_eq!(
            classify_surface_commit(false, false, false),
            SurfaceCommitService::None
        );
    }

    #[test]
    fn remove_constraint_applies_matching_cursor_position_hint() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let pointer = state.seat.get_pointer().unwrap();

        state.runtime.pointer_location = Point::from((500.0, 500.0));
        pointer.set_location(Point::from((500.0, 500.0)));

        let dummy_surface =
            smithay::reexports::wayland_server::protocol::wl_surface::WlSurface::from_id(
                &state.display_handle.clone(),
                smithay::reexports::wayland_server::backend::ObjectId::null(),
            )
            .unwrap();

        // Without an active window/surface origin, unmatching surface hint remains untouched
        PointerConstraintsHandler::remove_constraint(&mut state, &dummy_surface, &pointer);

        assert_eq!(state.runtime.pointer_location, Point::from((500.0, 500.0)));
        assert_eq!(pointer.current_location(), Point::from((500.0, 500.0)));
    }
}
