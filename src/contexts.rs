//! Context split for backend-specific operations.
//!
//! Core state (`CoreState`, tags/layouts/monitors/clients/config) remains
//! backend-agnostic and is accessed via `CoreCtx`. Backend-specific code
//! receives explicit backend references instead of runtime checks.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::config::{SchemeHover, SchemeTag, SchemeWin};
use crate::core_state::{
    CoreState, DragState, KeyboardLayoutState, PendingWork, RuntimeConfig, WmBehavior,
};
use crate::geometry::{GeometryApplyMode, MoveResizeOptions};
use crate::model::WmModel;
use crate::types::{MonitorId, Rect, Systray, WaylandSystray, WaylandSystrayMenu, WindowId};

pub struct CoreCtx<'a> {
    pub(crate) g: &'a mut CoreState,
    work: &'a mut PendingWork,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub focus: &'a mut FocusState,
}

impl<'a> CoreCtx<'a> {
    pub fn new(
        g: &'a mut CoreState,
        work: &'a mut PendingWork,
        running: &'a mut bool,
        bar: &'a mut BarState,
        focus: &'a mut FocusState,
    ) -> Self {
        Self {
            g,
            work,
            running,
            bar,
            focus,
        }
    }

    pub fn model(&self) -> &WmModel {
        &self.g.model
    }

    pub fn model_mut(&mut self) -> &mut WmModel {
        &mut self.g.model
    }

    /// Access all backend-neutral state. Prefer the category-specific
    /// accessors when an operation only needs one part of the state.
    pub fn state(&self) -> &CoreState {
        self.g
    }

    pub fn state_mut(&mut self) -> &mut CoreState {
        self.g
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.g.config
    }

    pub fn config_mut(&mut self) -> &mut RuntimeConfig {
        &mut self.g.config
    }

    pub fn behavior(&self) -> &WmBehavior {
        &self.g.behavior
    }

    pub fn behavior_mut(&mut self) -> &mut WmBehavior {
        &mut self.g.behavior
    }

    pub fn drag_state(&self) -> &DragState {
        &self.g.drag
    }

    pub fn drag_state_mut(&mut self) -> &mut DragState {
        &mut self.g.drag
    }

    pub fn keyboard_layout(&self) -> &KeyboardLayoutState {
        &self.g.keyboard_layout
    }

    pub fn keyboard_layout_mut(&mut self) -> &mut KeyboardLayoutState {
        &mut self.g.keyboard_layout
    }

    pub fn pending_launches_mut(
        &mut self,
    ) -> &mut std::collections::VecDeque<crate::client::PendingLaunch> {
        &mut self.g.pending_launches
    }

    pub fn quit(&mut self) {
        *self.running = false;
    }

    pub fn queue_layout_for_all_monitors(&mut self) {
        self.work.layout.mark_all();
    }

    pub fn queue_layout_for_all_monitors_urgent(&mut self) {
        self.work.layout.mark_all_urgent();
    }

    pub fn queue_layout_for_monitor_urgent(&mut self, monitor_id: MonitorId) {
        self.work.layout.mark_monitor_urgent(monitor_id);
    }

    pub fn queue_layout_for_client(&mut self, win: WindowId) {
        self.work
            .layout
            .mark_monitor_opt(self.g.model.clients.monitor_id(win));
    }

    pub fn queue_monitor_config_apply(&mut self) {
        self.work.queue_monitor_config_apply();
    }

    pub fn queue_input_config_apply(&mut self) {
        self.work.queue_input_config_apply();
    }

    pub fn pending_work(&self) -> &PendingWork {
        self.work
    }

    pub fn pending_work_mut(&mut self) -> &mut PendingWork {
        self.work
    }

    // -------------------------------------------------------------------------
    // Color scheme helpers
    // -------------------------------------------------------------------------

    pub fn status_scheme(&self) -> crate::bar::paint::BarScheme {
        let c = &self.g.config.colors.status_bar;
        crate::bar::paint::BarScheme {
            fg: c.fg,
            bg: c.bg,
            detail: c.detail,
        }
    }

