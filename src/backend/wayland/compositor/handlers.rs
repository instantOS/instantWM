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
        selection::data_device::{ClientDndGrabHandler, ServerDndGrabHandler},
        shm::ShmHandler,
        xwayland_keyboard_grab::XWaylandKeyboardGrabHandler,
        xwayland_shell::XWaylandShellHandler,
    },
    xwayland::XWaylandClientData,
};

use super::{
    focus::KeyboardFocusTarget,
    state::{WaylandClientState, WaylandRuntime},
};

impl CompositorHandler for WaylandRuntime {
    fn compositor_state(&mut self) -> &mut smithay::wayland::compositor::CompositorState {
        &mut self.state.compositor_state
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
        on_commit_buffer_handler::<Self>(surface);

        // Check if this commit is from a pending toplevel that has finally
        // produced a buffer.  If so, promote it to a managed window.
        if let Some(pos) = self
            .state
            .pending_toplevels
            .iter()
            .position(|t| t.wl_surface() == surface)
        {
            let has_buffer =
                smithay::backend::renderer::utils::with_renderer_surface_state(surface, |state| {
                    state.buffer().is_some()
                })
                .unwrap_or(false);
            if has_buffer {
                let toplevel = self.state.pending_toplevels.swap_remove(pos);
                let _ = self.state.map_new_toplevel(toplevel);
            }
        }

        self.state.popups.commit(surface);

        if let Some(popup) = self.state.popups.find_popup(surface)
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

        if let Some(window) = self
            .state
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&root))
            .cloned()
        {
            window.on_commit();
        }

        super::layer_shell::handle_layer_commit(&mut self.state, surface);
    }
}

impl ShmHandler for WaylandRuntime {
    fn shm_state(&self) -> &smithay::wayland::shm::ShmState {
        &self.state.shm_state
    }
}

impl BufferHandler for WaylandRuntime {
    fn buffer_destroyed(
        &mut self,
        _buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    ) {
    }
}

impl DmabufHandler for WaylandRuntime {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.state.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: ImportNotifier,
    ) {
        // Tag the dmabuf with the render node so clients know which device to use.
        if let Some(node) = self.state.render_node {
            dmabuf.set_node(node);
        }

        let imported = self
            .renderer
            .import_dmabuf(&dmabuf, None)
            .ok()
            .is_some();
        if imported {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}

impl ClientDndGrabHandler for WaylandRuntime {
    fn started(
        &mut self,
        _source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        _seat: smithay::input::Seat<Self>,
    ) {
        self.state.dnd_icon = icon;
    }

    fn dropped(
        &mut self,
        _icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        _accepted: bool,
        _seat: smithay::input::Seat<Self>,
    ) {
        self.state.dnd_icon = None;
    }
}

impl ServerDndGrabHandler for WaylandRuntime {
    fn send(
        &mut self,
        _mime_type: String,
        _fd: std::os::unix::io::OwnedFd,
        _seat: smithay::input::Seat<Self>,
    ) {
    }
}

impl OutputHandler for WaylandRuntime {}

impl smithay::wayland::foreign_toplevel_list::ForeignToplevelListHandler for WaylandRuntime {
    fn foreign_toplevel_list_state(
        &mut self,
    ) -> &mut smithay::wayland::foreign_toplevel_list::ForeignToplevelListState {
        &mut self.state.foreign_toplevel_list_state
    }
}

smithay::delegate_foreign_toplevel_list!(WaylandRuntime);

impl XWaylandShellHandler for WaylandRuntime {
    fn xwayland_shell_state(
        &mut self,
    ) -> &mut smithay::wayland::xwayland_shell::XWaylandShellState {
        &mut self.state.xwayland_shell_state
    }
}

impl XWaylandKeyboardGrabHandler for WaylandRuntime {
    fn keyboard_focus_for_xsurface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<Self::KeyboardFocus> {
        let win = self.state.window_id_for_surface(surface)?;
        let window = self.state.window_index.get(&win)?;
        Some(KeyboardFocusTarget::Window(window.clone()))
    }
}

impl smithay::wayland::idle_inhibit::IdleInhibitHandler for WaylandRuntime {
    fn inhibit(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.state.idle_inhibiting_surfaces.insert(surface);
        log::debug!("idle inhibited for surface");
    }

    fn uninhibit(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        self.state.idle_inhibiting_surfaces.remove(&surface);
        log::debug!("idle uninhibited for surface");
    }
}
