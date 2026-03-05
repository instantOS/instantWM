//! Context split for backend-specific operations.
//!
//! Core state (`Globals`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit `X11Ctx` / `WaylandCtx` instead of runtime checks.

use std::ops::Deref;

use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::globals::Globals;
use crate::types::{Client, Rect, WindowId};
use x11rb::rust_connection::RustConnection;

pub struct CoreCtx<'a> {
    pub g: &'a mut Globals,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub focus: &'a mut FocusState,
}

impl<'a> CoreCtx<'a> {
    pub fn new(
        g: &'a mut Globals,
        running: &'a mut bool,
        bar: &'a mut BarState,
        focus: &'a mut FocusState,
    ) -> Self {
        Self {
            g,
            running,
            bar,
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

    pub fn reborrow(&mut self) -> CoreCtx<'_> {
        CoreCtx {
            g: self.g,
            running: self.running,
            bar: self.bar,
            focus: self.focus,
        }
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

impl<'a> WmCtxX11<'a> {
    pub fn reborrow(&mut self) -> WmCtxX11<'_> {
        WmCtxX11 {
            core: self.core.reborrow(),
            backend: self.backend.reborrow(),
            x11: X11Ctx {
                conn: self.x11.conn,
                screen_num: self.x11.screen_num,
            },
        }
    }

    pub fn selected_client(&self) -> Option<WindowId> {
        self.core.selected_client()
    }
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
    // Backend-agnostic core accessors - use these for common operations

    /// Access the shared core context immutably.
    pub fn core(&self) -> &CoreCtx<'_> {
        match self {
            WmCtx::X11(ctx) => &ctx.core,
            WmCtx::Wayland(ctx) => &ctx.core,
        }
    }

    /// Access the shared core context mutably.
    pub fn core_mut(&mut self) -> &mut CoreCtx<'a> {
        match self {
            WmCtx::X11(ctx) => &mut ctx.core,
            WmCtx::Wayland(ctx) => &mut ctx.core,
        }
    }

    pub fn selected_client(&self) -> Option<WindowId> {
        self.core().selected_client()
    }

    pub fn client(&self, win: WindowId) -> Option<&Client> {
        self.core().client(win)
    }

    pub fn g(&self) -> &Globals {
        self.core().g
    }

    pub fn g_mut(&mut self) -> &mut Globals {
        self.core_mut().g
    }

    pub fn quit(&mut self) {
        self.core_mut().quit();
    }

    // Backend-agnostic operations (delegate through BackendOps)

    pub fn backend(&self) -> &BackendRef<'_> {
        match self {
            WmCtx::X11(ctx) => &ctx.backend,
            WmCtx::Wayland(ctx) => &ctx.backend,
        }
    }

    pub fn backend_mut(&mut self) -> &mut BackendRef<'a> {
        match self {
            WmCtx::X11(ctx) => &mut ctx.backend,
            WmCtx::Wayland(ctx) => &mut ctx.backend,
        }
    }

    pub fn flush(&self) {
        self.backend().flush();
    }

    pub fn raise(&self, win: WindowId) {
        self.backend().raise_window(win);
    }

    pub fn restack(&self, wins: &[WindowId]) {
        self.backend().restack(wins);
    }

    pub fn resize_client(&self, win: WindowId, rect: Rect) {
        self.backend().resize_window(win, rect);
    }

    pub fn set_border(&self, win: WindowId, width: i32) {
        self.backend().set_border_width(win, width);
    }

    pub fn map_window(&self, win: WindowId) {
        self.backend().map_window(win);
    }

    pub fn unmap_window(&self, win: WindowId) {
        self.backend().unmap_window(win);
    }

    pub fn set_focus(&self, win: WindowId) {
        self.backend().set_focus(win);
    }

    // For backend-specific operations, use match on the enum directly
    // instead of accessor methods that return Option.
}

impl<'a> Deref for WmCtx<'a> {
    type Target = CoreCtx<'a>;

    fn deref(&self) -> &Self::Target {
        self.core()
    }
}
