//! Window-manager root object.
//!
//! `Wm` owns all runtime state and the active backend.

use crate::backend::Backend;
use crate::contexts::{CoreCtx, WmCtx, WmCtxWayland, WmCtxX11};
use crate::globals::Globals;

pub struct Wm {
    pub g: Globals,
    pub backend: Backend,
    pub running: bool,
    pub bar: crate::bar::BarState,
    pub focus: crate::client::focus::FocusState,
}

impl Wm {
    pub fn new(backend: Backend) -> Self {
        Self {
            g: Globals::default(),
            backend,
            running: true,
            bar: crate::bar::BarState::default(),
            focus: crate::client::focus::FocusState::default(),
        }
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn ctx(&mut self) -> WmCtx<'_> {
        let core = CoreCtx::new(
            &mut self.g,
            &mut self.running,
            &mut self.bar,
            &mut self.focus,
        );
        match &mut self.backend {
            Backend::X11(data) => WmCtx::X11(WmCtxX11 {
                core,
                x11: crate::backend::x11::X11BackendRef::new(&data.conn, data.screen_num),
                x11_runtime: &mut data.x11_runtime,
                systray: data.systray.as_mut(),
            }),
            Backend::Wayland(data) => WmCtx::Wayland(WmCtxWayland {
                core,
                wayland: &data.backend,
                wayland_systray: &mut data.wayland_systray,
                wayland_systray_menu: data.wayland_systray_menu.as_mut(),
            }),
        }
    }
}
