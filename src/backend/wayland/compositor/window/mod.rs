use smithay::desktop::Window;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Size};

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::types::{Rect, WindowId};

pub mod animations;
pub mod classify;
pub mod focus;
pub mod hit_test;
pub mod lifecycle;
pub mod management;
pub mod properties;
pub mod x11;

pub use classify::WindowType;
pub(crate) use x11::is_unmanaged_x11_overlay;

/// Convert Smithay's currently displayed inner-surface geometry to the WM's
/// outer-origin/content-size rectangle convention.
///
/// `client.geo` uses the same convention, but represents the logical target.
/// Keeping this conversion separate prevents render code from accidentally
/// drawing a target geometry while the compositor is presenting an animation
/// frame somewhere else.
fn displayed_rect_from_space_geometry(
    location: Point<i32, Logical>,
    size: Size<i32, Logical>,
    border_width: i32,
) -> Rect {
    let border_width = border_width.max(0);
    Rect::new(
        location.x - border_width,
        location.y - border_width,
        size.w.max(1),
        size.h.max(1),
    )
}

fn committed_size_is_stale(
    pending_authoritative_size: Option<(i32, i32)>,
    committed_size: (i32, i32),
) -> bool {
    pending_authoritative_size.is_some_and(|configured| configured != committed_size)
}

/// Correlate a committed geometry observation with the latest outstanding
/// size configure using xdg serials.
///
/// Serials come from one globally ordered counter, so an acknowledgement
/// compares cleanly against any earlier request. Unlike size equality this
/// cannot alias two distinct requests that happen to share dimensions.
fn classify_geometry_response(
    outstanding: Option<smithay::utils::Serial>,
    answered: Option<smithay::utils::Serial>,
) -> crate::geometry::GeometryResponse {
    use crate::geometry::GeometryResponse;
    match (outstanding, answered) {
        (None, _) => GeometryResponse::Unsolicited,
        (Some(_), None) => GeometryResponse::Stale,
        (Some(requested), Some(answered)) if answered.is_no_older_than(&requested) => {
            GeometryResponse::Current
        }
        (Some(_), Some(_)) => GeometryResponse::Stale,
    }
}

impl WaylandState {
    /// Check if a window exists in the index.
    pub fn window_exists(&self, window: WindowId) -> bool {
        self.window_index.contains_key(&window)
    }

    /// Allocate a new window ID.
    pub(crate) fn alloc_window_id(&mut self) -> WindowId {
        loop {
            let id = self.next_window_id;
            self.next_window_id = self.next_window_id.wrapping_add(1).max(1);
            let window_id = WindowId::from(id);
            if !self.window_index.contains_key(&window_id) {
                return window_id;
            }
        }
    }

    /// Find a window by ID.
    pub(crate) fn find_window(&self, window: WindowId) -> Option<&Window> {
        self.window_index.get(&window)
    }

    /// Return the rectangle currently presented on screen for a managed
    /// window, in the core model's outer-origin/content-size convention.
    ///
    /// This deliberately reads Smithay space rather than `client.geo`:
    /// animations commit their logical destination immediately while the
    /// space element advances through intermediate displayed positions.
    pub(crate) fn displayed_window_rect(&self, window: &Window, border_width: i32) -> Option<Rect> {
        if let Some(marker) = window.user_data().get::<WindowIdMarker>()
            && let Some(frame) = self.displayed_animation_frame(marker.id)
        {
            return Some(frame);
        }
        let location = self.space.element_location(window)?;
        Some(displayed_rect_from_space_geometry(
            location,
            window.geometry().size,
            border_width,
        ))
    }

    /// Sync client size from the compositor's committed window state.
    ///
    /// Wayland resizes are configure-driven, so the client may commit a
    /// different size than the compositor requested.  Keep WM geometry
    /// width/height aligned with the actual surface, but preserve the
    /// authoritative WM position (`client.geo.x/y`).
    ///
    /// Position is always owned by the WM layer and flows one-way into
    /// the compositor via `sync_space_from_globals`.  We never read it
    /// back from the Smithay space.
    pub(crate) fn sync_client_size_from_window(&mut self, window: WindowId) {
        let Some(element) = self.find_window(window).cloned() else {
            return;
        };
        let committed = element.geometry();
        let new_w = committed.size.w.max(1);
        let new_h = committed.size.h.max(1);
        let acknowledged = self.native_acknowledged_configure(window, &element);

        self.push_command(
            crate::backend::wayland::commands::WmCommand::UpdateWindowSize {
                win: window,
                w: new_w,
                h: new_h,
                acknowledged_configure: acknowledged,
            },
        );
    }

