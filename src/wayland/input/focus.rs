//! Device-neutral focus operations shared by Wayland input handlers.

use crate::types::{MouseButton, WindowId};
use crate::wm::Wm;

/// Focus the managed target selected by an explicit pointer or touch action.
pub(crate) fn focus_managed_target(
    wm: &mut Wm,
    target: Option<WindowId>,
    button: Option<MouseButton>,
) {
    let mut ctx = wm.ctx();
    let Some(window) = target else {
        crate::focus::focus(&mut ctx, None);
        return;
    };

    crate::focus::select_monitor_for_client(&mut ctx, window);
    crate::focus::focus(&mut ctx, Some(window));
    if let Some(button) = button {
        crate::focus::raise_floating_on_client_click(&mut ctx, window, button);
    }
}
