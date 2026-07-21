//! Client termination: graceful close and forceful kill.

use crate::contexts::WmCtx;
use crate::types::WindowId;

pub fn kill_client(ctx: &mut WmCtx, win: WindowId) {
    let Some(client) = ctx.core().model().client(win) else {
        return;
    };

    if client.is_locked {
        return;
    }

    ctx.close_window(win);
}

pub fn shut_kill(ctx: &mut WmCtx) {
    let has_clients = !ctx
        .core()
        .model()
        .expect_selected_monitor()
        .clients
        .is_empty();

    if has_clients {
        if let Some(win) = ctx.core().model().selected_win() {
            kill_client(ctx, win);
        }
    } else {
        crate::util::spawn(ctx, &["instantshutdown"]);
    }
}

pub fn close_win(ctx: &mut WmCtx, win: WindowId) {
    let is_locked = ctx
        .core()
        .model()
        .client(win)
        .is_none_or(|client| client.is_locked);

    if is_locked {
        return;
    }

    ctx.close_window(win);
}
