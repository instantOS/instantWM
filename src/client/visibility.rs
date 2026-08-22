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
        for (win, client) in mon.iter_clients(&model.clients) {
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

/// Walk the client list, moving each client on- or off-screen.
///
/// Visible clients (those whose tag-set overlaps the monitor's selected tags)
/// are positioned at their stored geometry.  Invisible clients are moved
/// `2 * client_width` pixels to the left of the screen (i.e. off-screen left).
///
/// This mirrors the classic dwm `showhide` function and is called by the
/// arrange path after every layout change.
pub fn apply_visibility(ctx: &mut crate::contexts::WmCtx) {
    ctx.apply_visibility_plan();
}

/// Make a managed client visible without changing keyboard focus.
///
/// Focus is a separate policy decision. Callers that represent explicit user
/// activation must request it through `crate::focus` after revealing the
/// client.
pub fn show_window(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(c) = ctx.core_mut().model_mut().client_mut(win) {
        if !c.is_hidden {
            return;
        }
        c.is_hidden = false;
        c.monitor_id
    } else {
        return;
    };

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
        .client(win)
        .and_then(|client| ctx.core().model().monitor(client.monitor_id))
        .is_some_and(|monitor| monitor.selected == Some(win));
    let monitor_id = if let Some(c) = ctx.core_mut().model_mut().client_mut(win) {
        if c.is_hidden {
            return;
        }
        let mid = c.monitor_id;

        ctx.conceal_client(win);

        if let Some(c_mut) = ctx.core_mut().model_mut().client_mut(win) {
            c_mut.is_hidden = true;
        }

        mid
    } else {
        return;
    };

    if was_selected {
        let next = preferred_focus.or_else(|| {
            ctx.core()
                .model()
                .monitor(monitor_id)
                .and_then(|m| m.z_order.iter_top_to_bottom().find(|&w| w != win))
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

    fn make_client(
        win: WindowId,
        tags: TagMask,
        mon: MonitorId,
        hidden: bool,
        sticky: bool,
    ) -> Client {
        Client {
            win,
            tags,
            monitor_id: mon,
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
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                is_hidden,
                ..Client::default()
            });
            wm.core
                .model
                .monitor_mut(monitor_id)
                .unwrap()
                .clients
                .push(win);
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

    /// Build a single monitor with given selected tags and client list.
    fn make_monitor(id: usize, selected: TagMask, client_wins: Vec<WindowId>) -> Monitor {
        let mut mon = Monitor {
            monitor_id: MonitorId::from_raw(id as u64),
            ..Monitor::default()
        };
        mon.set_selected_tags(selected);
        mon.clients = client_wins;
        mon
    }

    fn make_model(monitors: Vec<Monitor>, clients: Vec<Client>) -> WmModel {
        let mut model = WmModel::new();
        for m in monitors {
            model.monitors.push(m);
        }
        for c in clients {
            model.insert_client(c);
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
            make_client(win1, tag1, MonitorId::from_raw(0), false, false),
            make_client(win2, tag2, MonitorId::from_raw(0), false, false),
        ];
        let mon = make_monitor(0, tag1, vec![win1, win2]);
        let model = make_model(vec![mon], clients);

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

        let clients = vec![make_client(win, tag, MonitorId::from_raw(0), true, false)];
        let mon = make_monitor(0, tag, vec![win]);
        let model = make_model(vec![mon], clients);

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

        let clients = vec![make_client(win, tag1, MonitorId::from_raw(0), false, true)];
        let mon = make_monitor(0, tag2, vec![win]);
        let model = make_model(vec![mon], clients);

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
            make_client(win1, tag, MonitorId::from_raw(0), false, false),
            make_client(win2, tag, MonitorId::from_raw(1), false, false),
        ];
        let mon0 = make_monitor(0, tag, vec![win1]);
        let mon1 = make_monitor(1, tag, vec![win2]);
        let model = make_model(vec![mon0, mon1], clients);

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

        let mut client = make_client(win, tag, MonitorId::from_raw(0), false, false);
        client.geo = rect;
        client.border_width = 2;
        client.set_placement(crate::types::ClientPlacement::Floating);

        let clients = vec![client];
        let mon = make_monitor(0, tag, vec![win]);
        let model = make_model(vec![mon], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].rect, rect);
        assert_eq!(plan[0].border_width, 2);
        assert_eq!(plan[0].mode, ClientMode::floating());
        assert!(plan[0].visible);
    }
}
