use smithay::desktop::Window;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Size};

use crate::backend::wayland::compositor::WaylandState;
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
    pub(crate) fn displayed_window_rect(
        &self,
        window: WindowId,
        border_width: i32,
    ) -> Option<Rect> {
        let element = self.find_window(window)?;
        let location = self.space.element_location(element)?;
        Some(displayed_rect_from_space_geometry(
            location,
            element.geometry().size,
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

        self.push_command(
            crate::backend::wayland::commands::WmCommand::UpdateWindowSize {
                win: window,
                w: new_w,
                h: new_h,
            },
        );
    }

    /// Decide whether committed client size may update logical floating
    /// geometry, and maintain configure retry state for rejected/stale sizes.
    pub(crate) fn committed_size_may_update_model(
        &mut self,
        window: WindowId,
        new_w: i32,
        new_h: i32,
    ) -> bool {
        // A fullscreen client can commit its old buffer after the compositor
        // has restored floating mode. While that one-shot restore configure is
        // outstanding, only its size may feed back into logical geometry.
        let pending_authoritative_size = self.pending_authoritative_sizes.get(&window).copied();
        let stale_for_authoritative_size =
            committed_size_is_stale(pending_authoritative_size, (new_w, new_h));
        if stale_for_authoritative_size {
            self.last_configured_size.remove(&window);
            self.request_space_sync();
            return false;
        }
        if pending_authoritative_size.is_some() {
            self.pending_authoritative_sizes.remove(&window);
        }

        // Outside an authoritative transition, floating clients may legally
        // commit a size different from the request. Invalidate configure
        // de-duplication so layout-owned modes can resend their target, while
        // still forwarding the actual size for floating-mode reconciliation.
        if self
            .last_configured_size
            .get(&window)
            .is_some_and(|&size| size != (new_w, new_h))
        {
            self.last_configured_size.remove(&window);
            self.request_space_sync();
        }
        true
    }

    /// Request the compositor to warp the pointer to `(x, y)` in logical
    /// screen coordinates.  The warp is deferred until the next event-loop
    /// tick so that the pointer handle and the caller's `pointer_location`
    /// variable can both be updated consistently.
    pub fn request_warp(&mut self, x: f64, y: f64) {
        self.pending_warp = Some(Point::from((x, y)));
    }

    pub(crate) fn begin_interactive_resize(&mut self, window: WindowId) {
        self.active_resizes.insert(window);
    }

    pub(crate) fn end_interactive_resize(&mut self, window: WindowId) {
        self.active_resizes.remove(&window);
        if let Some(element) = self.find_window(window).cloned() {
            self.send_toplevel_configure(&element, None);
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
    use super::{committed_size_is_stale, displayed_rect_from_space_geometry};
    use smithay::utils::{Point, Size};

    #[test]
    fn committed_size_must_match_an_authoritative_transition() {
        assert!(!committed_size_is_stale(None, (800, 600)));
        assert!(!committed_size_is_stale(Some((800, 600)), (800, 600)));
        assert!(committed_size_is_stale(Some((800, 600)), (1920, 1080)));
    }

    #[test]
    fn displayed_geometry_converts_inner_space_location_to_core_coordinates() {
        let displayed =
            displayed_rect_from_space_geometry(Point::from((103, 204)), Size::from((800, 600)), 3);

        assert_eq!(displayed, crate::types::Rect::new(100, 201, 800, 600));
    }
}
