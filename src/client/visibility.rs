//! Client visibility: mapping/unmapping windows and WM_STATE transitions.
//!
//! The policy (which clients are visible where, and with what geometry) is
//! computed here as a backend-neutral plan. Execution is projected through
//! [`crate::contexts::WmCtx`]: X11 parks windows mapped-but-offscreen, the
//! Wayland compositor maps/unmaps surfaces.

use crate::contexts::WmCtx;
use crate::model::WmModel;
use crate::types::{ClientMode, Rect, WindowId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct VisibilityEntry {
    pub win: WindowId,
    pub rect: Rect,
    pub border_width: i32,
    pub mode: ClientMode,
    pub visible: bool,
}

/// Snapshot visibility policy without performing backend I/O.
pub(crate) fn visibility_plan(model: &WmModel) -> Vec<VisibilityEntry> {
    let mut plan = Vec::new();
    for mon in model.monitors_iter_all() {
        let selected_tags = mon.visible_tags();
        for (win, client) in mon.iter_clients() {
            plan.push(VisibilityEntry {
                win,
                rect: client.geo,
                border_width: client.border_width,
                mode: client.mode(),
                visible: client.is_visible(selected_tags),
            });
        }
    }
    plan
}

// ---------------------------------------------------------------------------
// Recursive show/hide pass
// ---------------------------------------------------------------------------

/// Make a managed client visible without changing keyboard focus.
///
/// Focus is a separate policy decision. Callers that represent explicit user
/// activation must request it through `crate::focus` after revealing the
/// client.
pub fn show_window(ctx: &mut WmCtx, win: WindowId) {
    // The owning monitor is resolved before the mutable borrow: a client cannot
    // name a monitor other than the one holding it.
    let Some(monitor_id) = ctx.core().model().monitor_of_client(win) else {
        return;
    };
    let Some(client) = ctx.core_mut().model_mut().client_mut(win) else {
        return;
    };
    if !client.is_hidden {
        return;
    }
    client.is_hidden = false;

    ctx.reveal_client(win);

    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

pub fn hide_for_user(ctx: &mut WmCtx, win: WindowId) {
    let scratchpad_name = ctx.core().model().client(win).and_then(|c| {
        if c.is_scratchpad() {
            Some(
                c.scratchpad()
                    .expect("is_scratchpad() implies scratchpad data is present")
                    .name()
                    .to_string(),
            )
        } else {
            None
        }
    });

    if let Some(name) = scratchpad_name {
        crate::floating::scratchpad_hide_name(ctx, &name);
    } else {
        hide(ctx, win);
    }
}

pub fn hide(ctx: &mut WmCtx, win: WindowId) {
    hide_with_focus(ctx, win, None);
}

/// Hide a client and restore a preferred focus target when it is still valid.
///
/// Ordinary hides fall back to the top of the persistent stack. Temporary UI
/// such as a scratchpad can supply the window that was focused before it was
/// shown, preserving overlapping-layout presentation across the round trip.
pub(crate) fn hide_with_focus(ctx: &mut WmCtx, win: WindowId, preferred_focus: Option<WindowId>) {
    let was_selected = ctx
        .core()
        .model()
        .client_view(win)
        .is_some_and(|view| view.monitor.selected == Some(win));
    // Resolved before the mutable borrow: a client cannot name a monitor other
    // than the one holding it.
    let Some(monitor_id) = ctx.core().model().monitor_of_client(win) else {
        return;
    };
    let Some(client) = ctx.core_mut().model_mut().client_mut(win) else {
        return;
    };
    if client.is_hidden {
        return;
    }

    ctx.conceal_client(win);

    if let Some(c_mut) = ctx.core_mut().model_mut().client_mut(win) {
        c_mut.is_hidden = true;
    }

    if was_selected {
        let next = preferred_focus.or_else(|| {
            ctx.core()
                .model()
                .monitor(monitor_id)
                .and_then(|m| m.z_order().iter_top_to_bottom().find(|&w| w != win))
        });
        crate::focus::focus(ctx, next);
    }
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

#[cfg(test)]
mod tests {
    use super::{show_window, visibility_plan};
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::model::WmModel;
    use crate::types::*;
    use crate::wm::Wm;

    fn make_client(win: WindowId, tags: TagMask, hidden: bool, sticky: bool) -> Client {
        Client {
            win,
            tags,
            is_hidden: hidden,
            is_sticky: sticky,
            mode: ClientMode::tiled(),
            geo: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
            ..Client::default()
        }
    }

    #[test]
    fn showing_a_window_does_not_change_focus() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        wm.core.model.monitors.set_selected(monitor_id);
        let focused = WindowId(1);
        let hidden = WindowId(2);
        for (win, is_hidden) in [(focused, false), (hidden, true)] {
            assert!(wm.core.model.add_client(
                monitor_id,
                Client {
                    win,
                    is_hidden,
                    ..Client::default()
                }
            ));
        }
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected(Some(focused));

        show_window(&mut wm.ctx(), hidden);

        assert!(!wm.core.model.client(hidden).unwrap().is_hidden);
        assert_eq!(wm.core.model.selected_win(), Some(focused));
    }

    /// Build a single monitor showing `selected` tags.
    fn make_monitor(selected: TagMask) -> Monitor {
        let mut mon = Monitor::default();
        mon.set_selected_tags(selected);
        mon
    }

    /// Build a model whose monitors are configured with `selected_tags`, and
    /// whose clients are owned by the monitor at the same index.
    fn make_model(selected_tags: &[TagMask], clients: Vec<(usize, Client)>) -> WmModel {
        let mut model = WmModel::new();
        let monitor_ids: Vec<MonitorId> = selected_tags
            .iter()
            .map(|selected| model.monitors.push(make_monitor(*selected)))
            .collect();
        // `add_client` adopts newest-first into the focus stack, so adding in
        // reverse keeps the clients' written order as the monitor's focus order.
        for (monitor_index, client) in clients.into_iter().rev() {
            let win = client.win;
            assert!(
                model.add_client(monitor_ids[monitor_index], client),
                "test fixture must add {win:?} to a known, unoccupied monitor slot"
            );
        }
        model
    }

    #[test]
    fn visibility_returns_clients_on_active_tag() {
        let win1 = WindowId(1);
        let win2 = WindowId(2);
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();

        let clients = vec![
            (0, make_client(win1, tag1, false, false)),
            (0, make_client(win2, tag2, false, false)),
        ];
        let model = make_model(&[tag1], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 2);

        // win1 is on tag1 (active) -> visible
        // win2 is on tag2 (inactive) but in the same monitor's client list -> not visible
        assert_eq!(plan[0].win, win1);
        assert!(plan[0].visible);
        assert_eq!(plan[1].win, win2);
        assert!(!plan[1].visible);
    }

    #[test]
    fn visibility_hidden_clients_are_not_visible() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();

        let clients = vec![(0, make_client(win, tag, true, false))];
        let model = make_model(&[tag], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].win, win);
        assert!(!plan[0].visible, "hidden client should not be visible");
    }

    #[test]
    fn visibility_sticky_clients_visible_on_any_tag() {
        let win = WindowId(1);
        let tag1 = TagMask::single(1).unwrap();
        let tag2 = TagMask::single(2).unwrap();

        let clients = vec![(0, make_client(win, tag1, false, true))];
        let model = make_model(&[tag2], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 1);
        assert!(
            plan[0].visible,
            "sticky client should be visible on any tag"
        );
    }

    #[test]
    fn visibility_multiple_monitors() {
        let win1 = WindowId(1);
        let win2 = WindowId(2);
        let tag = TagMask::single(1).unwrap();

        let clients = vec![
            (0, make_client(win1, tag, false, false)),
            (1, make_client(win2, tag, false, false)),
        ];
        let model = make_model(&[tag, tag], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 2);
        assert!(plan[0].visible);
        assert!(plan[1].visible);
    }

    #[test]
    fn visibility_preserves_geometry_and_mode() {
        let win = WindowId(1);
        let tag = TagMask::single(1).unwrap();
        let rect = Rect {
            x: 50,
            y: 50,
            w: 200,
            h: 300,
        };

        let mut client = make_client(win, tag, false, false);
        client.geo = rect;
        client.border_width = 2;
        client.set_placement(crate::types::ClientPlacement::Floating);

        let clients = vec![(0, client)];
        let model = make_model(&[tag], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].rect, rect);
        assert_eq!(plan[0].border_width, 2);
        assert_eq!(plan[0].mode, ClientMode::floating());
        assert!(plan[0].visible);
    }
}
