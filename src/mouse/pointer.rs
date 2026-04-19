//! Shared pointer hit testing.
//!
//! Keep motion helpers cheap: monitor lookup plus rectangle math only.  Richer
//! button classification is allowed to touch bar hit caches because clicks are
//! rare compared with motion events.

use crate::contexts::CoreCtx;
use crate::types::{BarPosition, EdgeDirection, MonitorId, Rect, SidebarTarget, WindowId};

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
    pub fn to_button_target(self) -> crate::types::ButtonTarget {
        match self {
            PointerRegion::Bar { pos, .. } => crate::types::ButtonTarget::Bar(pos),
            PointerRegion::Sidebar(_) => crate::types::ButtonTarget::SideBar,
            PointerRegion::Client(_) => crate::types::ButtonTarget::ClientWin,
            PointerRegion::Root { .. } => crate::types::ButtonTarget::Root,
        }
    }
}

#[inline]
pub(crate) fn point_rect(root_x: i32, root_y: i32) -> Rect {
    Rect::new(root_x, root_y, 1, 1)
}

#[inline]
fn sidebar_min_y(monitor_rect: Rect, bar_height: i32) -> i32 {
    monitor_rect.y + bar_height.max(1) + 60
}

#[inline]
pub fn right_sidebar_rect(monitor_rect: Rect, bar_height: i32) -> Rect {
    let min_y = sidebar_min_y(monitor_rect, bar_height);
    Rect::new(
        monitor_rect.x + monitor_rect.w - crate::types::SIDEBAR_WIDTH,
        min_y,
        crate::types::SIDEBAR_WIDTH,
        (monitor_rect.y + monitor_rect.h - min_y).max(0),
    )
}

/// Cheap sidebar-only hit test for pointer motion.
pub fn sidebar_target_at(core: &CoreCtx<'_>, root_x: i32, root_y: i32) -> Option<SidebarTarget> {
    let monitor_id = crate::types::find_monitor_by_rect(
        core.globals().monitors.monitors(),
        &point_rect(root_x, root_y),
    )?;
    let mon = core.globals().monitor(monitor_id)?;
    let rect = right_sidebar_rect(mon.monitor_rect, mon.bar_height);
    rect.contains_point(root_x, root_y)
        .then_some(SidebarTarget {
            monitor_id,
            edge: EdgeDirection::Right,
            rect,
        })
}

/// Full click classification shared by X11 and Wayland button handlers.
pub fn button_region_at(
    core: &mut CoreCtx<'_>,
    root_x: i32,
    root_y: i32,
    clicked_win: Option<WindowId>,
) -> PointerRegion {
    if let Some((monitor_id, pos)) =
        crate::bar::resolve_bar_position_at_root(core, root_x, root_y, true)
    {
        return PointerRegion::Bar { monitor_id, pos };
    }

    if let Some(target) = sidebar_target_at(core, root_x, root_y) {
        if target.monitor_id != core.globals().selected_monitor_id() {
            core.globals_mut().set_selected_monitor(target.monitor_id);
        }
        return PointerRegion::Sidebar(target);
    }

    if let Some(win) = clicked_win {
        return PointerRegion::Client(win);
    }

    let monitor_id = crate::types::find_monitor_by_rect(
        core.globals().monitors.monitors(),
        &point_rect(root_x, root_y),
    )
    .unwrap_or_else(|| core.globals().selected_monitor_id());
    if monitor_id != core.globals().selected_monitor_id() {
        core.globals_mut().set_selected_monitor(monitor_id);
    }
    PointerRegion::Root { monitor_id }
}

#[cfg(test)]
mod tests {
    use super::right_sidebar_rect;
    use crate::types::{Rect, SIDEBAR_WIDTH};

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
}
