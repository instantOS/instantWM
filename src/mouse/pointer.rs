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
    Sidebar(SidebarTarget),
    Client(WindowId),
    Root {
        monitor_id: MonitorId,
    },
}

impl PointerRegion {
    /// Return the config-binding target for regions owned by the binding system.
    ///
    /// The sidebar is a compositor gesture and deliberately has no configurable
    /// button target: its press, motion, and release must share one lifecycle.
    pub fn binding_target(self) -> Option<crate::types::ButtonTarget> {
        match self {
            PointerRegion::Bar { pos, .. } => Some(crate::types::ButtonTarget::Bar(pos)),
            PointerRegion::Sidebar(_) => None,
            PointerRegion::Client(_) => Some(crate::types::ButtonTarget::ClientWin),
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

#[inline]
pub fn right_sidebar_rect(monitor_rect: Rect, bar_height: i32) -> Rect {
    let min_y = sidebar_min_y(monitor_rect, bar_height);
    Rect::new(
        monitor_rect.right() - crate::types::SIDEBAR_WIDTH,
        min_y,
        crate::types::SIDEBAR_WIDTH,
        (monitor_rect.bottom() - min_y).max(0),
    )
}

/// Cheap sidebar-only hit test for pointer motion.
pub fn sidebar_target_at(model: &WmModel, root: Point) -> Option<SidebarTarget> {
    let monitor_id = model.monitors.id_intersecting_rect(point_rect(root))?;
    let mon = model.monitor(monitor_id)?;
    let rect = right_sidebar_rect(mon.monitor_rect, mon.bar_height);
    rect.contains_point(root).then_some(SidebarTarget {
        monitor_id,
        edge: EdgeDirection::Right,
        rect,
    })
}

/// Resolve the sidebar only when compositor desktop is exposed at `root`.
///
/// A 50px invisible region must not steal input from a client. Motion and
/// button handlers both call this policy so the offered cursor and press owner
/// cannot disagree.
pub fn desktop_sidebar_target_at(
    model: &WmModel,
    root: Point,
    window_at_root: Option<WindowId>,
) -> Option<SidebarTarget> {
    window_at_root
        .is_none()
        .then(|| sidebar_target_at(model, root))
        .flatten()
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

    if let Some(target) = desktop_sidebar_target_at(core.model(), root, clicked_win) {
        return PointerRegion::Sidebar(target);
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
    use super::{desktop_sidebar_target_at, right_sidebar_rect};
    use crate::model::WmModel;
    use crate::types::{Monitor, Point, Rect, SIDEBAR_WIDTH, WindowId};

    #[test]
    fn right_sidebar_rect_uses_shared_width_and_monitor_origin() {
        let rect = right_sidebar_rect(Rect::new(100, 200, 1920, 1080), 30);

        assert_eq!(rect.x, 100 + 1920 - SIDEBAR_WIDTH);
        assert_eq!(rect.y, 200 + 30 + 60);
        assert_eq!(rect.w, SIDEBAR_WIDTH);
        assert_eq!(rect.h, 1080 - 30 - 60);
    }

    #[test]
    fn right_sidebar_rect_never_has_negative_height() {
        let rect = right_sidebar_rect(Rect::new(0, 0, 100, 40), 30);

        assert_eq!(rect.h, 0);
    }

    #[test]
    fn desktop_sidebar_never_steals_a_client_point() {
        let mut model = WmModel::new();
        model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1920, 1080),
            bar_height: 30,
            ..Monitor::default()
        });
        let point = Point::new(1900, 500);

        assert!(desktop_sidebar_target_at(&model, point, None).is_some());
        assert_eq!(
            desktop_sidebar_target_at(&model, point, Some(WindowId(7))),
            None
        );
    }
}
