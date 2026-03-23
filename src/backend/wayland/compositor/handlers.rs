use smithay::{
    backend::renderer::ImportDma,
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::PopupKind,
    reexports::wayland_server::Client,
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorHandler, get_parent, is_sync_subsurface},
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
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
        on_commit_buffer_handler::<Self>(surface);

        // Check if this commit is from a pending toplevel that has finally
        // produced a buffer.  If so, promote it to a managed window.
        if let Some(pos) = self
            .pending_toplevels
            .iter()
            .position(|t| t.wl_surface() == surface)
        {
            let has_buffer =
                smithay::backend::renderer::utils::with_renderer_surface_state(surface, |state| {
                    state.buffer().is_some()
                })
                .unwrap_or(false);

            let (w, h) = smithay::backend::renderer::utils::with_renderer_surface_state(surface, |state| {
                state.surface_size().map(|s| (s.w, s.h)).unwrap_or((0, 0))
            }).unwrap_or((0, 0));

            if has_buffer {
                let toplevel = self.pending_toplevels.swap_remove(pos);
                let win = self.map_new_toplevel(toplevel);

                // If the requested dimensions are 1x1 or 0x0, it's likely a dummy
                // window (e.g. some clipboard tools use this to gain focus).
                // Force it to float so it doesn't cause a layout shift.
                if w <= 1 && h <= 1 {
                    if let Some(g) = self.globals_mut() {
                        if let Some(client) = g.clients.get_mut(&win) {
                            client.is_floating = true;
                        }
                    }
                }
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

        if let Some(window) = self
            .space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(&root))
            .cloned()
        {
            window.on_commit();
        }

        // Mark content dirty so the DRM backend schedules a render on the
        // next VBlank.  The damage tracker will handle GPU-efficiency.
        self.content_dirty_pending = true;

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

        if let Some(renderer) = self.renderer.as_mut() {
            // DRM path: renderer is available, import immediately.
            let imported = renderer.import_dmabuf(&dmabuf, None).ok().is_some();
            if imported {
                let _ = notifier.successful::<Self>();
            } else {
                notifier.failed();
            }
        } else {
            // Winit path: renderer is owned by the winit backend, not stored
            // in state. Queue the import — it will be processed during
            // render_frame when the renderer is available via backend.bind().
            self.pending_dmabuf_imports.push((dmabuf, notifier));
        }
    }
}

impl ClientDndGrabHandler for WaylandState {
    fn started(
        &mut self,
        _source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        _seat: smithay::input::Seat<Self>,
    ) {
        self.dnd_icon = icon;
    }

    fn dropped(
        &mut self,
        _icon: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
        _accepted: bool,
        _seat: smithay::input::Seat<Self>,
    ) {
        self.dnd_icon = None;
    }
}

impl ServerDndGrabHandler for WaylandState {
    fn send(
        &mut self,
        _mime_type: String,
        _fd: std::os::unix::io::OwnedFd,
        _seat: smithay::input::Seat<Self>,
    ) {
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
        if let Some(win) = self.window_id_for_surface(surface)
            && let Some(window) = self.window_index.get(&win)
        {
            return Some(KeyboardFocusTarget::Window(window.clone()));
        }
        // For unmanaged X11 surfaces (like dmenu), search in the space
        if let Some(window) = self.window_for_surface(surface) {
            return Some(KeyboardFocusTarget::Window(window));
        }

        // Fallback: If XWayland requests a grab for a surface that isn't mapped
        // as a full window (e.g., grabbing the root window or a dummy surface),
        // we must still allow the grab by returning the raw WlSurface.
        Some(KeyboardFocusTarget::WlSurface(surface.clone()))
    }
}

impl KeyboardShortcutsInhibitHandler for WaylandState {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, _inhibitor: KeyboardShortcutsInhibitor) {
        // We handle the inhibitor implicitly via KeyboardShortcutsInhibitState::keyboard_shortcuts_inhibited
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
