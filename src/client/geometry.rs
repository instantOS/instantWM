//! Client geometry: resizing, size-hint enforcement, and dimension helpers.
//!
//! # Responsibilities
//!
//! * [`WmCtx::move_resize`](crate::contexts::WmCtx::move_resize) – high-level geometry API.
//! * [`apply_size_hints`] – clamp a proposed geometry to ICCCM size hints.
//! * [`scale_client`] – resize a client to a percentage of its monitor.
//!
//! # Dimension helpers
//!
//! Client dimensions including borders are available as methods:
//! * [`Client::total_width`](crate::types::Client::total_width) – total width including borders
//! * [`Client::total_height`](crate::types::Client::total_height) – total height including borders

use crate::geometry::MoveResizeOptions;
use crate::model::WmModel;
use crate::types::{Client, Monitor, Point, Rect, Size, SnapPosition, WindowId};

/// Record the resolved geometry of a managed client.
///
/// Backends may request a resize optimistically, but this helper is called only
/// once the WM knows the geometry that actually applies to the window right
/// now. Shared state lives here so backend callbacks do not each reinvent the
/// current and saved-floating geometry update contract.
pub fn sync_client_geometry(model: &mut WmModel, win: WindowId, rect: Rect) {
    let work_area = model.client_view(win).map(|view| view.monitor.work_rect());
    if let Some(client) = model.client_mut(win) {
        client.update_geometry(rect);
        if client.mode().is_floating()
            && client.snap_status == SnapPosition::None
            && let Some(work_area) = work_area
        {
            client.save_floating_placement(rect, work_area);
        }
    }
}

/// Why a tiled client is acquiring floating geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatingPlacementIntent {
    /// A non-pointer action such as a key binding, IPC command, or mode toggle.
    RestoreOrCenter,
    /// A pointer gesture must keep the same point under the cursor as the
    /// tiled window changes size.
    PreservePointerAnchor(Point),
}

const FIRST_FLOAT_MAX_NUMERATOR: i32 = 3;
const FIRST_FLOAT_MAX_DENOMINATOR: i32 = 4;

/// Resolve the authoritative rectangle for a tiled-to-floating transition.
///
/// Previous real floating placements are restored and rebased from their
/// reference work area. First-time transitions use the client's pre-layout
/// preferred size when available, otherwise its tiled size, capped to 75% of
/// the work area. Every result is fully contained inside the current work area.
pub fn resolve_floating_transition(
    client: &Client,
    work_area: Rect,
    intent: FloatingPlacementIntent,
) -> Rect {
    if !work_area.is_valid() {
        return client.geo;
    }

    let border = client.border_width.max(0);
    let saved = client.saved_floating_placement();
    let mut size = saved
        .map(|placement| placement.rect.size())
        .or_else(|| client.preferred_floating_size())
        .unwrap_or_else(|| client.geo.size());

    let maximum = if saved.is_some() {
        Size::new(work_area.w, work_area.h)
    } else {
        Size::new(
            work_area.w * FIRST_FLOAT_MAX_NUMERATOR / FIRST_FLOAT_MAX_DENOMINATOR,
            work_area.h * FIRST_FLOAT_MAX_NUMERATOR / FIRST_FLOAT_MAX_DENOMINATOR,
        )
    };
    size.w = size.w.max(1).min((maximum.w - 2 * border).max(1));
    size.h = size.h.max(1).min((maximum.h - 2 * border).max(1));

    let total_w = size.w + 2 * border;
    let total_h = size.h + 2 * border;
    let position = match intent {
        FloatingPlacementIntent::PreservePointerAnchor(pointer) => {
            let current_total_w = client.total_width().max(1);
            let current_total_h = client.total_height().max(1);
            let anchor_x =
                ((pointer.x - client.geo.x) as f64 / current_total_w as f64).clamp(0.0, 1.0);
            let anchor_y =
                ((pointer.y - client.geo.y) as f64 / current_total_h as f64).clamp(0.0, 1.0);
            Point::new(
                pointer.x - (anchor_x * total_w as f64).round() as i32,
                pointer.y - (anchor_y * total_h as f64).round() as i32,
            )
        }
        FloatingPlacementIntent::RestoreOrCenter => saved
            .map(|placement| rebase_saved_position(placement, work_area, total_w, total_h))
            .unwrap_or_else(|| {
                Point::new(
                    work_area.x + (work_area.w - total_w) / 2,
                    work_area.y + (work_area.h - total_h) / 2,
                )
            }),
    };

    contain_floating_rect(
        Rect::new(position.x, position.y, size.w, size.h),
        work_area,
        border,
    )
}

