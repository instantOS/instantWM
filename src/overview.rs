use crate::contexts::WmCtx;
use crate::floating::{restore_all_floating, save_all_floating};
use crate::geometry::MoveResizeOptions;
use crate::globals::Globals;
use crate::types::{Monitor, Rect, TagMask, WindowId};

pub const OVERVIEW_MODE_NAME: &str = "overview";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitMode {
    RestorePrevious,
    ToSelectedWindow,
}

#[derive(Debug, Clone)]
pub struct OverviewState {
    restore_tags: TagMask,
}

impl OverviewState {
    pub fn new(restore_tags: TagMask) -> Self {
        Self { restore_tags }
    }
}

pub fn is_active(globals: &Globals) -> bool {
    globals.selected_monitor().overview_state.is_some()
}

pub fn is_active_on_monitor(globals: &Globals, monitor: &Monitor) -> bool {
    monitor.overview_state.is_some() && globals.selected_monitor_id() == monitor.id()
}

fn set_selected_tags_with_history(mon: &mut Monitor, new_mask: TagMask) -> bool {
    if mon.selected_tags() == new_mask {
        return false;
    }

    let previous_current_tag = mon.current_tag_number();
    mon.sel_tags = !mon.sel_tags;
    mon.set_selected_tags(new_mask);
    if previous_current_tag != mon.current_tag_number()
        && let Some(previous_current_tag) = previous_current_tag
    {
        mon.prev_tag = Some(previous_current_tag);
    }
    true
}

pub fn handle_mode_transition(ctx: &mut WmCtx<'_>, previous_mode: &str, next_mode: &str) {
    let entering_overview = previous_mode != OVERVIEW_MODE_NAME && next_mode == OVERVIEW_MODE_NAME;
    let leaving_overview = previous_mode == OVERVIEW_MODE_NAME && next_mode != OVERVIEW_MODE_NAME;

    if entering_overview {
        enter(ctx);
    } else if leaving_overview {
        exit(ctx, ExitMode::RestorePrevious);
    }
}

/// Exit overview mode with a specific [`ExitMode`].
///
/// Bypasses `WmCtx::set_current_mode` to avoid threading `ExitMode` through
/// the general mode system. If `set_current_mode` ever gains side effects,
/// this path must be updated to match.
pub fn exit_overview(ctx: &mut WmCtx<'_>, mode: ExitMode) {
    ctx.core_mut().globals_mut().behavior.current_mode = "default".to_string();
    exit(ctx, mode);
}

fn enter(ctx: &mut WmCtx<'_>) {
    let selected_monitor_id = ctx.core().globals().selected_monitor_id();
    let all_tags = TagMask::all(ctx.core().globals().tags.count());

    {
        let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
        if mon.overview_state.is_some() {
            return;
        }
        let restore_tags = mon.selected_tags();
        let _ = set_selected_tags_with_history(mon, all_tags);
        mon.overview_state = Some(OverviewState::new(restore_tags));
    }

    save_all_floating(ctx, Some(selected_monitor_id));
    crate::focus::focus(ctx, None);
    ctx.core_mut()
        .globals_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
}

fn exit(ctx: &mut WmCtx<'_>, mode: ExitMode) {
    let state = {
        let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
        mon.overview_state.take()
    };

    let Some(state) = state else { return };

    let selected_monitor_id = ctx.core().globals().selected_monitor_id();

    match mode {
        ExitMode::RestorePrevious => {
            let restore_mask = state.restore_tags;
            restore_all_floating(ctx, Some(selected_monitor_id));

            if !restore_mask.is_empty() {
                let _ = {
                    let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
                    set_selected_tags_with_history(mon, restore_mask)
                };
            }

            crate::focus::focus(ctx, None);
        }
        ExitMode::ToSelectedWindow => {
            let selected_window = ctx.core().globals().selected_win();
            let selected_tags = selected_window.and_then(|win| {
                ctx.core()
                    .globals()
                    .clients
                    .get(&win)
                    .map(|c| c.tags.without_scratchpad())
                    .filter(|tags| !tags.is_empty())
            });
            let restore_mask = state.restore_tags;

            restore_all_floating(ctx, Some(selected_monitor_id));

            let target_mask = selected_tags.or(Some(restore_mask));
            if let Some(mask) = target_mask
                && !mask.is_empty()
            {
                let _ = {
                    let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
                    set_selected_tags_with_history(mon, mask)
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
        .globals_mut()
        .queue_layout_for_monitor_urgent(selected_monitor_id);
}

pub fn toggle_overview(ctx: &mut WmCtx<'_>, _mask: TagMask) {
    if is_active(ctx.core().globals()) {
        exit_overview(ctx, ExitMode::ToSelectedWindow);
        ctx.request_bar_update();
        return;
    }

    if ctx.core().globals().selected_monitor().clients.is_empty() {
        return;
    }

    ctx.set_current_mode(OVERVIEW_MODE_NAME.to_string());
    ctx.request_bar_update();
}

pub fn cancel_overview(ctx: &mut WmCtx<'_>, _mask: TagMask) {
    if !is_active(ctx.core().globals()) {
        return;
    }

    ctx.reset_mode();
    ctx.request_bar_update();
}

/// Arrange the selected monitor in overview mode.
///
/// Uses a grid of anchor points while preserving each window's current size.
/// This produces a "cards"-style overview where windows can overlap but remain
/// partially visible.
pub fn arrange(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let selected_tags = m.selected_tags();
    let mut ordered_windows: Vec<WindowId> = m.z_order.iter_bottom_to_top().collect();
    for &win in &m.clients {
        if !ordered_windows.contains(&win) {
            ordered_windows.push(win);
        }
    }

    let clients: Vec<(WindowId, i32, i32, bool)> = ordered_windows
        .into_iter()
        .filter_map(|win| {
            let c = ctx.core().globals().clients.get(&win)?;
            if !c.is_visible(selected_tags) || c.is_edge_scratchpad() {
                return None;
            }
            Some((win, c.geo.w.max(1), c.geo.h.max(1), c.mode.is_floating()))
        })
        .collect();

    if clients.is_empty() {
        return;
    }

    let mut gridwidth = 1_i32;
    while (gridwidth * gridwidth) < clients.len() as i32 {
        gridwidth += 1;
    }

    let work_rect = m.work_rect;
    let cell_w = (work_rect.w / gridwidth).max(1);
    let cell_h = (work_rect.h / gridwidth).max(1);

    for (i, (win, width, height, is_floating)) in clients.iter().copied().enumerate() {
        if is_floating && let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            client.save_floating_geometry();
        }

        let row = i as i32 / gridwidth;
        let col = i as i32 % gridwidth;
        let x = work_rect.x + col * cell_w;
        let y = work_rect.y + row * cell_h;

        ctx.move_resize(
            win,
            Rect {
                x,
                y,
                w: width,
                h: height,
            },
            MoveResizeOptions::hinted_immediate(false),
        );
        ctx.raise_window_visual_only(win);
    }

    if let Some(selected) = m.sel
        && clients.iter().any(|(win, _, _, _)| *win == selected)
    {
        ctx.raise_window_visual_only(selected);
    }

    ctx.flush();
}
