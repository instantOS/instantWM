use crate::backend::{BackendState, WaylandBackendData, X11BackendData};
use crate::contexts::{CoreCtx, WmCtx, WmCtxWayland, WmCtxX11};
use crate::core_state::{CoreState, PendingWork};
use crate::systray::NativeMenuRequestSlot;

pub struct Wm<B: BackendState> {
    pub core: CoreState,
    pub work: PendingWork,
    pub backend: B,
    pub running: bool,
    pub bar: crate::bar::BarState,
    pub focus: crate::client::focus::FocusState,
}

impl<B: BackendState> Wm<B> {
    pub fn new(backend: B) -> Self {
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
        self.core_ctx().configure_tray_icons();
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Borrow the backend-neutral core state as a [`CoreCtx`].
    ///
    /// Use this when an operation needs only core state; [`WmCtx`] is the
    /// entry point whenever the backend is involved too.
    pub fn core_ctx(&mut self) -> CoreCtx<'_> {
        self.split_core_and_backend().0
    }

    /// Split `Wm` into its two disjoint halves: the backend-neutral core
    /// state and the owned backend.
    ///
    /// This is the one place that names the fields making up a [`CoreCtx`];
    /// [`Wm::core_ctx`] and backend context construction are built on it, so adding a
    /// field touches a single spot.
    pub(crate) fn split_core_and_backend(&mut self) -> (CoreCtx<'_>, &mut B) {
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

    /// Which backend is driving.
    ///
    /// Mirrors [`WmCtx::backend_kind`] for callers that only need the kind and
    /// must not borrow the whole context (the shared tick, for instance).
    pub fn backend_kind(&self) -> crate::backend::BackendKind {
        B::KIND
    }
}

pub type X11Wm = Wm<X11BackendData>;
pub type WaylandWm = Wm<WaylandBackendData>;

impl X11Wm {
    pub fn x11_ctx(&mut self) -> WmCtx<'_> {
        let (core, data) = self.split_core_and_backend();
        WmCtx::X11(WmCtxX11 {
            core,
            x11: crate::backend::x11::X11BackendRef::new(&data.conn, data.screen_num),
            x11_runtime: &mut data.x11_runtime,
            xembed_tray: &mut data.xembed_tray,
        })
    }
}
impl WaylandWm {
    pub fn wayland_ctx<'a>(
        &'a mut self,
        state: &'a mut crate::backend::wayland::compositor::WaylandState,
    ) -> WmCtx<'a> {
        let (core, data) = self.split_core_and_backend();
        WmCtx::Wayland(WmCtxWayland {
            core,
            wayland: crate::backend::wayland::WaylandBackend::new(state),
            bar_renderer: &mut data.bar_renderer,
        })
    }
}
