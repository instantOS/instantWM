//! Context split for backend-specific operations.
//!
//! Core state (`Globals`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit `X11BackendRef` / `WaylandCtx` instead of runtime checks.

use crate::backend::BackendOps;
use crate::backend::BackendRef;
use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::geometry::{GeometryApplyMode, MoveResizeOptions};
use crate::globals::Globals;
use crate::types::{Client, Rect, Systray, WaylandSystray, WaylandSystrayMenu, WindowId};

pub struct CoreCtx<'a> {
    g: &'a mut Globals,
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

    pub fn globals(&self) -> &Globals {
        self.g
    }

    pub fn globals_mut(&mut self) -> &mut Globals {
        self.g
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

impl<'a> WaylandCtx<'a> {
    pub fn reborrow(&self) -> WaylandCtx<'_> {
        WaylandCtx {
            backend: self.backend,
        }
    }
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
        self.x11_runtime
    }

    pub fn x11_runtime_mut(&mut self) -> &mut X11RuntimeConfig {
        self.x11_runtime
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
            wayland: self.wayland.reborrow(),
            xwayland: self.xwayland.as_ref().map(|xw| XwaylandCtx {
                xdisplay: xw.xdisplay,
                xwm: xw.xwm,
            }),
            wayland_systray: self.wayland_systray,
            wayland_systray_menu: self.wayland_systray_menu.as_deref_mut(),
        }
    }

    pub fn wayland_systray(&self) -> &WaylandSystray {
        self.wayland_systray
    }

    pub fn wayland_systray_mut(&mut self) -> &mut WaylandSystray {
        self.wayland_systray
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

    /// Request backend-specific space/compositor sync after authoritative WM
    /// geometry changes.
    pub fn request_space_sync(&self) {
        if let WmCtx::Wayland(ctx) = self {
            ctx.wayland.backend.request_space_sync();
        }
    }

    pub fn pointer_location(&self) -> Option<(i32, i32)> {
        self.backend().pointer_location()
    }

    pub fn warp_pointer(&self, x: f64, y: f64) {
        self.backend().warp_pointer(x, y);
    }

    pub fn raise_window_visual_only(&self, win: WindowId) {
        self.backend().raise_window_visual_only(win);
    }

    /// Raise a client and persist that z-order in monitor state.
    ///
    /// Use this for interactive operations (move/resize drags) so later
    /// z-order syncs do not drop the dragged floating window behind others.
    pub fn raise_client(&mut self, win: WindowId) {
        if let Some(mid) = self.core().globals().clients.monitor_id(win)
            && let Some(mon) = self.core_mut().globals_mut().monitor_mut(mid)
        {
            mon.z_order.raise(win);
        }
        self.backend().raise_window_visual_only(win);
    }

    pub fn apply_window_order_bottom_to_top(&self, wins: &[WindowId]) {
        self.backend().apply_window_order_bottom_to_top(wins);
    }

    pub(crate) fn set_geometry_impl(
        &mut self,
        win: WindowId,
        rect: Rect,
        apply_mode: GeometryApplyMode,
    ) {
        match self {
            WmCtx::X11(x11) => {
                if apply_mode == GeometryApplyMode::VisualOnly {
                    x11.backend.resize_window(win, rect);
                    x11.backend.flush();
                    return;
                }

                // X11 clients may ignore or adjust resize requests (size hints).
                // Query the actual geometry back and sync that into WM state.
                x11.backend.resize_window(win, rect);
                x11.backend.flush();
                let actual = crate::backend::x11::query_window_rect(&x11.x11, win).unwrap_or(rect);
                crate::client::sync_client_geometry(x11.core.globals_mut(), win, actual);

                crate::client::focus::configure_x11(&mut x11.core, &x11.x11, win);
            }
            WmCtx::Wayland(_) => {
                if apply_mode == GeometryApplyMode::Logical {
                    crate::client::sync_client_geometry(self.core_mut().globals_mut(), win, rect);
                }
                self.backend().resize_window(win, rect);
                if apply_mode == GeometryApplyMode::VisualOnly {
                    self.backend().flush();
                }
            }
        }
    }

    pub fn move_resize(&mut self, win: WindowId, rect: Rect, options: MoveResizeOptions) {
        crate::geometry::move_resize(self, win, rect, options);
    }

    pub fn set_border(&mut self, win: WindowId, width: i32) {
        if let Some(client) = self.core_mut().globals_mut().clients.get_mut(&win) {
            client.border_width = width.max(0);
        }
        // Border width is X11-specific; Wayland doesn't support border width
        if let WmCtx::X11(x11) = self {
            x11.x11.set_border_width(win, width);
        }
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
        let bar_height = self.core().globals().cfg.bar_height;

        // No target window – centre on the selected monitor's work area.
        if win == WindowId::default() {
            let mon = self.core().globals().selected_monitor();
            let target_x = (mon.work_rect.x + mon.work_rect.w / 2) as f64;
            let target_y = (mon.work_rect.y + mon.work_rect.h / 2) as f64;
            self.warp_pointer(target_x, target_y);
            return;
        }

        let Some(c) = self.client(win).cloned() else {
            return;
        };

        let Some((ptr_x, ptr_y)) = self.pointer_location() else {
            return;
        };

        // Skip if already inside the window (including border).
        let in_window = c.geo.contains_point(ptr_x, ptr_y)
            || (ptr_x > c.geo.x - c.border_width
                && ptr_y > c.geo.y - c.border_width
                && ptr_x < c.geo.x + c.geo.w + c.border_width * 2
                && ptr_y < c.geo.y + c.geo.h + c.border_width * 2);

        let on_bar = c.monitor(self.core().globals()).is_some_and(|mon| {
            (ptr_y > mon.bar_y && ptr_y < mon.bar_y + bar_height) || (mon.topbar && ptr_y == 0)
        });

        if in_window || on_bar {
            return;
        }

        let target_x = (c.geo.x + c.geo.w / 2) as f64;
        let target_y = (c.geo.y + c.geo.h / 2) as f64;
        self.warp_pointer(target_x, target_y);
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
    /// - X11: marks the bar dirty; the normal calloop tick redraws it.
    /// - Wayland: marks bar cache as dirty; next frame re-renders.
    pub fn request_bar_update(&mut self, monitor_id: Option<crate::types::MonitorId>) {
        match self {
            WmCtx::X11(ctx_x11) => {
                let _ = monitor_id;
                ctx_x11.core.bar.mark_dirty();
            }
            WmCtx::Wayland(ctx_wayland) => {
                let _ = monitor_id;
                if !ctx_wayland.wayland.backend.request_bar_redraw() {
                    ctx_wayland.core.bar.mark_dirty();
                }
            }
        }
    }

    pub fn current_mode(&self) -> &str {
        &self.core().globals().behavior.current_mode
    }

    pub fn set_current_mode(&mut self, mode: impl Into<String>) {
        self.core_mut().globals_mut().behavior.current_mode = mode.into();
    }

    pub fn reset_mode(&mut self) {
        self.set_current_mode("default");
    }

    pub fn with_behavior_mut<R>(
        &mut self,
        f: impl FnOnce(&mut crate::globals::WmBehavior) -> R,
    ) -> R {
        f(&mut self.core_mut().globals_mut().behavior)
    }

    // For backend-specific operations, use match on the enum directly
    // instead of accessor methods that return Option.
}
