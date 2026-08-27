//! Backend dispatch seam for the window manager core.
//!
//! Shared domain modules (layouts, clients, tags, mouse, ipc) never import
//! backend namespaces. Backend differences are expressed here as
//! delegation-only methods on `WmCtx`: each method matches once over the
//! two context variants and forwards to per-backend implementations living
//! under `crate::backend`. Methods must not make policy decisions.
//!
//! Rules of thumb when adding behavior:
//! * Pure decision logic belongs in the owning domain module (ideally a
//!   model transition that returns data).
//! * Protocol effects belong behind a `WmCtx` method or an ops trait;
//!   never branch on backend type outside this module, `wm.rs`, and
//!   `backend::startup` wiring.
//! * A missing Wayland implementation is stated as a documented no-op arm,
//!   never as a silent early return in shared code.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::bar::BarState;
use crate::client::focus::FocusState;
use crate::core_state::{CoreState, DerivedState, EffectiveConfig, PendingWork, WmBehavior};
use crate::geometry::{GeometryApplyMode, MoveResizeOptions};
use crate::model::WmModel;
use crate::types::{MonitorId, Rect, WindowId, XEmbedTray};
use std::time::{Duration, Instant};

pub struct CoreCtx<'a> {
    pub(crate) state: &'a mut CoreState,
    work: &'a mut PendingWork,
    running: &'a mut bool,
    pub bar: &'a mut BarState,
    pub focus: &'a mut FocusState,
}

impl<'a> CoreCtx<'a> {
    pub fn new(
        state: &'a mut CoreState,
        work: &'a mut PendingWork,
        running: &'a mut bool,
        bar: &'a mut BarState,
        focus: &'a mut FocusState,
    ) -> Self {
        Self {
            state,
            work,
            running,
            bar,
            focus,
        }
    }

    pub fn model(&self) -> &WmModel {
        &self.state.model
    }

    pub fn model_mut(&mut self) -> &mut WmModel {
        &mut self.state.model
    }

    /// Return a managed client's current logical geometry.
    #[inline]
    pub fn client_geo(&self, win: WindowId) -> Option<Rect> {
        self.model().client(win).map(|client| client.geo)
    }

    /// Access all backend-neutral state. Prefer the category-specific
    /// accessors when an operation only needs one part of the state.
    pub fn state(&self) -> &CoreState {
        self.state
    }

    pub fn state_mut(&mut self) -> &mut CoreState {
        self.state
    }

    pub fn config(&self) -> &EffectiveConfig {
        &self.state.config
    }

    pub fn config_mut(&mut self) -> &mut EffectiveConfig {
        &mut self.state.config
    }

    pub fn derived(&self) -> &DerivedState {
        &self.state.derived
    }

    pub fn derived_mut(&mut self) -> &mut DerivedState {
        &mut self.state.derived
    }

    pub fn behavior(&self) -> &WmBehavior {
        &self.state.behavior
    }

    pub fn behavior_mut(&mut self) -> &mut WmBehavior {
        &mut self.state.behavior
    }

    pub fn interaction(&self) -> &crate::core_state::InteractionState {
        &self.state.interaction
    }

    pub fn interaction_mut(&mut self) -> &mut crate::core_state::InteractionState {
        &mut self.state.interaction
    }