    pub fn tag_hover_fill_scheme(&self) -> crate::bar::paint::BarScheme {
        let colors = self
            .model()
            .tags
            .colors
            .colors_for(SchemeHover::Hover, SchemeTag::Filled);
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    pub fn tag_scheme(
        &self,
        m: &crate::types::Monitor,
        tag_index: u32,
        occupied_tags: crate::types::TagMask,
        urgent_tags: crate::types::TagMask,
        is_hover: bool,
    ) -> crate::bar::paint::BarScheme {
        let tag_num = tag_index as usize + 1;
        let tag_role = if urgent_tags.contains(tag_num) {
            SchemeTag::Urgent
        } else if occupied_tags.contains(tag_num) {
            let selmon = self.g.model.monitors.sel();
            let sel_has_tag = selmon
                .and_then(|selmon| {
                    selmon.sel.and_then(|selected_window| {
                        self.g
                            .model
                            .clients
                            .get(&selected_window)
                            .map(|c| c.tags.contains(tag_num))
                    })
                })
                .unwrap_or(false);

            let is_selected = selmon.is_some_and(|selmon| selmon.num == m.num);

            if is_selected && sel_has_tag {
                SchemeTag::Focus
            } else if m.selected_tags().contains(tag_num) {
                SchemeTag::NoFocus
            } else if !m.showtags {
                SchemeTag::Filled
            } else {
                SchemeTag::Inactive
            }
        } else if m.selected_tags().contains(tag_num) {
            SchemeTag::Empty
        } else {
            SchemeTag::Inactive
        };

        let colors = self.g.model.tags.colors.colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            tag_role,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    pub fn window_scheme(
        &self,
        c: &crate::types::Client,
        is_hover: bool,
    ) -> crate::bar::paint::BarScheme {
        let selmon = self.g.model.monitors.sel();
        let is_selected = selmon.and_then(|s| s.sel) == Some(c.win);
        let is_edge_scratchpad = c.is_edge_scratchpad();

        let window_role = if is_selected {
            if is_edge_scratchpad {
                SchemeWin::EdgeScratchpadFocus
            } else if c.is_sticky {
                SchemeWin::StickyFocus
            } else {
                SchemeWin::Focus
            }
        } else if is_edge_scratchpad {
            SchemeWin::EdgeScratchpad
        } else if c.is_sticky {
            SchemeWin::Sticky
        } else if c.is_minimized() {
            SchemeWin::Minimized
        } else if c.is_urgent {
            SchemeWin::Urgent
        } else {
            SchemeWin::Normal
        };

        let colors = self.g.config.colors.window.colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            window_role,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    pub fn close_button_scheme(
        &self,
        is_hover: bool,
        is_locked: bool,
        is_fullscreen: bool,
    ) -> crate::bar::paint::BarScheme {
        use crate::config::{SchemeClose, SchemeHover};

        let close_role = if is_locked {
            SchemeClose::Locked
        } else if is_fullscreen {
            SchemeClose::Fullscreen
        } else {
            SchemeClose::Normal
        };

        let colors = self.g.config.colors.close_button.colors_for(
            if is_hover {
                SchemeHover::Hover
            } else {
                SchemeHover::NoHover
            },
            close_role,
        );
        crate::bar::paint::BarScheme {
            fg: colors.fg,
            bg: colors.bg,
            detail: colors.detail,
        }
    }

    pub fn normalize_current_mode(&mut self) {
        if self.g.behavior.current_mode == "default"
            || self.g.behavior.current_mode == crate::overview::OVERVIEW_MODE_NAME
        {
            return;
        }

        if !self
            .config()
            .bindings
            .modes
            .contains_key(&self.g.behavior.current_mode)
        {
            self.g.behavior.current_mode = "default".to_string();
        }
    }

    pub fn reborrow(&mut self) -> CoreCtx<'_> {
        CoreCtx {
            g: self.g,
            work: self.work,
            running: self.running,
            bar: self.bar,
            focus: self.focus,
        }
    }
}

pub struct WmCtxX11<'a> {
    pub core: CoreCtx<'a>,
    pub x11: X11BackendRef<'a>,
    pub x11_runtime: &'a mut X11RuntimeConfig,
    pub systray: Option<&'a mut Systray>,
}

impl<'a> WmCtxX11<'a> {
    pub fn reborrow(&mut self) -> WmCtxX11<'_> {
        WmCtxX11 {
            core: self.core.reborrow(),
            x11: X11BackendRef::new(self.x11.conn, self.x11.screen_num),
            x11_runtime: self.x11_runtime,
            systray: self.systray.as_deref_mut(),
        }
    }

