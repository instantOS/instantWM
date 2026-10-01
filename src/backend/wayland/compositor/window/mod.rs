use smithay::desktop::Window;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Size};

use crate::backend::wayland::compositor::{WaylandState, WindowIdMarker};
use crate::types::{Rect, WindowId};

pub mod animations;
pub mod classify;
pub mod focus;
pub(super) mod geometry_sync;
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

    /// Observe a native Wayland client's committed size.
    ///
    /// Wayland resizes are configure-driven, so the client may commit a
    /// different size than the compositor requested.  Keep WM geometry
    /// width/height aligned with the actual surface, but preserve the
    /// authoritative WM position (`client.geo.x/y`).
    ///
    /// Position is always owned by the WM layer and flows one-way into
    /// the compositor via `sync_space_from_globals`.  We never read it
    /// back from the Smithay space.
    pub(crate) fn observe_native_committed_size(&mut self, window: WindowId) {
        let Some(element) = self.find_window(window).cloned() else {
            return;
        };
        debug_assert!(element.x11_surface().is_none());
        let committed = element.geometry();
        let new_w = committed.size.w.max(1);
        let new_h = committed.size.h.max(1);
        let acknowledged = Self::native_acknowledged_configure(&element);

        self.push_command(
            crate::backend::wayland::commands::WmCommand::ObserveCommittedSize {
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
    /// a newer scheduled or sent request still wins. Always carry the serial:
    /// a request can be sent before the observation leaves the command queue.
    fn native_acknowledged_configure(element: &Window) -> Option<smithay::utils::Serial> {
        element.toplevel().and_then(|toplevel| {
            toplevel.with_cached_state(|state| state.last_acked.as_ref().map(|c| c.serial))
        })
    }

    /// Decide whether committed client size may update logical floating
    /// geometry. Protocol convergence uses the same lifecycle as WM resizes.
    pub(crate) fn native_commit_may_update_model(
        &mut self,
        window: WindowId,
        new_w: i32,
        new_h: i32,
        acknowledged_configure: Option<smithay::utils::Serial>,
        client_size_is_authoritative: bool,
    ) -> bool {
        // A queued observation can outlive unmanagement. It must not create
        // protocol state for a surface whose lifecycle has already ended.
        let Some(sync) = self.geometry_sync.get_mut(&window) else {
            return false;
        };
        let decision = sync.observe(
            crate::types::Size::new(new_w.max(1), new_h.max(1)),
            acknowledged_configure,
            client_size_is_authoritative,
        );
        if decision.needs_dispatch {
            self.request_space_sync();
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
        if let Some(window) = ended.filter(|window| Some(*window) != desired)
            && let Some(element) = self.find_window(window).cloned()
        {
            self.send_toplevel_configure(&element, None);
        }
    }

    pub(crate) fn is_interactive_resize(&self, window: WindowId) -> bool {
        self.active_resize == Some(window)
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
    use super::displayed_rect_from_space_geometry;
    use crate::types::{Size as ContentSize, WindowId};
    use smithay::utils::{Point, Serial, Size};

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
    fn constrained_response_requests_dispatch_but_stale_response_does_not() {
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let _ = state.take_space_sync_pending();
        let win = WindowId(18);
        state
            .geometry_sync
            .entry(win)
            .or_default()
            .sent(ContentSize::new(1200, 900), Some(Serial::from(11)));
        assert!(!state.native_commit_may_update_model(
            win,
            1920,
            1080,
            Some(Serial::from(10)),
            true,
        ));
        assert!(!state.take_space_sync_pending());
        assert!(
            state.native_commit_may_update_model(win, 1198, 898, Some(Serial::from(11)), true,)
        );
        assert_eq!(
            state.geometry_sync.get(&win).unwrap().scheduled_size(),
            Some(ContentSize::new(1198, 898))
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
