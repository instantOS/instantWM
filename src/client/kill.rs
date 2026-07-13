//! Client termination: graceful close and forceful kill.

use crate::contexts::WmCtx;
use crate::types::WindowId;

pub fn kill_client(ctx: &mut WmCtx, win: WindowId) {
    let Some(client) = ctx.core().model().clients.get(&win) else {
        return;
    };

    if client.is_locked {
        return;
    }

    force_close(ctx, win);
}

fn force_close(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(ctx_x11) => {
            let wmatom_delete = ctx_x11.x11_runtime.wmatom.delete;
            crate::backend::x11::kill::force_close(ctx_x11, win, wmatom_delete);
        }
        WmCtx::Wayland(wl) => {
            let _ = wl.wayland.close_window(win);
        }
    }
}

pub fn shut_kill(ctx: &mut WmCtx) {
    let has_clients = !ctx.core().model().selected_monitor().clients.is_empty();

    if has_clients {
        if let Some(win) = ctx.core().model().selected_win() {
            kill_client(ctx, win);
        }
    } else {
        crate::util::spawn(ctx, &["instantshutdown"]);
    }
}

pub fn close_win(ctx: &mut WmCtx, win: WindowId) {
    let is_locked = ctx.core().model().clients.is_locked(win);

    if is_locked {
        return;
    }

    force_close(ctx, win);
}
