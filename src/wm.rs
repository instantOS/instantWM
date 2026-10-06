use crate::backend::{BackendState, WaylandBackendData, X11BackendData};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::core_state::WmCore;
use crate::systray::NativeMenuRequestSlot;

pub struct Wm<B: BackendState> {
    pub core: WmCore,
    pub backend: B,
}

impl<B: BackendState> Wm<B> {
    pub fn new(backend: B) -> Self {
        Self {
            core: WmCore::default(),
            backend,
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
        self.core.bar.systray_host.start(native_menu_request, wake);
        self.core_ctx().configure_tray_icons();
    }

    pub fn quit(&mut self) {
        self.core.running = false;
    }

    /// Borrow the backend-neutral core state as a [`WmCore`].
    ///
    /// Use this when an operation needs only core state; [`WmCtx`] is the
    /// entry point whenever the backend is involved too.
    pub fn core_ctx(&mut self) -> &mut WmCore {
        &mut self.core
    }

    pub(crate) fn split_core_and_backend(&mut self) -> (&mut WmCore, &mut B) {
        (&mut self.core, &mut self.backend)
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