    pub fn x11_runtime(&self) -> &X11RuntimeConfig {
        self.x11_runtime
    }
}

pub struct WmCtxWayland<'a> {
    pub core: CoreCtx<'a>,
    pub wayland: &'a crate::backend::wayland::WaylandBackend,
    pub wayland_systray: &'a mut WaylandSystray,
    pub wayland_systray_menu: Option<&'a mut WaylandSystrayMenu>,
}

impl<'a> WmCtxWayland<'a> {
    pub fn reborrow(&mut self) -> WmCtxWayland<'_> {
        WmCtxWayland {
            core: self.core.reborrow(),
            wayland: self.wayland,
            wayland_systray: self.wayland_systray,
            wayland_systray_menu: self.wayland_systray_menu.as_deref_mut(),
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

    pub fn quit(&mut self) {
        self.core_mut().quit();
    }

    pub fn window_backend(&self) -> &dyn crate::backend::WindowOps {
        match self {
            WmCtx::X11(ctx) => &ctx.x11,
            WmCtx::Wayland(ctx) => ctx.wayland,
        }
    }

    pub fn pointer_backend(&self) -> &dyn crate::backend::PointerOps {
        match self {
            WmCtx::X11(ctx) => &ctx.x11,
            WmCtx::Wayland(ctx) => ctx.wayland,
        }
    }

    pub fn output_backend(&self) -> &dyn crate::backend::OutputOps {
        match self {
            WmCtx::X11(ctx) => &ctx.x11,
            WmCtx::Wayland(ctx) => ctx.wayland,
        }
    }

    pub fn numlock_mask(&self) -> u32 {
        match self {
            WmCtx::X11(ctx) => ctx.x11_runtime().numlockmask,
            WmCtx::Wayland(_) => 0, // Wayland handles modifiers internally
        }
    }

    /// Request backend-specific space/compositor sync after authoritative WM
    /// geometry changes.
    pub fn request_space_sync(&self) {
        if let WmCtx::Wayland(ctx) = self {
            ctx.wayland.request_space_sync();
        }
    }

    /// Raise a client and persist that z-order in monitor state.
    ///
    /// Use this for interactive operations (move/resize drags) so later
    /// z-order syncs do not drop the dragged floating window behind others.
    pub fn raise_client(&mut self, win: WindowId) {
        if let Some(mid) = self.core().model().clients.monitor_id(win)
            && let Some(mon) = self.core_mut().model_mut().monitor_mut(mid)
        {
            mon.z_order.raise(win);
        }
        self.window_backend().raise_window_visual_only(win);
    }

    pub(crate) fn set_geometry_impl(
        &mut self,
        win: WindowId,
        rect: Rect,
        apply_mode: GeometryApplyMode,
    ) {
        match self {
            WmCtx::X11(_) => {
                if apply_mode == GeometryApplyMode::VisualOnly {
                    self.window_backend().resize_window(win, rect);
                    self.window_backend().flush();
                    return;
                }

                // X11 clients may ignore or adjust resize requests (size hints).
                // Query the actual geometry back and sync that into WM state.
                self.window_backend().resize_window(win, rect);
                self.window_backend().flush();
                let WmCtx::X11(x11) = self else {
                    unreachable!()
                };
                let actual = crate::backend::x11::query_window_rect(&x11.x11, win).unwrap_or(rect);
                crate::client::sync_client_geometry(x11.core.model_mut(), win, actual);

                crate::backend::x11::focus::configure_x11(x11.core.g, &x11.x11, win);
            }
            WmCtx::Wayland(_) => {
                if apply_mode == GeometryApplyMode::Logical {
                    crate::client::sync_client_geometry(self.core_mut().model_mut(), win, rect);
                }
                self.window_backend().resize_window(win, rect);
                if apply_mode == GeometryApplyMode::VisualOnly {
                    self.window_backend().flush();
                }
            }
        }
    }

    pub fn move_resize(&mut self, win: WindowId, rect: Rect, options: MoveResizeOptions) {
        crate::geometry::move_resize(self, win, rect, options);
    }

    pub fn set_border(&mut self, win: WindowId, width: i32) {
        if let Some(client) = self.core_mut().model_mut().clients.get_mut(&win) {
            client.border_width = width.max(0);
        }
    }

    /// Update root EWMH workspace/tag properties. X11 only; no-op on Wayland.
    pub fn update_ewmh_desktop_props(&mut self) {
        if let WmCtx::X11(ctx) = self {
            crate::backend::x11::update_ewmh_desktop_props(ctx.core.g, &ctx.x11, ctx.x11_runtime);
        }
    }

    /// Warp cursor to client.
    ///
    /// On X11 this uses `XWarpPointer`.  On Wayland the warp is deferred to
    /// the next event-loop tick via `WaylandState::pending_warp` so that
    /// the pointer handle and the external `pointer_location` variable are
    /// both updated atomically.
    pub fn warp_cursor_to_client(&mut self, win: WindowId) {
        // No target window – centre on the selected monitor's work area.
        if win == WindowId::default() {
            let mon = self.core().model().selected_monitor();
            let target_x = (mon.work_rect.x + mon.work_rect.w / 2) as f64;
            let target_y = (mon.work_rect.y + mon.work_rect.h / 2) as f64;
            self.pointer_backend().warp_pointer(target_x, target_y);
            return;
        }

        let Some(c) = self.core().model().clients.get(&win).cloned() else {
            return;
        };

        let Some(ptr) = self.pointer_backend().pointer_location() else {
            return;
        };

        // Skip if already inside the window (including border).
        let in_window = c.geo.contains_point(ptr)
            || (ptr.x > c.geo.x - c.border_width
                && ptr.y > c.geo.y - c.border_width
                && ptr.x < c.geo.x + c.geo.w + c.border_width * 2
                && ptr.y < c.geo.y + c.geo.h + c.border_width * 2);

        let on_bar = c
            .monitor(self.core().model())
            .is_some_and(|mon| mon.bar_contains_y(self.core().model().clients.map(), ptr.y));

        if in_window || on_bar {
            return;
        }

        let target_x = (c.geo.x + c.geo.w / 2) as f64;
        let target_y = (c.geo.y + c.geo.h / 2) as f64;
        self.pointer_backend().warp_pointer(target_x, target_y);
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
    pub fn request_bar_update(&mut self) {
        match self {
            WmCtx::X11(ctx_x11) => {
                ctx_x11.core.bar.mark_dirty();
            }
            WmCtx::Wayland(ctx_wayland) => {
                if !ctx_wayland.wayland.request_bar_redraw() {
                    ctx_wayland.core.bar.mark_dirty();
                }
            }
        }
    }

    pub fn current_mode(&self) -> &str {
        &self.core().behavior().current_mode
    }

    pub fn set_current_mode(&mut self, mode: impl Into<String>) {
        let next_mode = mode.into();
        let previous_mode = self.core().behavior().current_mode.clone();
        if previous_mode == next_mode {
            return;
        }

        self.core_mut().behavior_mut().current_mode = next_mode.clone();
        crate::overview::handle_mode_transition(self, &previous_mode, &next_mode);
    }

    pub fn reset_mode(&mut self) {
        self.set_current_mode("default");
    }

    pub fn with_behavior_mut<R>(
        &mut self,
        f: impl FnOnce(&mut crate::core_state::WmBehavior) -> R,
    ) -> R {
        f(self.core_mut().behavior_mut())
    }
}
