//! Shared pointer hit testing.
//!
//! Keep motion helpers cheap: monitor lookup plus rectangle math only.  Richer
//! button classification is allowed to touch bar hit caches because clicks are
//! rare compared with motion events.

use crate::contexts::CoreCtx;
use crate::model::WmModel;
use crate::types::{BarPosition, EdgeDirection, MonitorId, Point, Rect, SidebarTarget, WindowId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerRegion {
    Bar {
        monitor_id: MonitorId,
        pos: BarPosition,
    },
    Client(WindowId),
    /// Visible bottom gesture strip. Not a binding target: presses here are
    /// swallowed so the strip neither acts like a bar nor falls through to
    /// desktop (root) bindings.
    BottomBar {
        monitor_id: MonitorId,
    },
    Root {
        monitor_id: MonitorId,
    },
}

impl PointerRegion {
    /// Return the config-binding target for regions owned by the binding system.
    ///
    pub fn binding_target(self) -> Option<crate::types::ButtonTarget> {
        match self {
            PointerRegion::Bar { pos, .. } => Some(crate::types::ButtonTarget::Bar(pos)),
            PointerRegion::Client(_) => Some(crate::types::ButtonTarget::ClientWin),
            PointerRegion::BottomBar { .. } => None,
            PointerRegion::Root { .. } => Some(crate::types::ButtonTarget::Root),
        }
    }
}

#[inline]
pub(crate) fn point_rect(root: Point) -> Rect {
    Rect::new(root.x, root.y, 1, 1)
}

#[inline]
fn sidebar_min_y(monitor_rect: Rect, bar_height: i32) -> i32 {
    monitor_rect.y + bar_height.max(1) + 60
}

/// `bottom_band` is the height of a visible bottom bar that the gesture strip
/// must stay above (0 when no bottom bar is visible).
#[inline]
pub fn right_sidebar_rect(monitor_rect: Rect, bar_height: i32, bottom_band: i32) -> Rect {
    let min_y = sidebar_min_y(monitor_rect, bar_height);
    let max_y = (monitor_rect.bottom() - bottom_band.max(0)).max(min_y);
    Rect::new(
        monitor_rect.right() - crate::types::SIDEBAR_WIDTH,
        min_y,
        crate::types::SIDEBAR_WIDTH,
        (max_y - min_y).max(0),
    )
}

/// Cheap sidebar-only hit test for pointer motion.
pub fn sidebar_target_at(model: &WmModel, root: Point) -> Option<SidebarTarget> {
    let monitor_id = model.monitors.id_intersecting_rect(point_rect(root))?;
    let mon = model.monitor(monitor_id)?;
    let bottom_band = if mon.bottom_bar_visible(&model.clients) {
        mon.bottom_bar_height
    } else {
        0
    };
    let rect = right_sidebar_rect(mon.monitor_rect, mon.bar_height, bottom_band);
    rect.contains_point(root).then_some(SidebarTarget {
        monitor_id,
        edge: EdgeDirection::Right,
        rect,
    })
}

/// Full click classification shared by X11 and Wayland button handlers.
pub fn button_region_at(
    core: &mut CoreCtx<'_>,
    root: Point,
    clicked_win: Option<WindowId>,
) -> PointerRegion {
    if let Some((monitor_id, pos)) = crate::bar::resolve_bar_position_at_root(core, root) {
        return PointerRegion::Bar { monitor_id, pos };
    }

    // Scope the test to the output under the pointer. Otherwise a strip on
    // one output can swallow clicks at the same Y coordinate on another.
    if let Some(monitor_id) = core.model().monitors.id_intersecting_rect(point_rect(root))
        && let Some(mon) = core.model().monitor(monitor_id)
        && mon.bottom_bar_contains_y(&core.model().clients, root.y)
    {
        return PointerRegion::BottomBar { monitor_id };
    }

    if let Some(win) = clicked_win {
        return PointerRegion::Client(win);
    }

    let monitor_id = core
        .model()
        .monitors
        .id_intersecting_rect(point_rect(root))
        .unwrap_or_else(|| core.model().selected_monitor_id());
    PointerRegion::Root { monitor_id }
}

#[cfg(test)]
mod tests {
    use super::{PointerRegion, button_region_at, right_sidebar_rect, sidebar_target_at};
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::model::WmModel;
    use crate::types::{Monitor, Point, Rect, SIDEBAR_WIDTH, WindowId};

    #[test]
    fn right_sidebar_rect_uses_shared_width_and_monitor_origin() {
        let rect = right_sidebar_rect(Rect::new(100, 200, 1920, 1080), 30, 0);

        assert_eq!(rect.x, 100 + 1920 - SIDEBAR_WIDTH);
        assert_eq!(rect.y, 200 + 30 + 60);
        assert_eq!(rect.w, SIDEBAR_WIDTH);
        assert_eq!(rect.h, 1080 - 30 - 60);
    }

    #[test]
    fn right_sidebar_rect_never_has_negative_height() {
        let rect = right_sidebar_rect(Rect::new(0, 0, 100, 40), 30, 0);

        assert_eq!(rect.h, 0);
    }

    #[test]
    fn right_sidebar_rect_stops_above_a_visible_bottom_bar() {
        let rect = right_sidebar_rect(Rect::new(0, 0, 1920, 1080), 30, 40);

        assert_eq!(rect.x, 1920 - SIDEBAR_WIDTH);
        assert_eq!(rect.y, 90);
        assert_eq!(rect.h, 1080 - 90 - 40);
    }

    #[test]
    fn bottom_bar_hit_test_is_scoped_to_the_pointer_monitor() {
        let mut wm = crate::wm::Wm::new(Backend::new_wayland(WaylandBackend::new()));

        let mut short = Monitor::new_with_values(true);
        short.show_bottom_bar = true;
        short.bottom_bar_height = 30;
        short.monitor_rect = Rect::new(0, 0, 1920, 1080);
        short.set_available_rect(short.monitor_rect);
        let short_id = wm.core.model.monitors.allocate_id();
        short.monitor_id = short_id;

        let mut tall = Monitor::new_with_values(true);
        tall.show_bottom_bar = false;
        tall.bottom_bar_height = 30;
        tall.monitor_rect = Rect::new(1920, 0, 1920, 1200);
        tall.set_available_rect(tall.monitor_rect);
        tall.monitor_id = wm.core.model.monitors.allocate_id();
        wm.core.model.monitors.restore(vec![short, tall]);

        let mut core = crate::contexts::CoreCtx::new(
            &mut wm.core,
            &mut wm.work,
            &mut wm.running,
            &mut wm.bar,
            &mut wm.focus,
        );

        assert_eq!(
            button_region_at(&mut core, Point::new(100, 1060), None),
            PointerRegion::BottomBar {
                monitor_id: short_id
            }
        );
        assert_eq!(
            button_region_at(
                &mut core,
                Point::new(2000, 1060),
                Some(WindowId::from(99_u32)),
            ),
            PointerRegion::Client(WindowId::from(99_u32))
        );
    }

    #[test]
    fn global_sidebar_hit_test_depends_only_on_monitor_geometry() {
        let mut model = WmModel::new();
        model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            bar_height: 30,
            ..Monitor::default()
        });
        let point = Point::new(1900, 500);

        assert!(sidebar_target_at(&model, point).is_some());
    }
}
