//! Shared pointer hit testing.
//!
//! Keep motion helpers cheap: monitor lookup plus rectangle math only.  Richer
//! button classification is allowed to touch bar hit caches because clicks are
//! rare compared with motion events.

use crate::contexts::CoreCtx;
use crate::model::WmModel;
use crate::types::{
    BarPosition, BottomBarTarget, EdgeDirection, MonitorId, Point, Rect, SidebarTarget, WindowId,
};

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
            PointerRegion::BottomBar { .. } => Some(crate::types::ButtonTarget::BottomBar),
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
    let monitor = model.monitors.monitor_intersecting_rect(point_rect(root))?;
    let bottom_band = if monitor.bottom_bar_visible(&model.clients) {
        monitor.bottom_bar_height
    } else {
        0
    };
    let rect = right_sidebar_rect(monitor.monitor_rect, monitor.bar_height, bottom_band);
    rect.contains_point(root).then_some(SidebarTarget {
        monitor_id: monitor.id(),
        edge: EdgeDirection::Right,
        rect,
        gesture_threshold: (monitor.monitor_rect.h / 30).max(1),
    })
}

/// Cheap bottom-bar-only hit test for pointer press ownership.
///
/// Mirrors `button_region_at`'s strip classification: the strip must be
/// visible on the monitor under the pointer.
pub fn bottom_bar_target_at(model: &WmModel, root: Point) -> Option<BottomBarTarget> {
    let monitor = model.monitors.monitor_intersecting_rect(point_rect(root))?;
    monitor
        .bottom_bar_contains_y(&model.clients, root.y)
        .then_some(BottomBarTarget {
            monitor_id: monitor.id(),
            gesture_threshold: (monitor.monitor_rect.w / 30).max(1),
        })
}

/// Full click classification shared by X11 and Wayland button handlers.
pub fn button_region_at(
    core: &CoreCtx<'_>,
    root: Point,
    clicked_win: Option<WindowId>,
) -> PointerRegion {
    let target = crate::bar::root_bar_target_at(core, root);
    if let Some(crate::bar::RootBarTarget::OnBar { monitor, position }) = target {
        return PointerRegion::Bar {
            monitor_id: monitor.id(),
            pos: position,
        };
    }

    // Scope the test to the output under the pointer. Otherwise a strip on
    // one output can swallow clicks at the same Y coordinate on another.
    if let Some(target) = target
        && target
            .monitor()
            .bottom_bar_contains_y(&core.model().clients, root.y)
    {
        return PointerRegion::BottomBar {
            monitor_id: target.monitor().id(),
        };
    }

    if let Some(win) = clicked_win {
        return PointerRegion::Client(win);
    }

    let monitor_id = target.map_or_else(
        || core.model().selected_monitor_id(),
        |target| target.monitor().id(),
    );
    PointerRegion::Root { monitor_id }
}

#[cfg(test)]
mod tests {
    use super::{
        PointerRegion, bottom_bar_target_at, button_region_at, right_sidebar_rect,
        sidebar_target_at,
    };
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

        let mut short = Monitor::new_with_values();
        short.show_bottom_bar = true;
        short.bottom_bar_height = 30;
        short.monitor_rect = Rect::new(0, 0, 1920, 1080);
        short.set_available_rect(short.monitor_rect);
        let short_id = wm.core.model.monitors.allocate_id();
        short.monitor_id = short_id;

        let mut tall = Monitor::new_with_values();
        tall.show_bottom_bar = false;
        tall.bottom_bar_height = 30;
        tall.monitor_rect = Rect::new(1920, 0, 1920, 1200);
        tall.set_available_rect(tall.monitor_rect);
        tall.monitor_id = wm.core.model.monitors.allocate_id();
        wm.core.model.monitors.restore(vec![short, tall]);

        let core = crate::contexts::CoreCtx::new(
            &mut wm.core,
            &mut wm.work,
            &mut wm.running,
            &mut wm.bar,
            &mut wm.focus,
        );

        assert_eq!(
            button_region_at(&core, Point::new(100, 1060), None),
            PointerRegion::BottomBar {
                monitor_id: short_id
            }
        );
        assert_eq!(
            button_region_at(&core, Point::new(2000, 1060), Some(WindowId::from(99_u32)),),
            PointerRegion::Client(WindowId::from(99_u32))
        );
    }

    #[test]
    fn bottom_bar_target_at_respects_visibility_and_y_band() {
        let mut model = WmModel::new();
        let mut mon = Monitor::new_with_values();
        mon.show_bottom_bar = true;
        mon.bottom_bar_height = 30;
        mon.monitor_rect = Rect::new(0, 0, 1920, 1080);
        mon.set_available_rect(mon.monitor_rect);
        let monitor_id = model.monitors.push(mon);

        let inside = Point::new(500, 1060);
        let outside = Point::new(500, 1000);

        let target = bottom_bar_target_at(&model, inside).expect("bottom-bar hit");
        assert_eq!(target.monitor_id, monitor_id);
        assert_eq!(target.gesture_threshold, 64);
        assert_eq!(bottom_bar_target_at(&model, outside), None);

        model.monitor_mut(monitor_id).unwrap().show_bottom_bar = false;
        assert_eq!(bottom_bar_target_at(&model, inside), None);
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

        let target = sidebar_target_at(&model, point).expect("sidebar hit");
        assert_eq!(target.gesture_threshold, 36);
    }
}
