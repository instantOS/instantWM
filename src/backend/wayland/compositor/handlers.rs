use smithay::{
    backend::renderer::ImportDma,
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::PopupKind,
    reexports::wayland_server::{Client, Resource},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorHandler, SurfaceAttributes, TraversalAction, get_parent, is_sync_subsurface,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
        output::OutputHandler,
        pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint},
        seat::WaylandFocus,
        shm::ShmHandler,
        xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
        xwayland_shell::XWaylandShellHandler,
    },
    xwayland::XWaylandClientData,
};

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
                        launch_pid: None,
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

        // Ask Smithay which outputs contain the root window rather than
        // reconstructing that relationship from geometry in the runtime.
        let committed_window = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&root))
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
                self.sync_client_size_from_window(id);
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
        if attrs.buffer.is_some() || attrs.buffer_delta.is_some() || !attrs.damage.is_empty() {
            SurfaceCommitService::Render
        } else if !attrs.frame_callbacks.is_empty() {
            SurfaceCommitService::FrameCallbacks
        } else {
            SurfaceCommitService::None
        }
    })
}

fn service_surface_commit(
    state: &mut WaylandState,
    service: SurfaceCommitService,
    window: Option<&smithay::desktop::Window>,
    layer_output: Option<&smithay::output::Output>,
) {
    match service {
        SurfaceCommitService::Render => match window {
            Some(window) => state.request_window_render(window),
            None => match layer_output {
                Some(output) => state.request_output_render(output),
                None => state.request_render(),
            },
        },
        SurfaceCommitService::FrameCallbacks => match window {
            Some(window) => state.request_window_frame_callbacks(window),
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

smithay::delegate_foreign_toplevel_list!(WaylandState);

impl XWaylandShellHandler for WaylandState {
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XWaylandKeyboardGrabHandler for WaylandState {
    fn keyboard_focus_for_xsurface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Self::KeyboardFocus> {
        let win = self.window_id_for_surface(surface)?;
        let window = self.window_index.get(&win)?;
        Some(KeyboardFocusTarget::Window(window.clone()))
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

        let Some(origin) = self.pointer_constraint_surface_origin(surface) else {
            return;
        };
        let target = origin + location;
        pointer.set_location(target);
        self.runtime.pointer_location = target;
    }
}
