//! Monitor-switch helpers for interactive mouse operations.
//!
//! When the user drags or resizes a window across a monitor boundary these
//! functions detect the crossing and call [`transfer_client`] + [`focus`] so the
//! window is correctly adopted by the new monitor.
//!
//! # Typical call flow
//!
//! ```text
//! shared move/resize interaction ends
//!   └─► handle_client_monitor_switch(win)
//!             └─► reads client.geo
//!                   └─► handle_monitor_switch(win, &rect)
//!                             ├─► MonitorManager lookup → target monitor id
//!                             └─► transfer_client(FollowWindow)
//!                                   ├─► reassigns client
//!                                   └─► focuses it on the new monitor
//! ```

use crate::contexts::WmCtx;
use crate::monitor::{TransferFocus, transfer_client};
use crate::types::*;

/// Check whether `rect` lies on a different monitor than the currently
/// selected one and, if so, migrate the window and update `selmon`.
///
/// This is the low-level primitive.  Most call-sites should use
/// [`handle_client_monitor_switch`] which reads the rect from the client.
///
/// # Parameters
///
/// * `ctx` - The mouse context containing monitor state
/// * `c_win` - The client window to potentially move
/// * `rect` - The window's geometry to check against monitor boundaries
pub fn handle_monitor_switch(ctx: &mut WmCtx, c_win: WindowId, rect: &Rect) {
    let new_mon = ctx.core().model().monitors.find_id_by_rect(rect);

    let Some(current_mon) = ctx
        .core()
        .model()
        .client(c_win)
        .map(|client| client.monitor_id)
    else {
        return;
    };

    let Some(target) = new_mon else { return };
    if target == current_mon {
        return;
    }

    let _ = transfer_client(ctx, c_win, target, TransferFocus::FollowWindow);
}

/// Convenience wrapper that reads the client's current geometry and delegates
/// to [`handle_monitor_switch`].
///
/// Call this at the end of every drag/resize loop so that windows dragged
/// across monitor boundaries are adopted by the correct monitor.
///
/// # Parameters
///
/// * `ctx` - The mouse context containing client and monitor state
/// * `c_win` - The client window to check and potentially move
pub fn handle_client_monitor_switch(ctx: &mut WmCtx, c_win: WindowId) {
    let Some(c) = ctx.core().model().client(c_win) else {
        return;
    };
    let rect = c.geo;

    handle_monitor_switch(ctx, c_win, &rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::types::{Client, Monitor, TagMask};
    use crate::wm::Wm;

    #[test]
    fn drop_uses_the_clients_assignment_not_the_selected_monitor() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let tags = TagMask::single(1).unwrap();
        let source = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 1000, 800),
            available_rect: Rect::new(0, 0, 1000, 800),
            ..Monitor::default()
        });
        let target = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(1000, 0, 1000, 800),
            available_rect: Rect::new(1000, 0, 1000, 800),
            ..Monitor::default()
        });
        wm.core
            .model
            .monitor_mut(source)
            .unwrap()
            .set_selected_tags(tags);
        wm.core
            .model
            .monitor_mut(target)
            .unwrap()
            .set_selected_tags(tags);
        wm.core.model.set_selected_monitor(target);

        let win = WindowId(41);
        wm.core.model.insert_client(Client {
            win,
            monitor_id: source,
            tags,
            geo: Rect::new(100, 100, 400, 300),
            ..Client::default()
        });
        wm.core.model.monitor_mut(source).unwrap().clients.push(win);

        handle_monitor_switch(&mut wm.ctx(), win, &Rect::new(1200, 100, 400, 300));

        assert_eq!(wm.core.model.client(win).unwrap().monitor_id, target);
        assert!(
            !wm.core
                .model
                .monitor(source)
                .unwrap()
                .clients
                .contains(&win)
        );
        assert!(
            wm.core
                .model
                .monitor(target)
                .unwrap()
                .clients
                .contains(&win)
        );
    }
}