    pub fn pending_launches_mut(
        &mut self,
    ) -> &mut std::collections::VecDeque<crate::client::PendingLaunch> {
        &mut self.state.pending_launches
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

    pub fn queue_layout_for_monitor(&mut self, monitor_id: MonitorId) {
        self.work.layout.mark_monitor(monitor_id);
    }

    pub fn queue_layout_for_monitor_urgent(&mut self, monitor_id: MonitorId) {
        self.work.layout.mark_monitor_urgent(monitor_id);
    }

    pub fn queue_layout_for_client(&mut self, win: WindowId) {
        if let Some(monitor_id) = self.state.model.client(win).map(|client| client.monitor_id) {
            self.work.layout.mark_monitor(monitor_id);
        }
    }

    /// Queue the first authoritative layout for a newly managed window and
    /// its post-layout entrance transition as one lifecycle operation.
    ///
    /// First layout is urgent because the Wayland surface remains intentionally
    /// unmapped until this work has assigned usable geometry.
    pub fn queue_initial_window_layout(&mut self, win: WindowId, monitor_id: MonitorId) {
        self.work.layout.mark_monitor_urgent(monitor_id);
        self.work.spawn_animations.insert(win);
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

    /// Run a model transaction and record any resulting global-selection
    /// transition. All production mutations that can affect selection cross
    /// this boundary, including indirect removal/reassignment effects.
    pub fn mutate_selection<R>(&mut self, mutation: impl FnOnce(&mut WmModel) -> R) -> R {
        self.mutate_state_selection(|state| mutation(&mut state.model))
    }

    pub fn mutate_state_selection<R>(&mut self, mutation: impl FnOnce(&mut CoreState) -> R) -> R {
        let previous = self.state.model.selected_win();
        let result = mutation(self.state);
        let current = self.state.model.selected_win();
        self.focus.record_selection(previous, current);
        result
    }

    pub fn select_monitor(&mut self, monitor_id: MonitorId) -> bool {
        self.mutate_selection(|model| {
            if model.monitor(monitor_id).is_none() || model.selected_monitor_id() == monitor_id {
                return false;
            }
            model.set_selected_monitor(monitor_id);
            true
        })
    }

    pub fn select_on_monitor(&mut self, monitor_id: MonitorId, selected: Option<WindowId>) -> bool {
        self.mutate_selection(|model| {
            let Some(monitor) = model.monitor_mut(monitor_id) else {
                return false;
            };
            if monitor.selected == selected {
                return false;
            }
            monitor.set_selected(selected);
            true
        })
    }

    pub fn reborrow(&mut self) -> CoreCtx<'_> {
        CoreCtx {
            state: self.state,
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
    /// Owned optional tray state. Keep the outer `Option` available so the X11
    /// backend can create the manager window during initialization.
    pub xembed_tray: &'a mut Option<XEmbedTray>,
}

impl<'a> WmCtxX11<'a> {
    pub fn reborrow(&mut self) -> WmCtxX11<'_> {
        WmCtxX11 {
            core: self.core.reborrow(),
            x11: X11BackendRef::new(self.x11.conn, self.x11.screen_num),
            x11_runtime: self.x11_runtime,
            xembed_tray: &mut *self.xembed_tray,
        }
    }

    pub fn x11_runtime(&self) -> &X11RuntimeConfig {
        self.x11_runtime
    }
}

pub struct WmCtxWayland<'a> {
    pub core: CoreCtx<'a>,
    pub wayland: &'a crate::backend::wayland::WaylandBackend,
}

impl<'a> WmCtxWayland<'a> {
    pub fn reborrow(&mut self) -> WmCtxWayland<'_> {
        WmCtxWayland {
            core: self.core.reborrow(),
            wayland: self.wayland,
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

    /// Reconcile the backend with the presentation derived from authoritative
    /// interaction state. This is intentionally level-triggered: callers may
    /// invoke it after any possibly relevant transition without tracking the
    /// previous native state.
    pub fn sync_interaction_projection(&mut self) {
        use crate::backend::InteractionProjectionOps;
        let desired = self.core().interaction().drag.presentation();
        match self {
            WmCtx::X11(ctx) => ctx.reconcile_interaction_projection(desired),
            WmCtx::Wayland(ctx) => ctx.reconcile_interaction_projection(desired),
        }
    }

    /// Commit one pointer-interaction model transition and reconcile whenever
    /// its derived native presentation changes.
    ///
    /// Production interaction mutations should cross this boundary instead of
    /// mutating `DragState` through `CoreCtx`; the closure form makes it
    /// impossible to return successfully with an unprojected presentation
    /// change. State-only motion updates avoid redundant backend work.
    pub fn transition_pointer_interaction<R>(
        &mut self,
        transition: impl FnOnce(&mut crate::core_state::DragState) -> R,
    ) -> R {
        let previous = self.core().interaction().drag.presentation();
        let result = transition(&mut self.core_mut().state_mut().interaction.drag);
        if self.core().interaction().drag.presentation() != previous {
            self.sync_interaction_projection();
        }
        result
    }

    /// Ask the active backend to close a managed window gracefully, falling
    /// back to its forceful mechanism when the protocol requires it.
    pub fn close_window(&mut self, win: WindowId) {
        use crate::backend::WindowCloseOps;
        match self {
            WmCtx::X11(ctx) => ctx.close_window(win),
            WmCtx::Wayland(ctx) => ctx.close_window(win),
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

    pub fn backend_kind(&self) -> crate::backend::BackendKind {
        match self {
            WmCtx::X11(_) => crate::backend::BackendKind::X11,
            WmCtx::Wayland(_) => crate::backend::BackendKind::Wayland,
        }
    }

    /// Begin/end a compositor-owned keyboard mode. Wayland already owns the
    /// input stream; X11 needs an active grab so unmodified modal keys cannot
    /// leak to the focused client.
    pub fn begin_modal_keyboard(&mut self) -> bool {
        use crate::backend::LayoutInteractionOps;
        match self {
            WmCtx::X11(ctx) => ctx.begin_modal_keyboard(),
            WmCtx::Wayland(ctx) => ctx.begin_modal_keyboard(),
        }
    }

    pub fn end_modal_keyboard(&mut self) {
        use crate::backend::LayoutInteractionOps;
        match self {
            WmCtx::X11(ctx) => ctx.end_modal_keyboard(),
            WmCtx::Wayland(ctx) => ctx.end_modal_keyboard(),
        }
    }

    /// Refresh the backend-native visualization of a pending tree placement.
    pub fn update_layout_preview(&mut self, rect: Option<Rect>) {
        self.update_interaction_outline(rect, crate::types::InteractionOutlineStyle::Layout, None);
    }

    pub fn update_close_preview(&mut self, target: Option<WindowId>, rect: Option<Rect>) {
        self.update_interaction_outline(rect, crate::types::InteractionOutlineStyle::Close, target);
    }

    fn update_interaction_outline(
        &mut self,
        rect: Option<Rect>,
        style: crate::types::InteractionOutlineStyle,
        target: Option<WindowId>,
    ) {
        let previous = self.core().state().interaction.layout_preview;
        let previous_style = self.core().state().interaction.layout_preview_style;
        if previous == rect && (rect.is_none() || previous_style == style) {
            return;
        }
        if rect.is_none() {
            self.core_mut()
                .state_mut()
                .interaction
                .pointer_placement_cache = None;
        }
        // Keyboard navigation changes a discrete virtual target and benefits
        // from interpolation. Pointer previews must track motion immediately.
        let animate = previous.is_some()
            && rect.is_some()
            && self.core().behavior().animated
            && self.current_mode().tree_placement().is_some();
        self.core_mut().state_mut().interaction.layout_preview = rect;
        self.core_mut().state_mut().interaction.layout_preview_style = style;
        let duration =
            self.core()
                .config()
                .animations
                .scale_duration(std::time::Duration::from_millis(
                    crate::constants::animation::WAYLAND_DEFAULT_ANIMATION_MILLIS,
                ));
        use crate::backend::LayoutInteractionOps;
        match self {
            WmCtx::X11(ctx) => ctx.layout_preview_changed(rect, style, target, animate, duration),
            WmCtx::Wayland(ctx) => {
                ctx.layout_preview_changed(rect, style, target, animate, duration)
            }
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
        let monitor_id = self
            .core()
            .model()
            .client(win)
            .map(|client| client.monitor_id);
        let Some(monitor_id) = monitor_id else {
            return;
        };
        self.core_mut().model_mut().raise_client_in_z_order(win);
        // Reapply the complete policy projection rather than visually raising
        // just this surface, which could place an ordinary window over a
        // protected transient dialog until the next layout pass.
        crate::layouts::sync_monitor_z_order(self, monitor_id);
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
                    return;
                }

                // ConfigureWindow is authoritative for managed clients. Size
                // hints have already been applied by shared geometry policy,
                // so reading geometry back here only stalls the event loop.
                self.window_backend().resize_window(win, rect);
                let WmCtx::X11(x11) = self else {
                    unreachable!()
                };
                crate::client::sync_client_geometry(x11.core.model_mut(), win, rect);

                crate::backend::x11::focus::configure(x11.core.state, &x11.x11, win);
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
        let width = width.max(0);
        if let Some(client) = self.core_mut().model_mut().client_mut(win) {
            client.border_width = width;
        }
        self.window_backend().set_border_width(win, width);
    }

    /// Update root EWMH workspace/tag properties. X11 only; no-op on Wayland.
    pub fn update_ewmh_desktop_props(&mut self) {
        if let WmCtx::X11(ctx) = self {
            crate::backend::x11::update_ewmh_desktop_props(
                ctx.core.state,
                &ctx.x11,
                ctx.x11_runtime,
            );
        }
    }

    /// Persist a client's tag assignment in backend-native state after the
    /// model changed it.
    ///
    /// X11 mirrors tags into `_INSTANTWM_TAGS` so external tools can read
    /// them. Wayland has no equivalent client-visible protocol state, so this
    /// is a no-op there.
    pub fn sync_client_tag_props(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::set_client_tag_prop(
                ctx.core.state(),
                &ctx.x11,
                ctx.x11_runtime,
                win,
            ),
            WmCtx::Wayland(_) => {}
        }
    }

    /// Advertise (or clear) fullscreen state for a managed client through
    /// backend protocol state.
    ///
    /// X11 rewrites the window's `_NET_WM_STATE` atom list from the explicit
    /// flag. The Wayland compositor re-derives xdg_toplevel state from the
    /// authoritative model, so the flag is only used to decide *that* a sync
    /// is due.
    pub fn set_client_fullscreen_signal(&mut self, win: WindowId, fullscreen: bool) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::fullscreen::set_fullscreen_atoms(
                &ctx.x11,
                ctx.x11_runtime,
                win,
                fullscreen,
            ),
            WmCtx::Wayland(ctx) => ctx.wayland.sync_window_presentation(win),
        }
    }

