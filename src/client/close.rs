//! Client termination: the single guarded close funnel.
//!
//! Close is always a request, never a guarantee: the backend asks the client
//! to terminate (`WM_DELETE_WINDOW` on X11, `xdg_toplevel.close` on Wayland)
//! and only the X11 backend escalates to `XKillClient`, and only for clients
//! that do not advertise close-protocol support at all. There is no
//! user-selectable forceful close, and Wayland has no forceful path.
//!
//! Every close caller -- keybindings, bar buttons, overview cards, IPC and
//! foreign-toplevel requests -- crosses [`close_client`], so the
//! managed-and-unlocked policy is enforced in exactly one place.

use crate::contexts::WmCtx;
use crate::types::WindowId;

/// Ask a managed, unlocked client to close.
///
/// No-op when `win` is not a managed client or is locked.
pub fn close_client(ctx: &mut WmCtx, win: WindowId) {
    if !ctx.core().state.model.client_is_closable(win) {
        return;
    }

    ctx.close_window(win);
}
