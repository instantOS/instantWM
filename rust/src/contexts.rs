//! Context split for backend-specific operations.
//!
//! Core state (`Globals`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit `X11Ctx` / `WaylandCtx` instead of runtime checks.

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

    pub fn reborrow(&mut self) -> CoreCtx<'_> {
        CoreCtx::new(self.g, self.running, self.bar, self.bar_painter, self.focus)
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

    pub fn backend(&self) -> &crate::backend::BackendRef<'_> {
        match self {
            WmCtx::X11(ctx) => &ctx.backend,
            WmCtx::Wayland(ctx) => &ctx.backend,
        }
    }

    pub fn backend_mut(&mut self) -> &mut crate::backend::BackendRef<'a> {
        match self {
            WmCtx::X11(ctx) => &mut ctx.backend,
            WmCtx::Wayland(ctx) => &mut ctx.backend,
        }
    }

    pub fn backend_kind(&self) -> crate::backend::BackendKind {
        self.backend().kind()
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

    pub fn x11_conn(&self) -> Option<crate::globals::X11Conn<'_>> {
        self.backend()
            .x11_conn()
            .map(|(conn, screen_num)| crate::globals::X11Conn::new(conn, screen_num))
    }

    pub fn resize_client(&self, win: WindowId, rect: Rect) {
        self.backend().resize_window(win, rect);
    }

    pub fn set_border(&self, win: WindowId, width: i32) {
        self.backend().set_border_width(win, width);
    }

    /// Get a reference to CoreCtx.
    pub fn core(&self) -> &CoreCtx<'_> {
        match self {
            WmCtx::X11(ctx) => &ctx.core,
            WmCtx::Wayland(ctx) => &ctx.core,
        }
    }

    /// Get a mutable reference to CoreCtx.
    pub fn core_mut(&mut self) -> &mut CoreCtx<'_> {
        match self {
            WmCtx::X11(ctx) => &mut ctx.core,
            WmCtx::Wayland(ctx) => &mut ctx.core,
        }
    }

    /// Get a reference to X11Ctx (only valid for X11 backend).
    pub fn x11(&self) -> Option<&X11Ctx<'_>> {
        match self {
            WmCtx::X11(ctx) => Some(&ctx.x11),
            WmCtx::Wayland(_) => None,
        }
    }

    /// Get a mutable reference to X11Ctx (only valid for X11 backend).
    pub fn x11_mut(&mut self) -> Option<&mut X11Ctx<'_>> {
        match self {
            WmCtx::X11(ctx) => Some(&mut ctx.x11),
            WmCtx::Wayland(_) => None,
        }
    }

    /// Get a reference to WaylandCtx (only valid for Wayland backend).
    pub fn wayland(&self) -> Option<&WaylandCtx<'_>> {
        match self {
            WmCtx::X11(_) => None,
            WmCtx::Wayland(ctx) => Some(&ctx.wayland),
        }
    }

    /// Get a mutable reference to WaylandCtx (only valid for Wayland backend).
    pub fn wayland_mut(&mut self) -> Option<&mut WaylandCtx<'_>> {
        match self {
            WmCtx::X11(_) => None,
            WmCtx::Wayland(ctx) => Some(&mut ctx.wayland),
        }
    }

    /// Get an Option of WaylandCtx for backend-agnostic code.
    pub fn wayland_ctx_opt(&self) -> Option<&WaylandCtx<'_>> {
        self.wayland()
    }

    /// Convert to &mut WmCtxX11, panics if not X11.
    pub fn as_x11_mut(&mut self) -> &mut WmCtxX11<'_> {
        match self {
            WmCtx::X11(ctx) => ctx,
            WmCtx::Wayland(_) => panic!("as_x11_mut called on Wayland backend"),
        }
    }

    /// Execute a closure with &mut CoreCtx and &X11Ctx (X11 only).
    pub fn with_x11_ctx<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut CoreCtx<'_>, &X11Ctx<'_>) -> R,
    {
        match self {
            WmCtx::X11(ctx) => f(&mut ctx.core, &ctx.x11),
            WmCtx::Wayland(_) => panic!("with_x11_ctx called on Wayland backend"),
        }
    }

    /// Execute a closure with &mut CoreCtx and &X11Ctx (X11 only), returning mutable reference.
    pub fn with_x11_ctx_mut<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut CoreCtx<'_>, &mut X11Ctx<'_>) -> R,
    {
        match self {
            WmCtx::X11(ctx) => f(&mut ctx.core, &mut ctx.x11),
            WmCtx::Wayland(_) => panic!("with_x11_ctx_mut called on Wayland backend"),
        }
    }

    /// Convert to &mut WmCtxWayland, panics if not Wayland.
    pub fn as_wayland_mut(&mut self) -> &mut WmCtxWayland<'_> {
        match self {
            WmCtx::X11(_) => panic!("as_wayland_mut called on X11 backend"),
            WmCtx::Wayland(ctx) => ctx,
        }
    }

    /// Spawn a command using the core context.
    pub fn spawn(&self, cmd: crate::config::commands::Cmd) {
        crate::util::spawn(self.core(), self.wayland_ctx_opt(), cmd);
    }
}