    /// Advertise (or clear) maximize state for a managed client through
    /// backend protocol state.
    pub fn set_client_maximized_signal(&mut self, win: WindowId, maximized: bool) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::fullscreen::set_maximized_atoms(
                &ctx.x11,
                ctx.x11_runtime,
                win,
                maximized,
            ),
            WmCtx::Wayland(ctx) => ctx.wayland.sync_window_presentation(win),
        }
    }

    /// Backend effects for a client entering real fullscreen.
    ///
    /// X11 must strip the server-side border and raise the surface ahead of
    /// the next layout pass. Wayland needs no extra work: geometry flows
    /// through `move_resize` and z-order through the arrange pipeline.
    pub fn apply_entered_fullscreen_effects(&mut self, win: WindowId, monitor_rect: Rect) {
        use crate::backend::WindowOps;
        match self {
            WmCtx::X11(ctx) => {
                crate::backend::x11::fullscreen::remove_border(&ctx.x11, win);
                ctx.x11.configure_window_geometry(win, monitor_rect);
                ctx.x11.raise_window_visual_only(win);
            }
            WmCtx::Wayland(_) => {}
        }
    }

    /// Backend effects for a client leaving real fullscreen.
    ///
    /// X11 reinstates the server-side border from the model; Wayland borders
    /// are compositor-rendered and need no restore step.
    pub fn apply_exited_fullscreen_effects(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(ctx) => {
                crate::backend::x11::fullscreen::restore_border(&ctx.x11, ctx.core.model(), win)
            }
            WmCtx::Wayland(_) => {}
        }
    }

    /// Apply the backend-native border presentation for a client entering
    /// floating placement.
    ///
    /// X11 pushes the modeled border width to the server and switches to the
    /// floating border color scheme. Wayland renders borders from the model
    /// during its frame pass, so this carries no extra state.
    pub fn apply_floating_border_scheme(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(ctx) => {
                let border = ctx
                    .core
                    .model()
                    .client(win)
                    .map(|client| client.border_width)
                    .unwrap_or(0);
                ctx.x11.set_border_width(win, border);
                crate::backend::x11::floating::apply_floating_borderscheme(
                    &ctx.x11,
                    win,
                    ctx.x11_runtime,
                );
            }
            WmCtx::Wayland(_) => {}
        }
    }

    /// Run the shared visibility plan through the backend's native
    /// concealment mechanism (see `crate::client::visibility`).
    ///
    /// X11 keeps invisible windows mapped but parked off-screen; Wayland
    /// maps/unmaps surfaces.
    pub fn apply_visibility_plan(&mut self) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::visibility::apply_visibility(ctx),
            WmCtx::Wayland(ctx) => crate::backend::wayland::visibility::apply_visibility(ctx),
        }
    }

    /// Reveal a previously hidden managed client at protocol level.
    ///
    /// X11 maps the window, resets `WM_STATE`, stacks it above and animates
    /// it up from below the screen edge. Wayland windows become visible on
    /// the next arrange pass — which maps pending surfaces — so this carries
    /// no immediate work there.
    pub fn reveal_client(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::visibility::show(ctx, win),
            WmCtx::Wayland(_) => {}
        }
    }

    /// Conceal a managed client at protocol level (minimize semantics).
    pub fn conceal_client(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::visibility::hide(ctx, win),
            WmCtx::Wayland(ctx) => crate::backend::wayland::visibility::hide(ctx, win),
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
            let mon = self.core().model().expect_selected_monitor();
            self.pointer_backend().warp_to_point(mon.center());
            return;
        }

        let Some(c) = self.core().model().client(win).cloned() else {
            return;
        };

        let Some(ptr) = self.pointer_backend().pointer_location() else {
            return;
        };

        // Skip if already inside the window's border-aware outer bounds.
        let in_window = c.total_rect().contains_point(ptr);

        let on_bar = self.core().model().client_view(win).is_some_and(|view| {
            view.monitor
                .bar_contains_y(&self.core().model().clients, ptr.y)
        });

        if in_window || on_bar {
            return;
        }

        self.pointer_backend().warp_to_point(c.geo.center());
    }

    /// Warp unconditionally to the center of a client's current geometry.
    ///
    /// Unlike [`Self::warp_cursor_to_client`], this must not use geometry-only
    /// containment as an early return: after changing tags, the same screen
    /// coordinates may belong to an entirely different visible window.
    pub fn warp_cursor_to_client_center(&mut self, win: WindowId) {
        let Some(rect) = self.core().client_geo(win) else {
            return;
        };
        self.pointer_backend().warp_to_point(rect.center());
    }

    /// Take any in-flight geometry animation for `win`, returning its
    /// current visual rectangle without snapping to the (obsolete) target.
    ///
    /// This is the correct origin when a live interaction retargets a moving
    /// window. X11 samples its tracked transition at `now`; the Wayland
    /// compositor returns the last frame it actually displayed.
    pub fn take_current_animation_rect(&mut self, win: WindowId, now: Instant) -> Option<Rect> {
        match self {
            WmCtx::X11(x11) => x11.x11_runtime.take_current_window_animation_rect(win, now),
            WmCtx::Wayland(wl) => wl.wayland.take_current_window_animation_rect(win, now),
        }
    }

    /// Drop an in-flight geometry animation for `win` without applying its
    /// final target.
    pub fn cancel_window_animation(&mut self, win: WindowId) {
        match self {
            WmCtx::X11(x11) => x11.x11_runtime.cancel_window_animation(win),
            WmCtx::Wayland(wl) => wl.wayland.cancel_window_animation(win),
        }
    }

    /// Stage an animated move/resize toward `to`, starting from `from`.
    ///
    /// X11 parks the surface at `from` immediately and re-configures toward
    /// `to` on every tick. The Wayland compositor records an animation
    /// target on surface state and interpolates during frame callbacks.
    pub fn begin_window_animation(
        &mut self,
        win: WindowId,
        from: Rect,
        to: Rect,
        duration: Duration,
    ) {
        match self {
            WmCtx::X11(x11) => x11
                .x11_runtime
                .begin_window_animation(&x11.x11, win, from, to, duration),
            WmCtx::Wayland(wl) => wl.wayland.begin_window_animation(win, from, to, duration),
        }
    }

    /// Whether an in-flight animation already targets the same rectangle as
    /// `target` and should be preserved instead of restarted.
    pub fn has_inflight_animation_to(&self, win: WindowId, target: Rect) -> bool {
        match self {
            WmCtx::X11(x11) => x11.x11_runtime.window_animation_targets(win, target),
            WmCtx::Wayland(wl) => wl.wayland.window_animation_targets(win, target),
        }
    }

    /// Snap a newly managed surface to `rect` right away.
    ///
    /// Wayland intentionally leaves freshly spawned windows unmapped until
    /// their first layout exists, so the client must receive the
    /// authoritative configure before a decorative spawn transition runs —
    /// otherwise it flashes at its initial buffer size. X11 maps windows
    /// eagerly and needs no staging step.
    pub fn snap_deferred_spawn_geometry(&mut self, win: WindowId, rect: Rect) {
        use crate::backend::WindowOps;
        match self {
            WmCtx::X11(_) => {}
            WmCtx::Wayland(wl) => {
                wl.wayland.resize_window(win, rect);
                wl.wayland.flush();
            }
        }
    }

    /// Apply backend-native size-hint constraints after the shared hint
    /// math produced `adjusted`.
    ///
    /// X11 re-reads live ICCCM hints from the server, which remain the
    /// authoritative source under X. Wayland constrains against the hints
    /// cached on the model from the client's last commit.
    pub fn refine_size_hints(&mut self, win: WindowId, apply_hints: bool, adjusted: &mut Rect) {
        match self {
            WmCtx::X11(ctx) => {
                if apply_hints {
                    crate::backend::x11::geometry::apply_icccm_size_hints(
                        ctx.core.model_mut(),
                        &ctx.x11,
                        win,
                        adjusted,
                    );
                }
            }
            WmCtx::Wayland(ctx) => {
                if apply_hints && let Some(client) = ctx.core.model().client(win) {
                    let constrained = client.size_hints.constrain_size(
                        adjusted.size(),
                        client.min_aspect,
                        client.max_aspect,
                    );
                    *adjusted = adjusted.with_size(constrained);
                }
            }
        }
    }

    /// Repaint a client's border with the scheme implied by its mode and
    /// focus state.
    ///
    /// X11 owns server-side borders and must push colors explicitly;
    /// Wayland renders borders during its frame pass from model state.
    pub fn refresh_client_border_color(&mut self, win: WindowId, focused: bool) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::focus::refresh_border_color(
                ctx.core.state(),
                &ctx.x11,
                ctx.x11_runtime,
                win,
                focused,
            ),
            WmCtx::Wayland(_) => {}
        }
    }

    /// Refresh the backend's managed-window inventory.
    ///
    /// X11 maintains `_NET_CLIENT_LIST` ordering for EWMH tooling. Wayland
    /// exposes toplevels as protocol handles; no root property exists there.
    pub fn sync_client_list(&mut self) {
        if let WmCtx::X11(ctx) = self {
            crate::backend::x11::properties::update_client_list(
                ctx.core.state(),
                &ctx.x11,
                ctx.x11_runtime,
            );
        }
    }

    /// Inject backend launch-environment variables into a child command.
    ///
    /// Both backends already received `DESKTOP_STARTUP_ID` from the caller.
    /// Wayland additionally mints an XDG activation token anchored to the
    /// focused surface (carrying `context` so the launch can be correlated
    /// when activation arrives) and points XWayland clients at the
    /// compositor-owned display.
    pub fn prepare_launch_environment(
        &self,
        command: &mut std::process::Command,
        context: crate::client::LaunchContext,
    ) {
        match self {
            WmCtx::X11(_) => {}
            WmCtx::Wayland(wl) => {
                let selected_window = self.core().model().selected_win();
                wl.wayland
                    .prepare_launch_environment(command, selected_window, context);
            }
        }
    }

    /// Re-measure and resize the top bar for one monitor after a visibility
    /// or geometry change. X11 owns real bar windows; the Wayland
    /// compositor re-renders its scene on the next frame.
    pub fn refresh_monitor_top_bar(&mut self, monitor_id: MonitorId) {
        match self {
            WmCtx::X11(ctx) => {
                if let Some(monitor) = ctx.core.model().monitors.get(monitor_id).cloned() {
                    crate::backend::x11::bar::resize_bar_win(
                        ctx.core.state(),
                        &ctx.x11,
                        &*ctx.x11_runtime,
                        ctx.xembed_tray.as_ref(),
                        &monitor,
                    );
                }
                ctx.core.bar.mark_dirty();
            }
            WmCtx::Wayland(ctx) => {
                if !ctx.wayland.request_bar_redraw() {
                    ctx.core.bar.mark_dirty();
                }
            }
        }
    }

    /// Bottom-bar counterpart of [`Self::refresh_monitor_top_bar`].
    pub fn refresh_monitor_bottom_bar(&mut self, monitor_id: MonitorId) {
        match self {
            WmCtx::X11(ctx) => {
                if let Some(monitor) = ctx.core.model().monitors.get(monitor_id).cloned() {
                    crate::backend::x11::bar::resize_bottom_bar_win(
                        ctx.core.state(),
                        &ctx.x11,
                        &*ctx.x11_runtime,
                        &monitor,
                    );
                }
                ctx.core.bar.mark_dirty();
            }
            WmCtx::Wayland(ctx) => {
                if !ctx.wayland.request_bar_redraw() {
                    ctx.core.bar.mark_dirty();
                }
            }
        }
    }

    /// Destroy backend bar windows owned by a removed monitor.
    ///
    /// X11 bar windows are real server resources; Wayland bars are
    /// compositor scene elements with nothing to destroy.
    pub fn destroy_monitor_bar_window(&mut self, bar_win: WindowId) {
        match self {
            WmCtx::X11(ctx) => {
                let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                crate::backend::x11::monitor_helpers::destroy_monitor_bar(&mut wm_ctx, bar_win);
            }
            WmCtx::Wayland(_) => {}
        }
    }

    /// Redraw bars that were marked dirty since the last pass.
    ///
    /// The Wayland compositor re-renders through frame callbacks and its
    /// async bar worker; only X11 draws eagerly here.
    pub fn redraw_bars_if_dirty(&mut self) {
        if !self.core().bar.needs_redraw() {
            return;
        }
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::bar::draw_bars(&mut ctx.core, ctx.x11_runtime),
            WmCtx::Wayland(_) => {}
        }
    }

    /// Re-apply backend key grabs after keybind or mode changes.
    ///
    /// X11 relies on passive grabs; Wayland always sees keys first.
    pub fn refresh_key_grabs(&mut self) {
        if let WmCtx::X11(ctx) = self {
            crate::backend::x11::keyboard::grab_keys(ctx.core.state(), &ctx.x11, ctx.x11_runtime);
        }
    }

    /// Push current bar content to backend-owned surfaces after config or
    /// layout changes. Wayland re-renders from the shared snapshot.
    pub fn refresh_bar_content(&mut self) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::bar::update_bars(
                ctx.core.state_mut(),
                &ctx.x11,
                ctx.x11_runtime,
                ctx.xembed_tray.as_ref(),
            ),
            WmCtx::Wayland(_) => {}
        }
    }

    /// Republish the parsed status line through backend-owned bar surfaces.
    pub fn refresh_status_content(&mut self) {
        match self {
            WmCtx::X11(ctx) => crate::backend::x11::bar::update_status(
                &mut ctx.core,
                &ctx.x11,
                ctx.x11_runtime,
                ctx.xembed_tray,
            ),
            WmCtx::Wayland(_) => {}
        }
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

    /// Refresh bar rendering and synchronize backend-owned bar geometry for
    /// one monitor. Tag changes use this because bar visibility is per-tag.
    pub fn request_bar_geometry_update(&mut self, monitor_id: MonitorId) {
        self.refresh_monitor_top_bar(monitor_id);
        self.refresh_monitor_bottom_bar(monitor_id);
    }

    pub fn current_mode(&self) -> &crate::core_state::ActiveWmMode {
        &self.core().behavior().current_mode
    }

    pub fn set_current_mode(&mut self, mode: impl Into<crate::core_state::ActiveWmMode>) {
        let next_mode = mode.into();
        self.transition_current_mode(next_mode, crate::overview::ExitMode::RestorePrevious);
    }

    pub(crate) fn transition_current_mode(
        &mut self,
        next_mode: crate::core_state::ActiveWmMode,
        overview_exit: crate::overview::ExitMode,
    ) -> crate::core_state::ActiveWmMode {
        let previous_mode = std::mem::replace(
            &mut self.core_mut().behavior_mut().current_mode,
            next_mode.clone(),
        );
        if previous_mode == next_mode {
            return previous_mode;
        }

        crate::overview::handle_mode_transition(self, &previous_mode, &next_mode, overview_exit);
        if matches!(
            previous_mode,
            crate::core_state::ActiveWmMode::TreePlacement(_)
        ) {
            self.update_layout_preview(None);
            self.end_modal_keyboard();
        }
        self.request_bar_update();
        self.refresh_key_grabs();
        previous_mode
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

#[cfg(test)]
mod mode_transition_tests {
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::core_state::{ActiveWmMode, KeyboardTreePlacement};
    use crate::layouts::tree::PlacementTarget;
    use crate::types::{MonitorId, Point, Rect, TagMask, WindowId};
    use crate::wm::Wm;

    #[test]
    fn leaving_placement_clears_its_preview_through_the_mode_transition() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let placement = KeyboardTreePlacement::new(
            WindowId(1),
            MonitorId::default(),
            TagMask::EMPTY,
            vec![PlacementTarget {
                target: WindowId(2),
                side: None,
                candidate_index: 0,
                position: Point::new(10, 10),
            }],
            0,
        )
        .expect("valid placement");
        wm.core.behavior.current_mode = ActiveWmMode::TreePlacement(placement);
        wm.core.interaction.layout_preview = Some(Rect::new(0, 0, 100, 100));

        wm.ctx()
            .set_current_mode(ActiveWmMode::Named("resize".to_string()));

        assert_eq!(
            wm.core.behavior.current_mode,
            ActiveWmMode::Named("resize".to_string())
        );
        assert_eq!(wm.core.interaction.layout_preview, None);
    }
}
