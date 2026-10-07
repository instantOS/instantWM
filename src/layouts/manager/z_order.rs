use crate::backend::WindowOps;
use crate::contexts::WmCtx;
use crate::types::{Monitor, WindowId};
use std::collections::HashSet;

/// Apply one compositor-wide projection. Monitor ownership controls layout and
/// visibility, never the layer of a window on a neighbouring output.
pub fn sync_z_order(ctx: &mut WmCtx<'_>) {
    let stack = global_z_order(&ctx.core().state.model);
    ctx.apply_z_order(&stack);
    ctx.flush();
}

#[derive(Default)]
struct StackLayers {
    tiled: Vec<WindowId>,
    bars: Vec<WindowId>,
    floating: Vec<WindowId>,
    fullscreen: Vec<WindowId>,
    transients: Vec<WindowId>,
    overview: Vec<WindowId>,
}

impl StackLayers {
    fn flatten(self) -> Vec<WindowId> {
        self.tiled
            .into_iter()
            .chain(self.bars)
            .chain(self.floating)
            .chain(self.fullscreen)
            .chain(self.transients)
            .chain(self.overview)
            .collect()
    }
}

pub(crate) fn global_z_order(model: &crate::model::WmModel) -> Vec<WindowId> {
    let mut layers = StackLayers::default();
    for monitor in model.monitors.iter_all() {
        if model.is_overview_active_on(monitor) {
            layers.overview.extend(crate::overview::z_order(monitor));
            layers.bars.extend(
                [monitor.bar_win, monitor.bottom_bar_win]
                    .into_iter()
                    .filter(|win| *win != WindowId::default()),
            );
            continue;
        }
        let local = monitor_stack_layers(monitor);
        layers.tiled.extend(local.tiled);
        layers.bars.extend(local.bars);
        layers.floating.extend(local.floating);
        layers.fullscreen.extend(local.fullscreen);
        layers.transients.extend(local.transients);
    }
    // Transient relationships can cross monitor ownership boundaries.
    layers.transients.sort_by_key(|win| {
        transient_depth(*win, |id| {
            model.client(id).and_then(|client| client.transient_for)
        })
    });
    layers.flatten()
}

/// Number of managed transient ancestors for `win`.
///
/// Unknown parents still count as one relationship so a dialog does not lose
/// its protected layer during parent teardown. Cycles are malformed protocol
/// input; stopping at the first repeated window keeps ordering deterministic.
fn transient_depth(win: WindowId, parent_of: impl Fn(WindowId) -> Option<WindowId>) -> usize {
    let mut depth = 0;
    let mut current = win;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let Some(parent) = parent_of(current) else {
            break;
        };
        depth += 1;
        current = parent;
    }
    depth
}

#[cfg(test)]
pub(super) fn compute_monitor_z_order(monitor: &Monitor) -> Vec<WindowId> {
    monitor_stack_layers(monitor).flatten()
}

fn monitor_stack_layers(monitor: &Monitor) -> StackLayers {
    let selected_window = monitor.selected;
    let selected_tags = monitor.visible_tags();
    let bar_win = monitor.bar_win;
    let bottom_bar_win = monitor.bottom_bar_win;
    let layout = monitor.current_layout();
    let tiled_focus = monitor.most_recent_focus(selected_tags, |win| {
        monitor
            .client(win)
            .is_some_and(|c| c.mode().is_normal_tiling() && c.is_visible(selected_tags))
    });

    let mut tiled_stack = Vec::new();
    let mut floating_stack = Vec::new();
    let mut fullscreen_stack = Vec::new();
    let mut transient_stack = Vec::new();
    for win in monitor.z_order().iter_bottom_to_top() {
        if let Some(c) = monitor.client(win)
            && c.is_visible(selected_tags)
        {
            let depth = transient_depth(win, |id| monitor.client(id).and_then(|c| c.transient_for));
            if depth > 0 {
                transient_stack.push((depth, win));
                continue;
            }
            let mode = c.mode();
            if mode.is_true_fullscreen() {
                fullscreen_stack.push(win);
            } else if mode.placement() == crate::types::ClientPlacement::Floating
                || mode.is_maximized()
            {
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
    let bars = [bar_win, bottom_bar_win]
        .into_iter()
        .filter(|win| *win != WindowId::default())
        .collect();
    StackLayers {
        tiled: tiled_stack,
        bars,
        floating: floating_stack,
        fullscreen: fullscreen_stack,
        transients: transient_stack.into_iter().map(|(_, win)| win).collect(),
        overview: Vec::new(),
    }
}
