use smithay::{
    backend::renderer::ImportDma,
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::PopupKind,
    reexports::wayland_server::Client,
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorHandler, get_parent, is_sync_subsurface},
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
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
        // Surface commits must drive redraws in the DRM backend; otherwise
        // removing the vblank self-redraw loop stalls client updates until
        // some unrelated input/layout path dirties the outputs.
        self.request_render();
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
                let toplevel = self.runtime.pending_toplevels.swap_remove(pos);
                let _ = self.map_new_toplevel(toplevel);
            }
        }

        self.popups.commit(surface);

        if let Some(popup) = self.popups.find_popup(surface)
            && let PopupKind::Xdg(ref popup_surface) = popup
            && !popup_surface.is_initial_configure_sent()
        {
            let _ = popup_surface.send_configure();
        }

        // Skip sync subsurfaces - they don't receive their own commits
        if is_sync_subsurface(surface) {
            return;
        }

        // Find the root surface by walking up the surface tree
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        // Only call on_commit for the root surface, not for subsurfaces
        if surface != &root {
            return;
        }

        let committed_window = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&root))
            .cloned();
        if let Some(window) = committed_window {
            window.on_commit();
            if let Some(id) = window
                .user_data()
                .get::<super::state::WindowIdMarker>()
                .map(|marker| marker.id)
            {
                self.sync_client_geometry_from_window(id);
            }
        }

        super::layer_shell::handle_layer_commit(self, surface);
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
