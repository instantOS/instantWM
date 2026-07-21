use std::collections::{HashMap, HashSet};

use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutOutput;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, Size, TagMask, WindowId};

pub const OVERVIEW_MODE_NAME: &str = "overview";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitMode {
    RestorePrevious,
    ToSelectedWindow,
}

#[derive(Debug, Clone)]
pub struct OverviewState {
    restore_tags: TagMask,
    /// Stable bottom-to-top card order captured on entry. Focus changes update
    /// the normal model stack, but must not reshuffle or obscure overview cards.
    window_order: Vec<WindowId>,
    /// Geometry before overview first moved each window. `Client::float_geo`
    /// cannot serve as the undo log because logical overview moves update it.
    restore_geometry: HashMap<WindowId, Rect>,
}

impl OverviewState {
    pub(crate) fn new(
        restore_tags: TagMask,
        window_order: Vec<WindowId>,
        restore_geometry: HashMap<WindowId, Rect>,
    ) -> Self {
        Self {
            restore_tags,
            window_order,
            restore_geometry,
        }
    }
}

pub(crate) fn handle_mode_transition(
    ctx: &mut WmCtx<'_>,
    previous_mode: &crate::core_state::ActiveWmMode,
    next_mode: &crate::core_state::ActiveWmMode,
    overview_exit: ExitMode,
) {
    match (previous_mode, next_mode) {
        (crate::core_state::ActiveWmMode::Overview, crate::core_state::ActiveWmMode::Overview) => {}
        (crate::core_state::ActiveWmMode::Overview, _) => exit(ctx, overview_exit),
        (_, crate::core_state::ActiveWmMode::Overview) => enter(ctx),
        _ => {}
    }
}

/// Exit overview mode with a specific [`ExitMode`].
///
pub fn exit_overview(ctx: &mut WmCtx<'_>, mode: ExitMode) {
    ctx.transition_current_mode(crate::core_state::ActiveWmMode::Default, mode);
}

fn enter(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().model().selected_monitor_id();
    let all_tags = TagMask::all(ctx.core().model().tags.count());
    let window_order = {
        let model = ctx.core().model();
        let monitor = model.expect_selected_monitor();
        initial_window_order(monitor, &model.clients, all_tags)
    };
    let restore_geometry = window_order
        .iter()
        .filter_map(|win| {
            ctx.core()
                .model()
                .client(*win)
                .map(|client| (*win, client.geo))
        })
        .collect();

    {
        let mon = ctx.core_mut().model_mut().expect_selected_monitor_mut();
        if mon.overview_state.is_some() {
            return;
        }
        let restore_tags = mon.selected_tags();
        let _ = mon.set_selected_tags_with_history(all_tags);
        mon.overview_state = Some(OverviewState::new(
            restore_tags,
            window_order,
            restore_geometry,
        ));
    }

    crate::focus::focus(ctx, None);
    ctx.core_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
}

fn exit(ctx: &mut WmCtx<'_>, mode: ExitMode) {
    let state = {
        let mon = ctx.core_mut().model_mut().expect_selected_monitor_mut();
        mon.overview_state.take()
    };

    let Some(state) = state else { return };

    let selected_monitor_id = ctx.core().model().selected_monitor_id();

    match mode {
        ExitMode::RestorePrevious => {
            let restore_mask = state.restore_tags;
            restore_window_geometry(ctx, selected_monitor_id, &state.restore_geometry);

            if !restore_mask.is_empty() {
                let _ = {
                    let mon = ctx.core_mut().model_mut().expect_selected_monitor_mut();
                    mon.set_selected_tags_with_history(restore_mask)
                };
            }

            crate::focus::focus(ctx, None);
        }
        ExitMode::ToSelectedWindow => {
            let selected_window = ctx.core().model().selected_win();
            let selected_tags = selected_window.and_then(|win| {
                ctx.core()
                    .state()
                    .model
                    .client(win)
                    .map(|c| c.tags.without_scratchpad())
                    .filter(|tags| !tags.is_empty())
            });
            let restore_mask = state.restore_tags;

            restore_window_geometry(ctx, selected_monitor_id, &state.restore_geometry);

            let target_mask = selected_tags.or(Some(restore_mask));
            if let Some(mask) = target_mask
                && !mask.is_empty()
            {
                let _ = {
                    let mon = ctx.core_mut().model_mut().expect_selected_monitor_mut();
                    mon.set_selected_tags_with_history(mask)
                };
            }

            if let Some(win) = selected_window {
                crate::focus::focus(ctx, Some(win));
            } else {
                crate::focus::focus(ctx, None);
            }
        }
    }

    ctx.core_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
}

