//! Context split for backend-specific operations.
//!
//! Core state (`Globals`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit `X11BackendRef` / `WaylandCtx` instead of runtime checks.

use std::ops::{Deref, DerefMut};

use crate::backend::x11::X11BackendRef;
use crate::backend::BackendKind;
use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::bar::BarState;
use crate::bar::{draw_bar, draw_bars_x11};
use crate::client::focus::FocusState;
use crate::globals::Globals;
use crate::types::{Client, Rect, WindowId};

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
    pub x11: X11BackendRef<'a>,
}

impl<'a> WmCtxX11<'a> {
    pub fn reborrow(&mut self) -> WmCtxX11<'_> {
        WmCtxX11 {
            core: self.core.reborrow(),
            backend: self.backend.reborrow(),
            x11: X11BackendRef::new(self.x11.conn, self.x11.screen_num),
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

impl<'a> WmCtxWayland<'a> {
    pub fn reborrow(&mut self) -> WmCtxWayland<'_> {
        WmCtxWayland {
            core: self.core.reborrow(),
            backend: self.backend.reborrow(),
            wayland: WaylandCtx {
                backend: self.wayland.backend,
            },
            xwayland: self.xwayland.as_ref().map(|xw| XwaylandCtx {
                xdisplay: xw.xdisplay,
                xwm: xw.xwm,
            }),
        }
    }
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

    pub fn set_border(&mut self, win: WindowId, width: i32) {
        if let Some(client) = self.g_mut().clients.get_mut(&win) {
            client.border_width = width.max(0);
        }
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

    /// Warp cursor to client (X11 only, no-op on Wayland).
    pub fn warp_cursor_to_client(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(x11) => {
                crate::focus::warp_cursor_to_client_x11(&x11.core, &x11.x11, win);
            }
            WmCtx::Wayland(_) => {
                // Wayland doesn't allow compositor cursor warping - no-op
            }
        }
    }

    pub fn backend_kind_REMOVED(&self) -> BackendKind {
        self.backend().kind()
    }

    /// Backend-agnostic bar refresh request.
    ///
    /// - X11: performs an immediate draw.
    /// - Wayland: marks bar cache as dirty; next frame re-renders.
    pub fn request_bar_update(&mut self, monitor_id: Option<usize>) {
        match self {
            WmCtx::X11(ctx_x11) => {
                ctx_x11.core.bar.mark_dirty();
                if let Some(id) = monitor_id {
                    draw_bar(&mut ctx_x11.core, &ctx_x11.x11, id);
                } else {
                    draw_bars_x11(&mut ctx_x11.core, &ctx_x11.x11);
                }
            }
            WmCtx::Wayland(ctx_wayland) => {
                let _ = monitor_id;
                ctx_wayland.core.bar.mark_dirty();
            }
        }
    }

    // For backend-specific operations, use match on the enum directly
    // instead of accessor methods that return Option.
}

impl<'a> Deref for WmCtx<'a> {
    type Target = CoreCtx<'a>;

    fn deref(&self) -> &Self::Target {
        match self {
            WmCtx::X11(ctx) => &ctx.core,
            WmCtx::Wayland(ctx) => &ctx.core,
        }
    }
}

impl<'a> DerefMut for WmCtx<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            WmCtx::X11(ctx) => &mut ctx.core,
            WmCtx::Wayland(ctx) => &mut ctx.core,
        }
    }
}
