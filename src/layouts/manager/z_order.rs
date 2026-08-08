use crate::contexts::WmCtx;
use crate::types::{Client, Monitor, MonitorId, WindowId};
use std::collections::{HashMap, HashSet};

pub fn sync_monitor_z_order(ctx: &mut WmCtx<'_>, monitor_id: MonitorId) {
    ctx.request_bar_geometry_update(monitor_id);

    let Some(monitor) = ctx.core().model().monitor(monitor_id) else {
        return;
    };

    if ctx.core().model().is_overview_active_on(monitor) {
        return;
    }

    let clients = &ctx.core().model().clients;
    let Some(stack) = compute_monitor_z_order(monitor, clients) else {
        return;
    };
    ctx.window_backend().apply_z_order(&stack);
    ctx.window_backend().flush();
}

/// Number of managed transient ancestors for `win`.
///
/// Unknown parents still count as one relationship so a dialog does not lose
/// its protected layer during parent teardown. Cycles are malformed protocol
/// input; stopping at the first repeated window keeps ordering deterministic.
fn transient_depth(win: WindowId, clients: &HashMap<WindowId, Client>) -> usize {
    let mut depth = 0;
    let mut current = win;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let Some(parent) = clients
            .get(&current)
            .and_then(|client| client.transient_for)
        else {
            break;
        };
        depth += 1;
        current = parent;
    }
    depth
}

pub(super) fn compute_monitor_z_order(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
) -> Option<Vec<WindowId>> {
    let selected_window = monitor.selected;
    let selected_tags = monitor.visible_tags();
    let bar_win = monitor.bar_win;
    let bottom_bar_win = monitor.bottom_bar_win;
    let layout = monitor.current_layout();
    let tiled_focus = monitor.most_recent_focus(selected_tags, |win| {
        clients
            .get(&win)
            .is_some_and(|c| c.mode().is_normal_tiling() && c.is_visible(selected_tags))
    });

    let mut tiled_stack = Vec::new();
    let mut floating_stack = Vec::new();
    let mut fullscreen_stack = Vec::new();
    let mut transient_stack = Vec::new();
    for win in monitor.z_order.iter_bottom_to_top() {
        if let Some(c) = clients.get(&win)
            && c.is_visible(selected_tags)
        {
            let depth = transient_depth(win, clients);
            if depth > 0 {
                transient_stack.push((depth, win));
                continue;
            }
            let mode = c.mode();
            if mode.is_true_fullscreen() {
                fullscreen_stack.push(win);
            } else if mode.is_fake_fullscreen() {
                // Fake fullscreen keeps its existing layout layer.
            } else if mode.is_normal_floating() || mode.is_maximized() {
                floating_stack.push(win);
            } else if layout.is_tiling() {
                tiled_stack.push(win);
            } else {
                floating_stack.push(win);
            }
        }
    }

    // Stable depth ordering keeps children above their transient ancestors,
    // while persistent z-order remains authoritative between siblings.
    transient_stack.sort_by_key(|(depth, _)| *depth);

    if let Some(tiled_focus) = tiled_focus
        && selected_window != Some(tiled_focus)
        && (selected_window.is_some_and(|win| floating_stack.contains(&win))
            || selected_window.is_some_and(|win| fullscreen_stack.contains(&win))
            || transient_stack
                .iter()
                .any(|(_, win)| Some(*win) == selected_window))
        && let Some(idx) = tiled_stack.iter().position(|&win| win == tiled_focus)
    {
        let selected = tiled_stack.remove(idx);
        tiled_stack.push(selected);
    }

    if let Some(idx) = selected_window
        .and_then(|selected| fullscreen_stack.iter().position(|&win| win == selected))
    {
        let selected = fullscreen_stack.remove(idx);
        fullscreen_stack.push(selected);
    } else if layout.is_maximized()
        && let Some(selected_window) = selected_window
    {
        // In maximized presentation, the focused tiled client must be
        // projected to the top of the tiled layer without mutating persistent
        // z-order.
        if let Some(idx) = tiled_stack.iter().position(|&win| win == selected_window) {
            let selected = tiled_stack.remove(idx);
            tiled_stack.push(selected);
        }
    }

    // Final z-order: tiled clients, bar, ordinary floating clients,
    // fullscreen clients, then transient dialogs. Focus never changes the
    // order within the floating layer. Keeping transients in the protected top
    // layer prevents a modal dialog from disappearing while its parent remains
    // blocked waiting for a response.
    let mut stack = tiled_stack;
    stack.push(bar_win);
    if bottom_bar_win != WindowId::default() {
        stack.push(bottom_bar_win);
    }
    stack.extend(floating_stack);
    stack.extend(fullscreen_stack);
    stack.extend(transient_stack.into_iter().map(|(_, win)| win));
    Some(stack)
}