pub fn toggle_overview(ctx: &mut WmCtx<'_>, _mask: TagMask) {
    if ctx.core().model().is_overview_active() {
        exit_overview(ctx, ExitMode::ToSelectedWindow);
        return;
    }

    if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .clients
        .is_empty()
    {
        return;
    }

    ctx.set_current_mode(crate::core_state::ActiveWmMode::Overview);
}

pub fn cancel_overview(ctx: &mut WmCtx<'_>, _mask: TagMask) {
    if !ctx.core().model().is_overview_active() {
        return;
    }

    ctx.reset_mode();
}

#[derive(Debug, Clone)]
pub(crate) struct OverviewLayout {
    pub moves: Vec<LayoutOutput>,
    /// Complete managed card order, bottom-to-top.
    pub z_order: Vec<WindowId>,
}

/// Arrange visible windows as an overlapping card hand. Client sizes are
/// preserved: only their origins change. Because origins advance monotonically
/// in the same direction as z-order, every card retains an exposed, clickable
/// strip even when later cards overlap it.
pub fn compute(monitor: &mut Monitor, clients: &HashMap<WindowId, Client>) -> OverviewLayout {
    let selected_tags = monitor.selected_tags();
    let newly_visible = monitor
        .clients
        .iter()
        .copied()
        .filter_map(|win| {
            let client = clients.get(&win)?;
            overview_eligible(client, selected_tags).then_some((win, client.geo))
        })
        .collect::<Vec<_>>();
    if let Some(state) = monitor.overview_state.as_mut() {
        for (win, rect) in newly_visible {
            state.restore_geometry.entry(win).or_insert(rect);
        }
    }
    let mut ordered_windows = monitor
        .overview_state
        .as_ref()
        .map(|state| state.window_order.clone())
        .unwrap_or_default();
    ordered_windows.retain(|win| {
        clients
            .get(win)
            .is_some_and(|client| overview_eligible(client, selected_tags))
    });
    // Windows mapped during overview join at the front of the hand without
    // disturbing the positions of existing cards.
    for &win in &monitor.clients {
        if !ordered_windows.contains(&win)
            && clients
                .get(&win)
                .is_some_and(|client| overview_eligible(client, selected_tags))
        {
            ordered_windows.push(win);
        }
    }

    let client_info: Vec<(WindowId, Size)> = ordered_windows
        .iter()
        .copied()
        .filter_map(|win| {
            let c = clients.get(&win)?;
            Some((win, Size::new(c.geo.w.max(1), c.geo.h.max(1))))
        })
        .collect();

    if client_info.is_empty() {
        return OverviewLayout {
            moves: Vec::new(),
            z_order: Vec::new(),
        };
    }

    let work_rect = monitor.work_rect();
    let sizes = client_info
        .iter()
        .map(|(_, size)| *size)
        .collect::<Vec<_>>();
    let rects = card_hand_rects(work_rect, &sizes);
    let moves = client_info
        .iter()
        .zip(rects)
        .map(|(&(win, _), rect)| LayoutOutput {
            win,
            rect,
            options: MoveResizeOptions::immediate(),
        })
        .collect();

    // Include non-card managed windows below the hand. This matters on X11,
    // where sibling-based restacking otherwise leaves an excluded overlay at
    // an unspecified level which could cover the cards.
    let card_windows = ordered_windows.iter().copied().collect::<HashSet<_>>();
    let mut z_order = monitor
        .clients
        .iter()
        .copied()
        .filter(|win| clients.contains_key(win) && !card_windows.contains(win))
        .collect::<Vec<_>>();
    z_order.extend(ordered_windows);

    OverviewLayout { moves, z_order }
}

