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

/// Check whether `rect` lies on a different monitor than the one currently
/// owning `c_win` and, if so, migrate the window and update `selmon`.
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
    let core_state = &ctx.core().state;
    let Some(target) = core_state.model.monitors.monitor_by_rect(*rect) else {
        return;
    };

    let Some(current_mon) = core_state.model.monitor_of_client(c_win) else {
        return;
    };

    if target.id() == current_mon {
        return;
    }

    let _ = transfer_client(ctx, c_win, target.id(), TransferFocus::FollowWindow);
}

/// Convenience wrapper that reads the client's current geometry and delegates
/// to [`handle_monitor_switch`].
///
/// Use this after a geometry-driven resize. Move drops resolve their destination
/// from the pointer explicitly, including tiled moves that keep source geometry.
///
/// # Parameters
///
/// * `ctx` - The mouse context containing client and monitor state
/// * `c_win` - The client window to check and potentially move
pub fn handle_client_monitor_switch(ctx: &mut WmCtx, c_win: WindowId) {
    let Some(c) = ctx.core().state.model.client(c_win) else {
        return;
    };
    let rect = c.geo;

    handle_monitor_switch(ctx, c_win, &rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestWm as Wm;

    use crate::test_support::{MonitorBuilder, add_client_with};
    use crate::types::TagMask;

    #[test]
    fn drop_uses_the_clients_assignment_not_the_selected_monitor() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let tags = TagMask::single(1).unwrap();
        let source = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(0, 0, 1000, 800))
                .tag_count(4)
                .build(),
        );
        let target = wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .monitor_rect(Rect::new(1000, 0, 1000, 800))
                .tag_count(4)
                .build(),
        );
        wm.core
            .state
            .model
            .monitor_mut(source)
            .unwrap()
            .set_selected_tags(tags);
        wm.core
            .state
            .model
            .monitor_mut(target)
            .unwrap()
            .set_selected_tags(tags);
        wm.core.state.model.set_selected_monitor(target);

        let win = WindowId(41);
        add_client_with(&mut wm.core.state.model, source, |client| {
            client.win = win;
            client.tags = tags;
            client.geo = Rect::new(100, 100, 400, 300);
        });

        handle_monitor_switch(&mut wm.test_ctx(), win, &Rect::new(1200, 100, 400, 300));

        assert_eq!(wm.core.state.model.monitor_of_client(win), Some(target));
        assert!(!wm.core.state.model.monitor(source).unwrap().has_client(win));
        assert!(wm.core.state.model.monitor(target).unwrap().has_client(win));
    }
}
