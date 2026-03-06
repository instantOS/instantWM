//! Window-manager root object.
//!
//! `Wm` owns all runtime state and the active backend.

use crate::backend::x11::{X11Backend, X11BackendRef};
use crate::backend::{Backend, BackendRef};
use crate::contexts::{CoreCtx, WaylandCtx, WmCtx, WmCtxWayland, WmCtxX11};
use crate::globals::{Globals, X11RuntimeConfig};
use crate::types::{Systray, WaylandSystray, WaylandSystrayMenu};

pub struct Wm {
    pub g: Globals,
    pub backend: Backend,
    pub running: bool,
    pub bar: crate::bar::BarState,
    pub bar_painter: crate::bar::wayland::WaylandBarPainter,
    pub focus: crate::client::focus::FocusState,
    // X11-specific state
    pub x11_runtime: X11RuntimeConfig,
    pub systray: Option<Systray>,
    // Wayland-specific state
    pub wayland_systray: WaylandSystray,
    pub wayland_systray_menu: Option<WaylandSystrayMenu>,
    pub wayland_systray_runtime: Option<crate::wayland_systray::WaylandSystrayRuntime>,
}

impl Wm {
    pub fn new(backend: Backend) -> Self {
        Self {
            g: Globals::default(),
            backend,
            running: true,
            bar: crate::bar::BarState::default(),
            bar_painter: crate::bar::wayland::WaylandBarPainter::default(),
            focus: crate::client::focus::FocusState::default(),
            x11_runtime: X11RuntimeConfig::default(),
            systray: None,
            wayland_systray: WaylandSystray::default(),
            wayland_systray_menu: None,
            wayland_systray_runtime: None,
        }
    }

    pub fn x11(&self) -> &X11Backend {
        match &self.backend {
            Backend::X11(x11) => x11,
            Backend::Wayland(_) => panic!("X11 backend requested while running Wayland"),
        }
    }

    pub fn x11_mut(&mut self) -> &mut X11Backend {
        match &mut self.backend {
            Backend::X11(x11) => x11,
            Backend::Wayland(_) => panic!("X11 backend requested while running Wayland"),
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn ctx(&mut self) -> WmCtx<'_> {
        let backend = BackendRef::from_backend(&self.backend);
        let core = CoreCtx::new(
            &mut self.g,
            &mut self.running,
            &mut self.bar,
            &mut self.focus,
        );
        match &mut self.backend {
            Backend::X11(x11) => WmCtx::X11(WmCtxX11 {
                core,
                backend,
                x11: X11BackendRef::new(&x11.conn, x11.screen_num),
                x11_runtime: &mut self.x11_runtime,
                systray: self.systray.as_mut(),
            }),
            Backend::Wayland(wayland) => WmCtx::Wayland(WmCtxWayland {
                core,
                backend,
                wayland: WaylandCtx { backend: wayland },
                xwayland: None,
                wayland_systray: &mut self.wayland_systray,
                wayland_systray_menu: self.wayland_systray_menu.as_mut(),
            }),
        }
    }
}
