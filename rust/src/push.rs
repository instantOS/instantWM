use crate::client::list::{attach, detach};
use crate::client::next_tiled;
use crate::contexts::WmCtx;
// focus() is used via focus_soft() in this module
use crate::layouts::arrange;
pub use crate::layouts::query::client_count;
use crate::types::WindowId;

pub fn next_c(ctx: &WmCtx, c_win: Option<WindowId>, include_floating: bool) -> Option<WindowId> {
    if !include_floating {
        return next_tiled(ctx, c_win);
    }

    let selected = ctx
        .g
        .selected_monitor()
        .map(|m| m.selected_tags())
        .unwrap_or(0);

    let mon = ctx.g.selected_monitor()?;
    if let Some(win) = c_win {
        let mut found = false;
        for &client_win in &mon.clients {
            if found {
                if let Some(c) = ctx.g.clients.get(&client_win) {
                    if c.is_visible_on_tags(selected) {
                        return Some(client_win);
                    }
                }
            }
            if client_win == win {
                found = true;
            }
        }
    }
    None
}

pub fn prev_c(ctx: &WmCtx, c_win: WindowId, include_floating: bool) -> Option<WindowId> {
    if ctx.g.monitors.is_empty() {
        return None;
    }

    let mon = ctx.g.selected_monitor()?;
    let selected = mon.selected_tags();

    let mut r: Option<WindowId> = None;

    for &win in &mon.clients {
        if win == c_win {
            break;
        }

        if let Some(c) = ctx.g.clients.get(&win) {
            if (include_floating || !c.isfloating) && c.is_visible_on_tags(selected) {
                r = Some(win);
            }
        }
    }

    r
}

pub fn push_up(ctx: &mut WmCtx, win: WindowId) {
    if client_count(ctx.g) < 2 {
        return;
    }

    let is_floating = ctx
        .g
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let selmon_id = ctx.g.selected_monitor_id();

    if let Some(mon) = ctx.g.monitors.get_mut(selmon_id) {
        if let Some(pos) = mon.clients.iter().position(|&w| w == win) {
            if pos > 0 {
                mon.clients.swap(pos, pos - 1);
            } else {
                let last = mon.clients.pop();
                if let Some(last_win) = last {
                    mon.clients.insert(1, last_win);
                }
            }
        }
    }

    crate::focus::focus_soft(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}

pub fn push_down(ctx: &mut WmCtx, win: WindowId) {
    if client_count(ctx.g) < 2 {
        return;
    }

    let is_floating = ctx
        .g
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if is_floating {
        return;
    }

    let selmon_id = ctx.g.selected_monitor_id();

    if let Some(mon) = ctx.g.monitors.get_mut(selmon_id) {
        if let Some(pos) = mon.clients.iter().position(|&w| w == win) {
            if pos + 1 < mon.clients.len() {
                mon.clients.swap(pos, pos + 1);
            } else {
                let first = mon.clients.remove(0);
                mon.clients.push(first);
            }
        }
    }

    crate::focus::focus_soft(ctx, Some(win));
    arrange(ctx, Some(selmon_id));
}
