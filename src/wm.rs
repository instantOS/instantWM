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

    /// Rebuild backend-owned bar resources after config changes.
    ///
    /// Thin wrapper over [`WmCtx::reinit_bar_resources`], which owns the
    /// choreography (X11 bakes schemes/fonts into the DrawContext at
    /// startup; both backends re-project bar metrics onto each monitor).
    /// Kept for `&mut Wm` call sites; prefer the `WmCtx` method when a
    /// context is already in hand.
    pub fn reinit_bar_resources(&mut self) {
        self.ctx().reinit_bar_resources();
    }

    /// Borrow the backend-neutral core state as a [`CoreCtx`].
    ///
    /// Use this when an operation needs only core state; [`Wm::ctx`] is the
    /// entry point whenever the backend is involved too.
    pub fn core_ctx(&mut self) -> CoreCtx<'_> {
        self.split_core_and_backend().0
    }

    /// Split `Wm` into its two disjoint halves: the backend-neutral core
    /// state and the owned backend.
    ///
    /// This is the one place that names the fields making up a [`CoreCtx`];
    /// both [`Wm::core_ctx`] and [`Wm::ctx`] are built on it, so adding a
    /// field touches a single spot.
    fn split_core_and_backend(&mut self) -> (CoreCtx<'_>, &mut Backend) {
        let Self {
            core,
            work,
            running,
            bar,
            focus,
            backend,
            ..
        } = self;
        (CoreCtx::new(core, work, running, bar, focus), backend)
    }

    pub fn ctx(&mut self) -> WmCtx<'_> {
        let (core, backend) = self.split_core_and_backend();
        match backend {
            Backend::X11(data) => WmCtx::X11(WmCtxX11 {
                core,
                x11: crate::backend::x11::X11BackendRef::new(&data.conn, data.screen_num),
                x11_runtime: &mut data.x11_runtime,
                xembed_tray: &mut data.xembed_tray,
            }),
            Backend::Wayland(data) => WmCtx::Wayland(WmCtxWayland {
                core,
                wayland: &data.backend,
                bar_renderer: &mut data.bar_renderer,
            }),
        }
    }
}