fn rebase_saved_position(
    placement: crate::types::SavedFloatingPlacement,
    work_area: Rect,
    total_w: i32,
    total_h: i32,
) -> Point {
    fn axis(
        old_pos: i32,
        old_start: i32,
        old_len: i32,
        new_start: i32,
        new_len: i32,
        total: i32,
    ) -> i32 {
        let old_travel = (old_len - total).max(0);
        let new_travel = (new_len - total).max(0);
        if old_travel == 0 {
            return new_start + new_travel / 2;
        }
        let fraction = ((old_pos - old_start) as f64 / old_travel as f64).clamp(0.0, 1.0);
        new_start + (fraction * new_travel as f64).round() as i32
    }

    Point::new(
        axis(
            placement.rect.x,
            placement.reference_work_area.x,
            placement.reference_work_area.w,
            work_area.x,
            work_area.w,
            total_w,
        ),
        axis(
            placement.rect.y,
            placement.reference_work_area.y,
            placement.reference_work_area.h,
            work_area.y,
            work_area.h,
            total_h,
        ),
    )
}

pub(crate) fn contain_floating_rect(mut rect: Rect, work_area: Rect, border: i32) -> Rect {
    let border = border.max(0);
    rect.w = rect.w.max(1).min((work_area.w - 2 * border).max(1));
    rect.h = rect.h.max(1).min((work_area.h - 2 * border).max(1));
    let total_w = rect.w + 2 * border;
    let total_h = rect.h + 2 * border;
    rect.x = if total_w >= work_area.w {
        work_area.x
    } else {
        rect.x.clamp(work_area.x, work_area.right() - total_w)
    };
    rect.y = if total_h >= work_area.h {
        work_area.y
    } else {
        rect.y.clamp(work_area.y, work_area.bottom() - total_h)
    };
    rect
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatingPlacementKind {
    /// A newly mapped window for which the compositor owns the position.
    NewAutomatic,
    /// A newly mapped window with an explicit client-provided position.
    NewExplicit,
}

/// Resolve a floating window rectangle before it becomes authoritative WM state.
///
/// Floating clients often provide stale coordinates, especially transient
/// dialogs restored from a previous monitor setup.  Keep the app-provided size
/// but ensure the position is usable on the target monitor.  Parent-relative
/// placement is preferred for new transients without a usable position.
#[cfg(test)]
fn resolve_floating_placement(
    model: &WmModel,
    win: WindowId,
    requested: Rect,
    kind: FloatingPlacementKind,
    parent: Option<WindowId>,
) -> Rect {
    let Some(view) = model.client_view(win) else {
        return requested;
    };
    let client = view.client;
    if !client.mode().is_floating() {
        return requested;
    }

    let work_rect = view.monitor.work_rect();
    let parent_rect = parent.and_then(|parent| model.client(parent).map(|client| client.geo));

    resolve_floating_placement_for_client(client, work_rect, requested, kind, parent_rect)
}

fn resolve_floating_placement_for_client(
    client: &Client,
    work_rect: Rect,
    requested: Rect,
    kind: FloatingPlacementKind,
    parent_rect: Option<Rect>,
) -> Rect {
    if !work_rect.is_valid() {
        return requested;
    }

    let mut rect = requested;
    rect.w = rect.w.max(1);
    rect.h = rect.h.max(1);

    let total_w = rect.total_width(client.border_width);
    let total_h = rect.total_height(client.border_width);
    let fully_outside_x = rect.x + total_w <= work_rect.x || rect.x >= work_rect.right();
    let fully_outside_y = rect.y + total_h <= work_rect.y || rect.y >= work_rect.bottom();

    // Wayland toplevels cannot provide an absolute position, and X11 clients
    // without USPosition/PPosition have likewise delegated placement to the
    // WM. Center those new windows over their parent or, for standalone
    // windows, in the monitor work area. Explicit X11 positions are preserved
    // unless they need clamping to remain usable.
    let used_automatic_position = if matches!(kind, FloatingPlacementKind::NewAutomatic) {
        if let Some(parent_rect) = parent_rect {
            rect.x = parent_rect.x + (parent_rect.w - total_w) / 2;
            rect.y = parent_rect.y + (parent_rect.h - total_h) / 2;
        } else {
            rect.x = work_rect.x + (work_rect.w - total_w) / 2;
            rect.y = work_rect.y + (work_rect.h - total_h) / 2;
        }
        true
    } else {
        false
    };

    rect.x = normalize_spawn_axis(
        rect.x,
        total_w,
        work_rect.x,
        work_rect.w,
        fully_outside_x && !used_automatic_position,
    );
    rect.y = normalize_spawn_axis(
        rect.y,
        total_h,
        work_rect.y,
        work_rect.h,
        fully_outside_y && !used_automatic_position,
    );
    rect
}

/// Compute a saner initial position for a newly managed floating client.
///
/// Automatically placed windows are centered over a transient parent or in
/// their monitor's work area. Explicitly positioned X11 windows retain their
/// requested position, subject to work-area clamping. The returned rect keeps
/// the original size and only adjusts position.
pub fn sane_floating_spawn_rect(
    model: &WmModel,
    win: WindowId,
    parent: Option<WindowId>,
    position_is_explicit: bool,
) -> Option<Rect> {
    let view = model.client_view(win)?;
    let client = view.client;
    if !client.mode().is_floating() {
        return None;
    }

    let kind = if position_is_explicit {
        FloatingPlacementKind::NewExplicit
    } else {
        FloatingPlacementKind::NewAutomatic
    };
    let parent_rect = parent.and_then(|parent| model.client(parent).map(|client| client.geo));
    let rect = resolve_floating_placement_for_client(
        client,
        view.monitor.work_rect(),
        client.geo,
        kind,
        parent_rect,
    );

    rect.differs_from(&client.geo).then_some(rect)
}

fn normalize_spawn_axis(
    pos: i32,
    total_len: i32,
    bounds_pos: i32,
    bounds_len: i32,
    fully_outside: bool,
) -> i32 {
    if total_len >= bounds_len {
        return bounds_pos;
    }

    let min_pos = bounds_pos;
    let max_pos = bounds_pos + bounds_len - total_len;

    if fully_outside {
        bounds_pos + (bounds_len - total_len) / 2
    } else {
        pos.clamp(min_pos, max_pos)
    }
}

/// Result of [`apply_size_hints`] indicating whether backend/protocol client
/// constraints should also be applied to the dimensions.
pub(crate) struct SizeHintsOutcome {
    pub should_apply_client_hints: bool,
}

pub fn apply_size_hints(
    model: &WmModel,
    config: &crate::core_state::RuntimeConfig,
    win: WindowId,
    rect: &mut Rect,
    interact: bool,
) -> SizeHintsOutcome {
    let view = match model.client_view(win) {
        Some(view) => view,
        None => {
            return SizeHintsOutcome {
                should_apply_client_hints: false,
            };
        }
    };
    let client = view.client;

    let old_geo = client.geo;
    let border_width = client.border_width;
    let should_apply_hints = config.window.resize_hints
        || client.mode().is_floating()
        || is_floating_layout(model, view.monitor);

    // Phase 1: Ensure positive dimensions.
    rect.w = rect.w.max(1);
    rect.h = rect.h.max(1);

    // Phase 2: Clamp position to keep window visible.
    clamp_position_to_bounds(
        &config.derived.display,
        rect,
        Some(view.monitor.work_rect()),
        interact,
        old_geo.total_width(border_width),
        old_geo.total_height(border_width),
    );

    // Phase 3: Enforce minimum size (bar height).
    let bar_height = config.derived.bar_height;
    rect.enforce_minimum(bar_height, bar_height);

    SizeHintsOutcome {
        should_apply_client_hints: should_apply_hints,
    }
}

/// Check if the given rect differs from the client's current stored geometry.
pub(crate) fn size_hints_changed(model: &WmModel, win: WindowId, rect: &Rect) -> bool {
    model
        .client(win)
        .map(|c| rect.differs_from(&c.geo))
        .unwrap_or(false)
}

/// Clamp window position to keep it within usable screen area.
fn clamp_position_to_bounds(
    display: &crate::core_state::DisplayConfig,
    geo: &mut Rect,
    work_rect: Option<Rect>,
    interact: bool,
    total_w: i32,
    total_h: i32,
) {
    if interact {
        let screen = Rect::new(0, 0, display.width, display.height);
        geo.clamp_position(&screen, total_w, total_h);
    } else if let Some(wr) = work_rect {
        geo.clamp_position(&wr, total_w, total_h);
    }
}

/// Check if the client's monitor is using a floating layout.
fn is_floating_layout(model: &WmModel, monitor: &Monitor) -> bool {
    if model.is_overview_active_on(monitor) {
        return false;
    }

    !monitor.is_tiling_layout()
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Calculate the target rect for scaling a client to `scale` percent of its monitor.
fn calculate_scaled_geometry(
    monitor_id: crate::types::MonitorId,
    old_geo: Rect,
    border_width: i32,
    scale: i32,
    get_monitor_rect: impl FnOnce(crate::types::MonitorId) -> Rect,
) -> Rect {
    let mon_rect = get_monitor_rect(monitor_id);

    let new_w = old_geo.w * scale / 100;
    let new_h = old_geo.h * scale / 100;
    let new_x = mon_rect.x + (mon_rect.w - new_w) / 2 - border_width;
    let new_y = mon_rect.y + (mon_rect.h - new_h) / 2 - border_width;

    Rect {
        x: new_x,
        y: new_y,
        w: new_w,
        h: new_h,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FloatingPlacementIntent, FloatingPlacementKind, resolve_floating_placement,
        resolve_floating_transition, sane_floating_spawn_rect, sync_client_geometry,
    };
    use crate::core_state::CoreState;
    use crate::model::WmModel;
    use crate::types::{
        Client, EdgeDirection, Monitor, MonitorId, Point, Rect, Size, SnapPosition, TagMask,
        WindowId,
    };

    fn outer_rect(rect: Rect, border: i32) -> Rect {
        Rect::new(
            rect.x,
            rect.y,
            rect.total_width(border),
            rect.total_height(border),
        )
    }

    fn globals_with_floating_client(rect: Rect, border_width: i32, work_rect: Rect) -> CoreState {
        let mut globals = CoreState::default();

        let mut monitor = Monitor::new_with_values(true, EdgeDirection::Top);
        monitor.monitor_rect = Rect::new(work_rect.x, work_rect.y, work_rect.w, work_rect.h);
        monitor.available_rect = monitor.monitor_rect;
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        globals.model.monitors.push(monitor);

        let mut client = Client::default();
        client.win = WindowId::from(1_u32);
        client.monitor_id = MonitorId::default();
        client.set_tag_mask(TagMask::single(1).unwrap());
        client.replace_mode_with_base(crate::types::BaseClientMode::Floating);
        client.border_width = border_width;
        client.geo = rect;
        client.save_floating_placement(rect, work_rect);
        client.old_geo = rect;
        globals.model.insert_client(client);

        globals
    }

    fn tiled_client(rect: Rect, border_width: i32) -> Client {
        Client {
            geo: rect,
            border_width,
            old_border_width: border_width,
            ..Client::default()
        }
    }

    #[test]
    fn first_float_from_a_full_work_area_tile_is_bounded_and_centered_below_bar() {
        let work = Rect::new(1920, 32, 1600, 868);
        let client = tiled_client(Rect::new(1920, 32, 1600, 868), 2);

        let resolved =
            resolve_floating_transition(&client, work, FloatingPlacementIntent::RestoreOrCenter);

        assert_eq!(resolved.size(), Size::new(1196, 647));
        let center = outer_rect(resolved, 2).center();
        assert!((center.x - work.center().x).abs() <= 1);
        assert!((center.y - work.center().y).abs() <= 1);
        assert!(resolved.y >= work.y);
        assert!(outer_rect(resolved, 2).bottom() <= work.bottom());
    }

    #[test]
    fn first_float_prefers_the_pre_layout_client_size() {
        let work = Rect::new(0, 30, 1920, 1050);
        let mut client = tiled_client(work, 1);
        client.set_preferred_floating_size(Size::new(800, 600));

        let resolved =
            resolve_floating_transition(&client, work, FloatingPlacementIntent::RestoreOrCenter);

        assert_eq!(resolved.size(), Size::new(800, 600));
        assert_eq!(outer_rect(resolved, 1).center(), work.center());
    }

    #[test]
    fn saved_float_is_rebased_between_different_work_areas() {
        let old_work = Rect::new(0, 30, 1920, 1050);
        let new_work = Rect::new(1920, 0, 1280, 1024);
        let saved = Rect::new(1116, 476, 800, 600);
        let mut client = tiled_client(Rect::new(1920, 0, 1280, 1024), 2);
        client.save_floating_placement(saved, old_work);

        let resolved = resolve_floating_transition(
            &client,
            new_work,
            FloatingPlacementIntent::RestoreOrCenter,
        );

        assert_eq!(resolved.size(), saved.size());
        assert_eq!(outer_rect(resolved, 2).right(), new_work.right());
        assert_eq!(outer_rect(resolved, 2).bottom(), new_work.bottom());
    }

    #[test]
    fn oversized_saved_float_is_shrunk_and_fully_contained() {
        let work = Rect::new(100, 50, 800, 600);
        let mut client = tiled_client(work, 3);
        client.save_floating_placement(
            Rect::new(-500, -400, 2000, 1600),
            Rect::new(0, 0, 2560, 1440),
        );

        let resolved =
            resolve_floating_transition(&client, work, FloatingPlacementIntent::RestoreOrCenter);

        assert_eq!(resolved, Rect::new(100, 50, 794, 594));
        assert_eq!(outer_rect(resolved, 3), work);
    }

    #[test]
    fn pointer_promotion_preserves_the_pointer_fraction() {
        let work = Rect::new(0, 30, 1200, 770);
        let mut client = tiled_client(Rect::new(0, 30, 600, 770), 0);
        client.set_preferred_floating_size(Size::new(480, 360));
        let pointer = Point::new(450, 222);

        let resolved = resolve_floating_transition(
            &client,
            work,
            FloatingPlacementIntent::PreservePointerAnchor(pointer),
        );

        assert_eq!(resolved, Rect::new(90, 132, 480, 360));
        assert_eq!(pointer.x - resolved.x, 360);
        assert_eq!(pointer.y - resolved.y, 90);
    }

    #[test]
    fn geometry_sync_records_only_real_floating_placements() {
        let mut model = WmModel::new();
        let monitor_id = model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1000, 800),
            available_rect: Rect::new(0, 30, 1000, 770),
            ..Monitor::default()
        });
        let win = WindowId(77);
        model.insert_client(Client {
            win,
            monitor_id,
            geo: Rect::new(0, 30, 1000, 770),
            ..Client::default()
        });

        sync_client_geometry(&mut model, win, Rect::new(10, 40, 900, 700));
        assert_eq!(model.client(win).unwrap().saved_floating_placement(), None);

        model
            .client_mut(win)
            .unwrap()
            .replace_mode_with_base(crate::types::BaseClientMode::Floating);
        let floating = Rect::new(100, 100, 700, 500);
        sync_client_geometry(&mut model, win, floating);
        let saved = model
            .client(win)
            .unwrap()
            .saved_floating_placement()
            .unwrap();
        assert_eq!(saved.rect, floating);
        assert_eq!(saved.reference_work_area, Rect::new(0, 30, 1000, 770));

        let free_placement = saved;
        model.client_mut(win).unwrap().snap_status = SnapPosition::Left;
        sync_client_geometry(&mut model, win, Rect::new(0, 30, 500, 770));
        assert_eq!(
            model.client(win).unwrap().saved_floating_placement(),
            Some(free_placement)
        );
    }

    #[test]
    fn sane_floating_spawn_rect_clamps_under_bar() {
        let globals = globals_with_floating_client(
            Rect::new(100, 0, 500, 300),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect =
            sane_floating_spawn_rect(&globals.model, WindowId::from(1_u32), None, true).unwrap();
        assert_eq!(rect.y, 32);
    }

    #[test]
    fn sane_floating_spawn_rect_centers_when_completely_offscreen() {
        let globals = globals_with_floating_client(
            Rect::new(-4000, -3000, 500, 300),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect =
            sane_floating_spawn_rect(&globals.model, WindowId::from(1_u32), None, true).unwrap();
        assert_eq!(rect.x, 708);
        assert_eq!(rect.y, 404);
    }

    #[test]
    fn sane_floating_spawn_rect_anchors_large_windows_to_work_area() {
        let globals = globals_with_floating_client(
            Rect::new(200, 200, 1900, 1100),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect =
            sane_floating_spawn_rect(&globals.model, WindowId::from(1_u32), None, true).unwrap();
        assert_eq!(rect.x, 16);
        assert_eq!(rect.y, 32);
    }

    #[test]
    fn app_requested_floating_geometry_is_clamped_before_sync() {
        let globals = globals_with_floating_client(
            Rect::new(100, 100, 500, 300),
            2,
            Rect::new(0, 32, 1920, 1048),
        );

        let rect = resolve_floating_placement(
            &globals.model,
            WindowId::from(1_u32),
            Rect::new(-4000, -3000, 500, 300),
            FloatingPlacementKind::NewExplicit,
            None,
        );

        assert_eq!(rect.x, 708);
        assert_eq!(rect.y, 404);
    }

    #[test]
    fn new_offscreen_transient_prefers_parent_center() {
        let mut globals = globals_with_floating_client(
            Rect::new(-4000, -3000, 400, 200),
            2,
            Rect::new(0, 32, 1920, 1048),
        );
        let mut parent = Client::default();
        parent.win = WindowId::from(2_u32);
        parent.monitor_id = MonitorId::default();
        parent.geo = Rect::new(500, 300, 800, 600);
        globals.model.insert_client(parent);

        let rect = resolve_floating_placement(
            &globals.model,
            WindowId::from(1_u32),
            Rect::new(-4000, -3000, 400, 200),
            FloatingPlacementKind::NewAutomatic,
            Some(WindowId::from(2_u32)),
        );

        assert_eq!(rect.x, 698);
        assert_eq!(rect.y, 498);
    }

    #[test]
    fn new_automatic_float_centers_in_work_area() {
        let globals = globals_with_floating_client(
            Rect::new(0, 0, 500, 300),
            2,
            Rect::new(1920, 32, 1920, 1048),
        );

        let rect =
            sane_floating_spawn_rect(&globals.model, WindowId::from(1_u32), None, false).unwrap();
        assert_eq!(rect, Rect::new(2628, 404, 500, 300));
    }

    #[test]
    fn new_explicit_float_preserves_usable_position() {
        let globals = globals_with_floating_client(
            Rect::new(2100, 180, 500, 300),
            2,
            Rect::new(1920, 32, 1920, 1048),
        );

        assert!(
            sane_floating_spawn_rect(&globals.model, WindowId::from(1_u32), None, true,).is_none()
        );
    }

    #[test]
    fn automatic_oversized_float_anchors_to_work_area() {
        let globals = globals_with_floating_client(
            Rect::new(0, 0, 2000, 1200),
            2,
            Rect::new(1920, 32, 1920, 1048),
        );

        let rect =
            sane_floating_spawn_rect(&globals.model, WindowId::from(1_u32), None, false).unwrap();
        assert_eq!(rect, Rect::new(1920, 32, 2000, 1200));
    }
}

/// Resize `win` to `scale` percent of its monitor dimensions, centred on screen.
///
/// `scale` is an integer percentage (e.g. `75` means 75 %).
pub fn scale_client(ctx: &mut crate::contexts::WmCtx<'_>, win: WindowId, scale: i32) {
    let target = {
        let core = ctx.core();
        let c = match core.model().client(win) {
            Some(c) => c,
            None => return,
        };
        calculate_scaled_geometry(c.monitor_id, c.geo, c.border_width, scale, |mid| {
            core.state()
                .model
                .monitors
                .get(mid)
                .map(|m| m.monitor_rect)
                .unwrap_or(c.geo)
        })
    };

    ctx.move_resize(win, target, MoveResizeOptions::hinted_immediate(false));
}
