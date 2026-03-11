//! Context split for backend-specific operations.
//!
//! Core state (`Globals`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit `X11BackendRef` / `WaylandCtx` instead of runtime checks.

use std::ops::{Deref, DerefMut};

use crate::backend::x11::X11BackendRef;
use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::bar::BarState;
use crate::bar::{draw_bar, draw_bars_x11};
use crate::client::focus::FocusState;
use crate::globals::Globals;
use crate::globals::X11RuntimeConfig;
use crate::types::{Client, Rect, Systray, WaylandSystray, WaylandSystrayMenu, WindowId};

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
    pub x11_runtime: &'a mut X11RuntimeConfig,
    pub systray: Option<&'a mut Systray>,
}

impl<'a> WmCtxX11<'a> {
    pub fn reborrow(&mut self) -> WmCtxX11<'_> {
        WmCtxX11 {
            core: self.core.reborrow(),
            backend: self.backend.reborrow(),
            x11: X11BackendRef::new(self.x11.conn, self.x11.screen_num),
            x11_runtime: self.x11_runtime,
            systray: self.systray.as_deref_mut(),
        }
    }

    pub fn selected_client(&self) -> Option<WindowId> {
        self.core.selected_client()
    }

    pub fn x11_runtime(&self) -> &X11RuntimeConfig {
        &self.x11_runtime
    }

    pub fn x11_runtime_mut(&mut self) -> &mut X11RuntimeConfig {
        &mut self.x11_runtime
    }

    pub fn systray(&self) -> Option<&Systray> {
        self.systray.as_deref()
    }

    pub fn systray_mut(&mut self) -> Option<&mut Systray> {
        self.systray.as_deref_mut()
    }
}

pub struct WmCtxWayland<'a> {
    pub core: CoreCtx<'a>,
    pub backend: BackendRef<'a>,
    pub wayland: WaylandCtx<'a>,
    pub xwayland: Option<XwaylandCtx<'a>>,
    pub wayland_systray: &'a mut WaylandSystray,
    pub wayland_systray_menu: Option<&'a mut WaylandSystrayMenu>,
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
            wayland_systray: self.wayland_systray,
            wayland_systray_menu: self.wayland_systray_menu.as_deref_mut(),
        }
    }

    pub fn wayland_systray(&self) -> &WaylandSystray {
        &self.wayland_systray
    }

    pub fn wayland_systray_mut(&mut self) -> &mut WaylandSystray {
        &mut self.wayland_systray
    }

    pub fn wayland_systray_menu(&self) -> Option<&WaylandSystrayMenu> {
        self.wayland_systray_menu.as_deref()
    }

    pub fn wayland_systray_menu_mut(&mut self) -> Option<&mut WaylandSystrayMenu> {
        self.wayland_systray_menu.as_deref_mut()
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

    pub fn client_mut(&mut self, win: WindowId) -> Option<&mut Client> {
        self.core_mut().client_mut(win)
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

    pub fn numlock_mask(&self) -> u32 {
        match self {
            WmCtx::X11(ctx) => ctx.x11_runtime().numlockmask,
            WmCtx::Wayland(_) => 0, // Wayland handles modifiers internally
        }
    }

    pub fn flush(&self) {
        self.backend().flush();
    }

    pub fn raise(&self, win: WindowId) {
        self.backend().raise_window(win);
    }

    /// Raise a window and persist that z-order in monitor stack state.
    ///
    /// Use this for interactive operations (move/resize drags) so later
    /// restacks do not drop the dragged floating window behind others.
    pub fn raise_interactive(&mut self, win: WindowId) {
        if let Some(mid) = self.g().clients.get(&win).map(|c| c.monitor_id) {
            if let Some(mon) = self.g_mut().monitor_mut(mid) {
                mon.stack.retain(|&w| w != win);
                mon.stack.push(win);
            }
        }
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

    /// Warp cursor to client.
    ///
    /// On X11 this uses `XWarpPointer`.  On Wayland the warp is deferred to
    /// the next event-loop tick via `WaylandState::pending_warp` so that
    /// the pointer handle and the external `pointer_location` variable are
    /// both updated atomically.
    pub fn warp_cursor_to_client(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(x11) => {
                crate::mouse::warp::warp_to_client_win(&x11.core, &x11.x11, x11.x11_runtime, win);
            }
            WmCtx::Wayland(wl) => {
                // Skip the warp if the pointer is already inside the window,
                // mirroring the X11 behaviour in warp_to_client_win.
                let Some(c) = wl.core.g.clients.get(&win) else {
                    return;
                };
                let target_x = (c.geo.x + c.geo.w / 2) as f64;
                let target_y = (c.geo.y + c.geo.h / 2) as f64;

                // Check current pointer position to avoid jarring jumps when
                // the cursor is already over the window.
                if let Some((ptr_x, ptr_y)) = wl.wayland.backend.pointer_location() {
                    let in_window = ptr_x >= c.geo.x
                        && ptr_x <= c.geo.x + c.geo.w
                        && ptr_y >= c.geo.y
                        && ptr_y <= c.geo.y + c.geo.h;
                    if in_window {
                        return;
                    }
                }

                wl.wayland.backend.warp_pointer(target_x, target_y);
            }
        }
    }

    /// Returns true when running under Wayland.
    pub fn is_wayland(&self) -> bool {
        matches!(self, WmCtx::Wayland(_))
    }

    /// Returns true when running under X11.
    pub fn is_x11(&self) -> bool {
        matches!(self, WmCtx::X11(_))
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
                    draw_bar(
                        &mut ctx_x11.core,
                        ctx_x11.x11_runtime,
                        ctx_x11.systray.as_deref(),
                        id,
                    );
                } else {
                    draw_bars_x11(
                        &mut ctx_x11.core,
                        ctx_x11.x11_runtime,
                        ctx_x11.systray.as_deref(),
                    );
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
