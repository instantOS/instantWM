use crate::backend::Backend;
use crate::contexts::{CoreCtx, WmCtx, WmCtxWayland, WmCtxX11};
use crate::core_state::{CoreState, PendingWork};
use crate::systray::NativeMenuRequestSlot;

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

    /// Start the StatusNotifier worker if it is not already running.
    ///
    /// Called by both backends during bootstrap. `native_menu_request` is the
    /// compositor-provided slot used to claim an item's native menu toplevel;
    /// backends without that capability (X11, where items position their own
    /// menus) pass `None`. `wake` pings the backend event loop whenever the
    /// worker publishes updates.
    pub(crate) fn start_systray(
        &mut self,
        native_menu_request: Option<NativeMenuRequestSlot>,
        wake: Option<calloop::ping::Ping>,
    ) {
        self.bar.systray_host.start(native_menu_request, wake);
    }

    /// Drain StatusNotifier worker events. Returns `true` when tray content
    /// changed and the bar must be redrawn.
    pub fn poll_systray(&mut self) -> bool {
        let changed = self.bar.systray_host.poll();
        if changed {
            self.bar.mark_dirty();
        }
        changed
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
                xembed_tray: &mut data.xembed_tray,
            }),
            Backend::Wayland(data) => WmCtx::Wayland(WmCtxWayland {
                core,
                wayland: &data.backend,
            }),
        }
    }
}
