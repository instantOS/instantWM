//! Client visibility: mapping/unmapping windows and WM_STATE transitions.

use crate::backend::WindowOps;
use crate::contexts::{WmCtx, WmCtxWayland};
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
        let selected_tags = mon.selected_tags();
        for (win, client) in mon.iter_clients(&model.clients) {
            plan.push(VisibilityEntry {
                win,
                rect: client.geo,
                border_width: client.border_width,
                mode: client.mode,
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
    match ctx {
        crate::contexts::WmCtx::X11(ctx_x11) => {
            crate::backend::x11::visibility::apply_visibility(ctx_x11);
        }
        crate::contexts::WmCtx::Wayland(ctx_wayland) => {
            apply_visibility_wayland(ctx_wayland);
        }
    }
}

pub fn apply_visibility_wayland(ctx: &mut WmCtxWayland<'_>) {
    let globals = ctx.core.state();
    for entry in visibility_plan(&globals.model) {
        if entry.visible {
            ctx.wayland.map_window(entry.win);
        } else {
            ctx.wayland.unmap_window(entry.win);
        }
    }
}

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

    if let WmCtx::X11(ctx_x11) = ctx {
        crate::backend::x11::visibility::show(ctx_x11, win);
    }

    crate::focus::focus(ctx, Some(win));
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

pub fn hide_for_user(ctx: &mut WmCtx, win: WindowId) {
    let scratchpad_name = ctx.core().model().client(win).and_then(|c| {
        if c.is_scratchpad() {
            Some(
                c.scratchpad
                    .as_ref()
                    .expect("is_scratchpad() implies scratchpad data is present")
                    .name
                    .clone(),
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
    let monitor_id = if let Some(c) = ctx.core_mut().model_mut().client_mut(win) {
        if c.is_hidden {
            return;
        }
        let mid = c.monitor_id;

        match ctx {
            WmCtx::X11(ctx_x11) => {
                crate::backend::x11::visibility::hide(ctx_x11, win);
            }
            WmCtx::Wayland(ctx_wl) => {
                hide_wayland(ctx_wl, win);
            }
        }

        if let Some(c_mut) = ctx.core_mut().model_mut().client_mut(win) {
            c_mut.is_hidden = true;
        }

        mid
    } else {
        return;
    };

    let snext = ctx
        .core()
        .state()
        .monitor(monitor_id)
        .and_then(|m| m.z_order.iter_top_to_bottom().find(|&w| w != win));
    crate::focus::focus(ctx, snext);
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

fn hide_wayland(ctx: &mut WmCtxWayland<'_>, win: WindowId) {
    ctx.wayland.unmap_window(win);
    ctx.wayland.flush();
}

#[cfg(test)]
mod tests {
    use super::visibility_plan;
    use crate::model::WmModel;
    use crate::types::*;

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
            mode: ClientMode::Tiling,
            geo: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
            ..Client::default()
        }
    }

    /// Build a single monitor with given selected tags and client list.
    fn make_monitor(id: usize, selected: TagMask, client_wins: Vec<WindowId>) -> Monitor {
        let mut mon = Monitor::default();
        mon.monitor_id = MonitorId::from_raw(id as u64);
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
        client.mode = ClientMode::Floating;

        let clients = vec![client];
        let mon = make_monitor(0, tag, vec![win]);
        let model = make_model(vec![mon], clients);

        let plan = visibility_plan(&model);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].rect, rect);
        assert_eq!(plan[0].border_width, 2);
        assert_eq!(plan[0].mode, ClientMode::Floating);
        assert!(plan[0].visible);
    }
}