    /// Read the serial of the configure this commit acknowledges.
    ///
    /// Classification is deferred until the queued observation is consumed so
    /// a newer request issued in the meantime still wins. When no request is
    /// outstanding the acknowledged serial can never influence the decision,
    /// so skip reading the surface state entirely.
    fn native_acknowledged_configure(
        &self,
        window: WindowId,
        element: &Window,
    ) -> Option<smithay::utils::Serial> {
        if !self.pending_size_configure.contains_key(&window) {
            return None;
        }
        element.toplevel().and_then(|toplevel| {
            toplevel.with_cached_state(|state| state.last_acked.as_ref().map(|c| c.serial))
        })
    }

    /// Whether the window's protocol surface is an xdg-shell toplevel (as
    /// opposed to an XWayland X11 surface).
    fn is_xdg_toplevel(&self, window: WindowId) -> bool {
        self.window_index
            .get(&window)
            .is_some_and(|element| element.toplevel().is_some())
    }

    /// Decide whether committed client size may update logical floating
    /// geometry, and maintain configure retry state for rejected/stale sizes.
    pub(crate) fn committed_size_may_update_model(
        &mut self,
        window: WindowId,
        new_w: i32,
        new_h: i32,
        acknowledged_configure: Option<smithay::utils::Serial>,
        client_size_is_authoritative: bool,
    ) -> bool {
        // Classify when the queued observation is consumed, not when it is
        // emitted. A newer pointer sample may have configured another size in
        // between; carrying the acknowledged serial preserves that ordering.
        let response = classify_geometry_response(
            self.pending_size_configure.get(&window).copied(),
            acknowledged_configure,
        );
        let decision =
            crate::geometry::reconcile_geometry_commit(response, client_size_is_authoritative);

        // A fullscreen client can commit its old buffer after the compositor
        // has restored floating mode. While that one-shot restore configure is
        // outstanding, only its size may feed back into logical geometry, and
        // every mismatching commit must keep prodding the client toward it —
        // including commits that are also stale for a newer request.
        let pending_authoritative_size = self.pending_authoritative_sizes.get(&window).copied();
        if committed_size_is_stale(pending_authoritative_size, (new_w, new_h)) {
            self.last_configured_size.remove(&window);
            self.request_space_sync();
            return false;
        }
        if response == crate::geometry::GeometryResponse::Stale {
            return false;
        }
        if decision.settle_request {
            self.pending_size_configure.remove(&window);
        }
        if pending_authoritative_size.is_some() {
            self.pending_authoritative_sizes.remove(&window);
        }

        // A floating client may legally constrain the current suggestion. The
        // accepted size must become both model and protocol state, so schedule
        // one convergence configure when it differs. Stale responses returned
        // above and therefore can never initiate the A <-> B feedback loop.
        // X11 surfaces carry position in their configures, so a client-committed
        // size must never make a future same-size placement skip the X11
        // configure that transports the new position.
        if decision.accept_client_size {
            let actual = (new_w.max(1), new_h.max(1));
            if self
                .last_configured_size
                .get(&window)
                .is_some_and(|&configured| configured != actual)
            {
                self.last_configured_size.remove(&window);
                self.request_space_sync();
            } else if self.is_xdg_toplevel(window) {
                self.last_configured_size.insert(window, actual);
            }
        }
        decision.accept_client_size
    }

    /// Request the compositor to warp the pointer to `(x, y)` in logical
    /// screen coordinates.  The warp is deferred until the next event-loop
    /// tick so that the pointer handle and the caller's `pointer_location`
    /// variable can both be updated consistently.
    pub fn request_warp(&mut self, x: f64, y: f64) {
        self.pending_warp = Some(Point::from((x, y)));
    }

