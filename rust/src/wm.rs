//! Window-manager root object.
//!
//! `Wm` owns all runtime state and the active backend.

use crate::backend::x11::X11Backend;
use crate::backend::{Backend, BackendRef};
use crate::contexts::{CoreCtx, WaylandCtx, WmCtx, WmCtxWayland, WmCtxX11, X11Ctx};
use crate::globals::Globals;

pub struct Wm {
    pub g: Globals,
    pub backend: Backend,
    pub running: bool,
    pub bar: crate::bar::BarState,
    pub bar_painter: crate::bar::wayland::WaylandBarPainter,
    pub focus: crate::client::focus::FocusState,
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
        match &self.backend {
            Backend::X11(x11) => WmCtx::X11(WmCtxX11 {
                core,
                backend,
                x11: X11Ctx {
                    conn: &x11.conn,
                    screen_num: x11.screen_num,
                },
            }),
            Backend::Wayland(wayland) => WmCtx::Wayland(WmCtxWayland {
                core,
                backend,
                wayland: WaylandCtx { backend: wayland },
                xwayland: None,
            }),
        }
    }
}
