//! Context split for backend-specific operations.
//!
//! Core state (`Globals`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit `X11Ctx` / `WaylandCtx` instead of runtime checks.

use crate::backend::BackendRef;
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::globals::Globals;
use crate::types::{Client, WindowId};
use x11rb::rust_connection::RustConnection;

pub struct CoreCtx<'a> {
    pub g: &'a mut Globals,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub bar_painter: &'a mut crate::bar::wayland::WaylandBarPainter,
    pub focus: &'a mut FocusState,
}

impl<'a> CoreCtx<'a> {
    pub fn new(
        g: &'a mut Globals,
        running: &'a mut bool,
        bar: &'a mut BarState,
        bar_painter: &'a mut crate::bar::wayland::WaylandBarPainter,
        focus: &'a mut FocusState,
    ) -> Self {
        Self {
            g,
            running,
            bar,
            bar_painter,
            focus,
        }
    }

    pub fn quit(&mut self) {
        *self.running = false;
    }

    pub fn client(&self, win: WindowId) -> Option<&Client> {
        self.g.clients.get(&win)
    }

    pub fn client_mut(&mut self, win: WindowId) -> Option<&mut Client> {
        self.g.clients.get_mut(&win)
    }

    pub fn selected_client(&self) -> Option<WindowId> {
        self.g.selected_win()
    }

    pub fn set_selected_client(&mut self, win: Option<WindowId>) {
        self.g.selected_monitor_mut().sel = win;
    }
}

pub struct X11Ctx<'a> {
    pub conn: &'a RustConnection,
    pub screen_num: usize,
}

pub struct WaylandCtx<'a> {
    pub backend: &'a crate::backend::wayland::WaylandBackend,
}

pub struct XwaylandCtx<'a> {
    pub xdisplay: u32,
    pub xwm: Option<&'a smithay::xwayland::X11Wm>,
}

pub struct WmCtxX11<'a> {
    pub core: CoreCtx<'a>,
    pub backend: BackendRef<'a>,
    pub x11: X11Ctx<'a>,
}

pub struct WmCtxWayland<'a> {
    pub core: CoreCtx<'a>,
    pub backend: BackendRef<'a>,
    pub wayland: WaylandCtx<'a>,
    pub xwayland: Option<XwaylandCtx<'a>>,
}

pub enum WmCtx<'a> {
    X11(WmCtxX11<'a>),
    Wayland(WmCtxWayland<'a>),
}

impl<'a> WmCtx<'a> {
    pub fn selected_client(&self) -> Option<WindowId> {
        match self {
            WmCtx::X11(ctx) => ctx.core.selected_client(),
            WmCtx::Wayland(ctx) => ctx.core.selected_client(),
        }
    }

    pub fn g(&self) -> &Globals {
        match self {
            WmCtx::X11(ctx) => ctx.core.g,
            WmCtx::Wayland(ctx) => ctx.core.g,
        }
    }

    pub fn g_mut(&mut self) -> &mut Globals {
        match self {
            WmCtx::X11(ctx) => ctx.core.g,
            WmCtx::Wayland(ctx) => ctx.core.g,
        }
    }
}