    /// Reconcile xdg-toplevel's `resizing` state with the interaction model.
    /// Ending a resize emits the final configure without the resizing flag;
    /// redundant synchronization has no protocol effect.
    pub(crate) fn reconcile_interactive_resize(&mut self, desired: Option<WindowId>) {
        if self.active_resize == desired {
            return;
        }

        let ended = std::mem::replace(&mut self.active_resize, desired);
        if let Some(window) = ended.filter(|window| Some(*window) != desired) {
            if let Some(element) = self.find_window(window).cloned() {
                self.send_toplevel_configure(&element, None);
            }
        }
    }

    /// Consume and return the pending warp target, if any.
    pub fn take_pending_warp(&mut self) -> Option<Point<f64, Logical>> {
        self.pending_warp.take()
    }

    pub(crate) fn raise_unmanaged_x11_windows(&mut self) {
        let overlays: Vec<_> = self
            .windows_in_z_order()
            .into_iter()
            .filter(|(_, typ)| typ.is_overlay())
            .map(|(w, _)| w.clone())
            .collect();
        for w in overlays {
            self.space.raise_element(&w, false);
        }
    }

    /// Collect all overlay/unmanaged windows (dmenu, override-redirect popups,
    /// etc.) that should be rendered above the bar but below the cursor.
    ///
    /// Returns each window with its output-local logical render origin.
    ///
    /// A Space location addresses the window geometry, while rendering starts
    /// at the surface-tree origin. Keeping this conversion here gives explicit
    /// overlays the same coordinate semantics as Smithay's normal Space path.
    pub fn overlay_windows_for_render(
        &self,
        output: &Output,
    ) -> Vec<(Window, Point<i32, smithay::utils::Logical>)> {
        let Some(output_rect) = self.space.output_geometry(output) else {
            return Vec::new();
        };

        self.windows_in_z_order()
            .into_iter()
            .filter(|(_, typ)| typ.is_overlay())
            .filter_map(|(w, _)| {
                let loc = self.space.element_location(w)?;
                let mut window_rect = w.bbox_with_popups();
                window_rect.loc += loc - w.geometry().loc;
                if !output_rect.overlaps(window_rect) {
                    return None;
                }
                let render_origin = loc - w.geometry().loc - output_rect.loc;
                Some((w.clone(), render_origin))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_geometry_response, committed_size_is_stale, displayed_rect_from_space_geometry,
    };
    use smithay::utils::{Point, Serial, Size};

    use crate::types::WindowId;

    #[test]
    fn committed_size_must_match_an_authoritative_transition() {
        assert!(!committed_size_is_stale(None, (800, 600)));
        assert!(!committed_size_is_stale(Some((800, 600)), (800, 600)));
        assert!(committed_size_is_stale(Some((800, 600)), (1920, 1080)));
    }

    #[test]
    fn interactive_resize_reconciliation_is_idempotent() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let win = WindowId(23);

        state.reconcile_interactive_resize(Some(win));
        assert_eq!(state.active_resize, Some(win));

        state.reconcile_interactive_resize(Some(win));
        assert_eq!(state.active_resize, Some(win));

        state.reconcile_interactive_resize(None);
        assert_eq!(state.active_resize, None);
        state.reconcile_interactive_resize(None);
        assert_eq!(state.active_resize, None);
    }

    #[test]
    fn delayed_floating_commit_cannot_recreate_an_older_resize_request() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let _ = state.take_space_sync_pending();
        let win = WindowId(17);
        let older = (800, 600);
        let latest = (1200, 900);
        let older_serial = smithay::utils::Serial::from(7);
        let latest_serial = smithay::utils::Serial::from(9);
        state.last_configured_size.insert(win, latest);
        state.pending_size_configure.insert(win, latest_serial);

        assert!(!state.committed_size_may_update_model(
            win,
            older.0,
            older.1,
            Some(older_serial),
            true,
        ));
        assert_eq!(state.pending_size_configure.get(&win), Some(&latest_serial));
        assert_eq!(state.last_configured_size.get(&win), Some(&latest));
        assert!(!state.take_space_sync_pending());

        assert!(state.committed_size_may_update_model(
            win,
            latest.0,
            latest.1,
            Some(latest_serial),
            true,
        ));
        assert!(!state.pending_size_configure.contains_key(&win));
        assert_eq!(state.last_configured_size.get(&win), Some(&latest));
        assert!(!state.take_space_sync_pending());
    }

