use crate::backend::Backend;
use crate::contexts::{CoreCtx, WmCtx, WmCtxWayland, WmCtxX11};
use crate::core_state::{CoreState, PendingWork};

pub struct Wm {
    pub core: CoreState,
    pub work: PendingWork,
    pub backend: Backend,
    pub running: bool,
    pub bar: crate::bar::BarState,
    pub focus: crate::client::focus::FocusState,
}

impl Wm {
    pub fn new(backend: Backend) -> Self {
        Self {
            core: CoreState::default(),
            work: PendingWork::default(),
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
            &mut self.core,
            &mut self.work,
            &mut self.running,
            &mut self.bar,
            &mut self.focus,
        );
        match &mut self.backend {
            Backend::X11(data) => WmCtx::X11(WmCtxX11 {
                core,
                x11: crate::backend::x11::X11BackendRef::new(&data.conn, data.screen_num),
                x11_runtime: &mut data.x11_runtime,
                xembed_tray: data.xembed_tray.as_mut(),
            }),
            Backend::Wayland(data) => WmCtx::Wayland(WmCtxWayland {
                core,
                wayland: &data.backend,
            }),
        }
    }
}