fn restore_window_geometry(
    ctx: &mut WmCtx<'_>,
    monitor_id: crate::types::MonitorId,
    geometry: &HashMap<WindowId, Rect>,
) {
    for (&win, &rect) in geometry {
        if ctx
            .core()
            .model()
            .client(win)
            .is_some_and(|client| client.monitor_id == monitor_id)
        {
            ctx.move_resize(win, rect, MoveResizeOptions::immediate());
        }
    }
}

fn overview_eligible(client: &Client, selected_tags: TagMask) -> bool {
    client.is_visible(selected_tags) && !client.is_edge_scratchpad()
}

fn initial_window_order(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    selected_tags: TagMask,
) -> Vec<WindowId> {
    let mut windows = monitor
        .clients
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, win)| {
            let client = clients.get(&win)?;
            overview_eligible(client, selected_tags).then_some((
                client
                    .tags
                    .without_scratchpad()
                    .first_tag()
                    .unwrap_or(usize::MAX),
                index,
                win,
            ))
        })
        .collect::<Vec<_>>();
    windows.sort_by_key(|&(tag, index, _)| (tag, index));
    windows.into_iter().map(|(_, _, win)| win).collect()
}

#[derive(Debug, Clone, Copy)]
enum CascadeAxis {
    Horizontal,
    Vertical,
}

const PREFERRED_EXPOSED_CARD_PIXELS: i32 = 64;

fn card_hand_rects(work_rect: Rect, sizes: &[Size]) -> Vec<Rect> {
    if sizes.is_empty() {
        return Vec::new();
    }
    let axis = if work_rect.w >= work_rect.h {
        CascadeAxis::Horizontal
    } else {
        CascadeAxis::Vertical
    };
    let extent = match axis {
        CascadeAxis::Horizontal => work_rect.w,
        CascadeAxis::Vertical => work_rect.h,
    }
    .max(1);
    let last_extent = match axis {
        CascadeAxis::Horizontal => sizes.last().expect("sizes is non-empty").w,
        CascadeAxis::Vertical => sizes.last().expect("sizes is non-empty").h,
    }
    .max(1);
    // Reserve a useful strip for every covered card before deciding how much
    // of the topmost card can remain on-screen. With extremely large hands the
    // strip gracefully approaches one pixel, the physical limit for a unique
    // pointer target.
    let covered_cards = sizes.len().saturating_sub(1).min(i32::MAX as usize) as i32;
    let desired_reserved = PREFERRED_EXPOSED_CARD_PIXELS.saturating_mul(covered_cards);
    let reserved = desired_reserved.min(extent.saturating_sub(1));
    let tail_budget = extent.saturating_sub(reserved).max(1);
    let tail = last_extent.min(tail_budget).max(1);
    let anchor_span = extent.saturating_sub(tail);
    let denominator = sizes.len().saturating_sub(1).max(1) as i64;

    sizes
        .iter()
        .enumerate()
        .map(|(index, &size)| {
            let offset = (i64::from(anchor_span) * index as i64 / denominator) as i32;
            match axis {
                CascadeAxis::Horizontal => Rect::new(
                    work_rect.x + offset,
                    centered_cross_origin(work_rect.y, work_rect.h, size.h),
                    size.w,
                    size.h,
                ),
                CascadeAxis::Vertical => Rect::new(
                    centered_cross_origin(work_rect.x, work_rect.w, size.w),
                    work_rect.y + offset,
                    size.w,
                    size.h,
                ),
            }
        })
        .collect()
}

fn centered_cross_origin(start: i32, available: i32, size: i32) -> i32 {
    start + available.saturating_sub(size).max(0) / 2
}

#[cfg(test)]
#[path = "overview/tests.rs"]
mod tests;