    /// Two distinct requests may share dimensions; the serial, not the size,
    /// must decide which request a commit answers. Here an in-flight commit
    /// acknowledges the older same-sized request and must stay stale.
    #[test]
    fn same_sized_requests_do_not_alias() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let _ = state.take_space_sync_pending();
        let win = WindowId(19);
        let size = (800, 600);
        state.pending_size_configure.insert(win, Serial::from(5));

        let older_request_acknowledged = classify_geometry_response(
            state.pending_size_configure.get(&win).copied(),
            Some(Serial::from(4)),
        );
        assert_eq!(
            older_request_acknowledged,
            crate::geometry::GeometryResponse::Stale
        );
        assert!(!state.committed_size_may_update_model(
            win,
            size.0,
            size.1,
            Some(Serial::from(4)),
            true,
        ));
        assert_eq!(
            state.pending_size_configure.get(&win),
            Some(&Serial::from(5))
        );
    }

    #[test]
    fn current_constrained_commit_schedules_one_protocol_convergence() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let _ = state.take_space_sync_pending();
        let win = WindowId(18);
        let requested = (1200, 900);
        let constrained = (1198, 898);
        state.last_configured_size.insert(win, requested);
        state.pending_size_configure.insert(win, Serial::from(11));

        assert!(state.committed_size_may_update_model(
            win,
            constrained.0,
            constrained.1,
            Some(Serial::from(11)),
            true,
        ));
        assert!(!state.pending_size_configure.contains_key(&win));
        assert!(!state.last_configured_size.contains_key(&win));
        assert!(state.take_space_sync_pending());
    }

    /// Layout-owned windows settle an outstanding request but never let the
    /// committed size clear configure de-duplication or schedule a reconfigure;
    /// otherwise a stale fullscreen buffer could restart the resize loop.
    #[test]
    fn layout_owned_current_commit_settles_without_touching_configure_state() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let _ = state.take_space_sync_pending();
        let win = WindowId(20);
        let configured = (1200, 900);
        let committed = (1000, 1000);
        state.last_configured_size.insert(win, configured);
        state.pending_size_configure.insert(win, Serial::from(3));

        assert!(!state.committed_size_may_update_model(
            win,
            committed.0,
            committed.1,
            Some(Serial::from(3)),
            false,
        ));
        assert!(!state.pending_size_configure.contains_key(&win));
        assert_eq!(state.last_configured_size.get(&win), Some(&configured));
        assert!(!state.take_space_sync_pending());
    }

    /// A commit that is stale for the latest request must still prod the
    /// retry for an outstanding authoritative (fullscreen restore) size.
    #[test]
    fn stale_commit_during_authoritative_transition_still_prods_a_retry() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let _ = state.take_space_sync_pending();
        let win = WindowId(21);
        let restore_size = (800, 600);
        let fullscreen_buffer = (1920, 1080);
        state.last_configured_size.insert(win, restore_size);
        state.pending_size_configure.insert(win, Serial::from(6));
        state.pending_authoritative_sizes.insert(win, restore_size);

        assert!(!state.committed_size_may_update_model(
            win,
            fullscreen_buffer.0,
            fullscreen_buffer.1,
            Some(Serial::from(5)),
            true,
        ));
        assert!(!state.last_configured_size.contains_key(&win));
        assert_eq!(
            state.pending_authoritative_sizes.get(&win),
            Some(&restore_size)
        );
        assert!(state.take_space_sync_pending());
    }

    #[test]
    fn displayed_geometry_converts_inner_space_location_to_core_coordinates() {
        let displayed =
            displayed_rect_from_space_geometry(Point::from((103, 204)), Size::from((800, 600)), 3);

        assert_eq!(displayed, crate::types::Rect::new(100, 201, 800, 600));
    }
}
